//! meter-heartbeat — a liveness tap. Consumes the (isolated, experiment) Kinesis stream
//! fed by an IoT `SELECT *, topic() FROM '#'` rule, extracts one heartbeat per message
//! (transport envelope only, NO decode), and writes each Lambda batch as a JSONL object
//! to the unified S3 bucket, partitioned by ingest date. Athena reads it for
//! "did device/gateway X report recently?". Prototype: JSONL now, Parquet is the harden step.

use aws_lambda_events::event::kinesis::KinesisEvent;
use chrono::Utc;
use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use serde_json::Value;

mod heartbeat;
mod parquet_out;

struct Ctx {
    s3: aws_sdk_s3::Client,
    bucket: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let cfg = aws_config::load_from_env().await;
    let ctx = Ctx {
        s3: aws_sdk_s3::Client::new(&cfg),
        bucket: std::env::var("HEARTBEAT_BUCKET").expect("HEARTBEAT_BUCKET"),
    };
    let ctx = &ctx;
    run(service_fn(move |e| handler(e, ctx))).await
}

async fn handler(event: LambdaEvent<KinesisEvent>, ctx: &Ctx) -> Result<(), Error> {
    let records = event.payload.records;
    let ingest = Utc::now();
    let ingest_time = ingest.to_rfc3339();

    // One heartbeat per record (skip records that aren't parseable JSON — a poison
    // message must not fail the whole batch and stall the shard).
    let rows: Vec<heartbeat::Heartbeat> = records
        .iter()
        .filter_map(|r| serde_json::from_slice::<Value>(&r.kinesis.data.0).ok())
        .map(|msg| {
            let topic = msg.get("topic").and_then(Value::as_str).unwrap_or("").to_owned();
            heartbeat::from_message(&topic, &msg, &ingest_time)
        })
        .collect();

    if rows.is_empty() {
        return Ok(());
    }
    let n = rows.len();
    let bytes = parquet_out::encode(&rows).map_err(|e| e.to_string())?;

    // Unique key per batch: ingest instant + the batch's first sequence number.
    let seq = records
        .first()
        .and_then(|r| r.kinesis.sequence_number.clone())
        .unwrap_or_else(|| ingest.timestamp_nanos_opt().unwrap_or(0).to_string());
    let key = format!(
        "heartbeat/dt={}/{}-{}.parquet",
        ingest.format("%Y-%m-%d"),
        ingest.timestamp_millis(),
        seq
    );

    ctx.s3
        .put_object()
        .bucket(&ctx.bucket)
        .key(&key)
        .body(bytes.into())
        .content_type("application/vnd.apache.parquet")
        .send()
        .await?;

    println!("wrote {n} heartbeats (parquet) -> s3://{}/{}", ctx.bucket, key);
    Ok(())
}
