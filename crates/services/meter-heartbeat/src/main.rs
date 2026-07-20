//! meter-heartbeat — a device-liveness tap. Consumes the Flink input Kinesis stream
//! (read-only), extracts one heartbeat per message (transport envelope only, NO decode),
//! and upserts the **latest state per device** into the `meter-liveness` DynamoDB table.
//!
//! Key: `pk = customerid` (stamped onto the stream by the IoT rules), `sk = meterid`
//! (the device). Gateway is NOT a key — a LoRaWAN uplink is heard by multiple gateways,
//! so it isn't unique; it's stored as an informational column. Latest-state means one
//! item per device (bounded by device count, not message rate) and O(1) reads — no files,
//! no compaction, no scan.

use std::collections::HashMap;

use aws_lambda_events::event::kinesis::KinesisEvent;
use aws_sdk_dynamodb::types::AttributeValue;
use chrono::Utc;
use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use serde_json::Value;

mod heartbeat;

struct Ctx {
    ddb: aws_sdk_dynamodb::Client,
    table: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let cfg = aws_config::load_from_env().await;
    let ctx = Ctx {
        ddb: aws_sdk_dynamodb::Client::new(&cfg),
        table: std::env::var("LIVENESS_TABLE").expect("LIVENESS_TABLE"),
    };
    let ctx = &ctx;
    run(service_fn(move |e| handler(e, ctx))).await
}

async fn handler(event: LambdaEvent<KinesisEvent>, ctx: &Ctx) -> Result<(), Error> {
    let records = event.payload.records;
    // One ingest instant for the whole batch — this is the "last_seen" (when WE received
    // it), and it's monotonic across invocations, which the conditional write relies on.
    let ingest_time = Utc::now().to_rfc3339();

    // Extract one heartbeat per record, then keep only the latest per (customerid, meterid)
    // in this batch — we store latest-state, so intra-batch duplicates collapse to one
    // write. A record that isn't JSON, or lacks the customerid/meterid key, is skipped
    // (a poison message must not fail the batch and stall the shard).
    let mut latest: HashMap<(String, String), heartbeat::Heartbeat> = HashMap::new();
    for r in &records {
        let Ok(msg) = serde_json::from_slice::<Value>(&r.kinesis.data.0) else { continue };
        let topic = msg.get("topic").and_then(Value::as_str).unwrap_or("").to_owned();
        let hb = heartbeat::from_message(&topic, &msg, &ingest_time);
        let (Some(cust), Some(meter)) = (hb.customerid.clone(), hb.device_id.clone()) else {
            continue;
        };
        // Keep the message with the greater event_time (per device the format is stable,
        // so string compare is chronological). No entry yet → insert.
        match latest.get(&(cust.clone(), meter.clone())) {
            Some(cur) if cur.event_time >= hb.event_time => {}
            _ => {
                latest.insert((cust, meter), hb);
            }
        }
    }

    let mut written = 0usize;
    for hb in latest.values() {
        match upsert(ctx, hb).await {
            Ok(true) => written += 1,
            Ok(false) => {}        // stale (older than stored last_seen) — expected, skip
            Err(e) => eprintln!("liveness upsert failed: {e}"), // best-effort: don't stall
        }
    }
    println!("liveness: {written} devices upserted from {} records", records.len());
    Ok(())
}

/// Conditional latest-state upsert for one device. Writes the whole current snapshot,
/// but only if this message is newer than what's stored (`attribute_not_exists(last_seen)
/// OR :ls > last_seen`) — a cheap guard against an out-of-order shard overwriting a newer
/// reading. Returns `Ok(false)` when the conditional check rejected the write (stale).
async fn upsert(ctx: &Ctx, hb: &heartbeat::Heartbeat) -> Result<bool, Error> {
    let cust = hb.customerid.as_deref().unwrap_or_default();
    let meter = hb.device_id.as_deref().unwrap_or_default();

    let mut item: HashMap<String, AttributeValue> = HashMap::new();
    item.insert("pk".into(), AttributeValue::S(cust.to_string()));
    item.insert("sk".into(), AttributeValue::S(meter.to_string()));
    item.insert("last_seen".into(), AttributeValue::S(hb.ingest_time.clone()));
    item.insert("transport".into(), AttributeValue::S(hb.transport.to_string()));
    let mut put_s = |k: &str, v: &Option<String>| {
        if let Some(x) = v {
            item.insert(k.into(), AttributeValue::S(x.clone()));
        }
    };
    put_s("gatewayid", &hb.gateway_id);
    put_s("schematype", &hb.schematype);
    put_s("event_time", &hb.event_time);

    let res = ctx
        .ddb
        .put_item()
        .table_name(&ctx.table)
        .set_item(Some(item))
        .condition_expression("attribute_not_exists(last_seen) OR :ls > last_seen")
        .expression_attribute_values(":ls", AttributeValue::S(hb.ingest_time.clone()))
        .send()
        .await;

    match res {
        Ok(_) => Ok(true),
        Err(e) if e
            .as_service_error()
            .is_some_and(|se| se.is_conditional_check_failed_exception()) => Ok(false),
        Err(e) => Err(e.into()),
    }
}
