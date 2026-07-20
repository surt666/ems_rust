//! meter-heartbeat — a device-liveness tap. Consumes the Flink input Kinesis stream
//! (read-only), extracts one heartbeat per message (transport envelope only, NO decode),
//! and PUTs the **latest state per device** to Amazon Data Firehose, which upserts it into
//! the `all.heartbeat` Iceberg table (S3 Tables) on `(customerid, meterid)`.
//!
//! Key: `customerid` (stamped onto the stream by the IoT rules), `meterid` (the device).
//! Gateway is NOT a key — a LoRaWAN uplink is heard by multiple gateways; it's an info
//! column. Firehose buffers ~5 min into large files + S3 Tables auto-compacts, so this is
//! ~100× cheaper than per-message DynamoDB writes (Iceberg destination waives the Firehose
//! 5 KB-per-record rounding), with no small-file read problem.

use std::collections::HashMap;

use aws_lambda_events::event::kinesis::KinesisEvent;
use aws_sdk_firehose::primitives::Blob;
use aws_sdk_firehose::types::Record;
use chrono::Utc;
use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use serde_json::{json, Value};

mod heartbeat;

/// Firehose PutRecordBatch caps at 500 records per call.
const BATCH_MAX: usize = 500;

struct Ctx {
    fh: aws_sdk_firehose::Client,
    stream: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let cfg = aws_config::load_from_env().await;
    let ctx = Ctx {
        fh: aws_sdk_firehose::Client::new(&cfg),
        stream: std::env::var("FIREHOSE_STREAM").expect("FIREHOSE_STREAM"),
    };
    let ctx = &ctx;
    run(service_fn(move |e| handler(e, ctx))).await
}

async fn handler(event: LambdaEvent<KinesisEvent>, ctx: &Ctx) -> Result<(), Error> {
    let records = event.payload.records;
    let ingest_time = Utc::now().to_rfc3339();

    // Extract one heartbeat per record, keep only the latest per (customerid, meterid) in
    // this batch (we store latest-state, so intra-batch duplicates collapse to one upsert).
    // A record that isn't JSON or lacks the customerid/meterid key is skipped (a poison
    // message must not fail the batch and stall the shard).
    let mut latest: HashMap<(String, String), heartbeat::Heartbeat> = HashMap::new();
    for r in &records {
        let Ok(msg) = serde_json::from_slice::<Value>(&r.kinesis.data.0) else { continue };
        let topic = msg.get("topic").and_then(Value::as_str).unwrap_or("").to_owned();
        let hb = heartbeat::from_message(&topic, &msg, &ingest_time);
        let (Some(cust), Some(meter)) = (hb.customerid.clone(), hb.device_id.clone()) else {
            continue;
        };
        match latest.get(&(cust.clone(), meter.clone())) {
            Some(cur) if cur.event_time >= hb.event_time => {}
            _ => {
                latest.insert((cust, meter), hb);
            }
        }
    }

    // One Firehose record per device (a single JSON object; Firehose upserts it into the
    // Iceberg table on customerid+meterid).
    let fh_records: Vec<Record> = latest
        .values()
        .filter_map(|hb| {
            let payload = json!({
                "customerid": hb.customerid,
                "meterid": hb.device_id,
                "last_seen": hb.ingest_time,
                "event_time": hb.event_time,
                "gatewayid": hb.gateway_id,
                "schematype": hb.schematype,
                "transport": hb.transport,
            });
            let bytes = serde_json::to_vec(&payload).ok()?;
            Record::builder().data(Blob::new(bytes)).build().ok()
        })
        .collect();

    if fh_records.is_empty() {
        return Ok(());
    }
    let total = fh_records.len();

    // PutRecordBatch in ≤500-record chunks. Best-effort per chunk: a transient partial
    // failure just means those devices' last_seen refreshes on their next message.
    for chunk in fh_records.chunks(BATCH_MAX) {
        let resp = ctx
            .fh
            .put_record_batch()
            .delivery_stream_name(&ctx.stream)
            .set_records(Some(chunk.to_vec()))
            .send()
            .await?;
        if resp.failed_put_count() > 0 {
            eprintln!("firehose: {} of {} records failed in chunk", resp.failed_put_count(), chunk.len());
        }
    }

    println!("liveness: {total} devices → firehose from {} records", records.len());
    Ok(())
}
