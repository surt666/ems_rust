# TODO

## Backfill Missed Measurements on New Meter Mapping

When a meter starts sending data before its mapping is added to `meter-identity` DDB, raw data lands in the `raw_data` Iceberg table but never gets enriched into `logical_meter_data`. There is currently no mechanism to recover these missed measurements.

### Problem

1. Meter starts sending → records land in `raw_data` immediately
2. No `MeterMapping` exists yet → enrichment drops/errors the records
3. Mapping added to DDB → only new records get enriched from this point
4. Gap between first data and mapping creation is lost

### Options

**Option A: Automatic backfill on mapping creation (recommended)**
- Trigger a Lambda from the DDB/Kinesis CDC stream when a new mapping is inserted
- Lambda queries `raw_data` Iceberg table for that daqId (all time or configurable window)
- Applies the mapping (enrichment + counter delta if applicable) and writes to `logical_meter_data`
- Zero manual intervention, no missed data

**Option B: Batch backfill job (simpler stopgap)**
- Manual or scheduled Glue job / Lambda / script
- Takes a daqId + time range, queries `raw_data`, enriches, and inserts into `logical_meter_data`
- Could be triggered on-demand from an admin API

**Option C: Kinesis replay (fragile)**
- Replay from Kinesis if within retention window (24h)
- Time-limited and doesn't cover older data

### Design Considerations

- Backfill must apply the same enrichment logic (unit mapping, counter delta) as the Flink pipeline to keep data consistent
- Counter delta backfill needs records in timestamp order — batch job can sort, unlike streaming
- Binning should also apply during backfill
- Need idempotency to avoid duplicate records if backfill overlaps with streaming data

## Switch Kinesis Consumers to Enhanced Fan-Out (EFO)

Both Flink apps (`FlinkKinesisConsumer`) use the shared-throughput `GetRecords` API on the `DAQ_INPUT_STREAM`. Deploying one app causes the other to fail with `GetRecords` retries exhausted because the 5 calls/sec and 2 MB/s per-shard limits are shared across all consumers.

### Options

**Option A: Enhanced Fan-Out (EFO)**

Each consumer gets a dedicated 2 MB/s pipe per shard via `SubscribeToShard`:

```scala
sourceProps.setProperty(ConsumerConfigConstants.RECORD_PUBLISHER_TYPE, "EFO")
sourceProps.setProperty(ConsumerConfigConstants.EFO_CONSUMER_NAME, "<unique-name>")
```

Each Flink app needs a distinct `EFO_CONSUMER_NAME`. Lambda ESMs also support EFO.

Downside: ~$0.015/shard-hour per consumer. With on-demand/serverless Kinesis the shard count is uncontrolled, so EFO costs can spike unpredictably.

**Option B: Increase `SHARD_GETRECORDS_INTERVAL_MILLIS` (cheap stopgap)**

Currently unset (default 200ms = 5 calls/sec per shard, the API limit). With 3 consumers (2 Flink + Lambda ESM) sharing the budget, this guarantees throttling. Increasing the interval on both Flink apps (e.g., 500–1000ms) reduces contention at the cost of higher latency.

**Option C: Consolidate into a single Flink app**

Eliminate the contention entirely by merging both pipelines into one app. No extra cost, but increases blast radius and coupling.
