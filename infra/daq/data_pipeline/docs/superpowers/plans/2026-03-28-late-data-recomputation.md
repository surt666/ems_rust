# Late Data Recomputation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Handle late-arriving data (beyond the Flink watermark window) and historical backfill by recomputing counter deltas from the `raw_cdk` Iceberg table using AWS Glue Spark, appending corrected records to `nested_meter_readings_cdk2` following event-sourcing semantics (append-only, no merge/update).

**Context:** The Flink pipeline uses a 1-hour `BoundedOutOfOrderness` watermark. Records arriving after the watermark are routed to a `LATE_ARRIVAL` Kinesis error stream. These records exist in `raw_cdk` (the raw sink has no watermark gate) but their deltas are missing from `nested_meter_readings_cdk2`. This plan closes that gap.

**Spec:** `docs/superpowers/specs/2026-03-27-meter-enrichment-design.md`

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                   LATE DATA RECOMPUTATION                       │
│                                                                 │
│  LATE_ARRIVAL Kinesis ──► Lambda (trigger) ──► Glue Spark Job   │
│                                                                 │
│  Glue Spark Job:                                                │
│    1. Read raw_cdk (filtered by meter + time range)             │
│    2. Join meter-identity DynamoDB table                         │
│    3. ORDER BY timestamp, compute deltas via LAG() window       │
│    4. APPEND corrected rows to nested_meter_readings_cdk2       │
│       with created = now() (event-sourcing: newest wins)        │
│                                                                 │
│  Query pattern (consumers):                                     │
│    ROW_NUMBER() OVER (PARTITION BY meter_id, timestamp          │
│                       ORDER BY created DESC) = 1                │
└─────────────────────────────────────────────────────────────────┘
```

## Design Decisions

1. **Append-only, no merge** — Consistent with the event-sourcing nature of the system. Corrected records are appended with a new `created` timestamp. Consumers query for the newest `created` per `(meter_id, timestamp)`.

2. **Glue Spark over Athena** — Athena is read-only; it cannot write back to Iceberg tables. Glue Spark can both read from `raw_cdk` and append to `nested_meter_readings_cdk2`.

3. **Targeted recomputation** — The Lambda trigger extracts the affected `daq_id` and time range from late arrival records, so the Spark job only reprocesses the relevant slice rather than the entire table.

4. **Meter identity from DynamoDB** — The Spark job reads the current meter-identity mapping from DynamoDB at job start (same table the Flink bootstrap loader uses). This is correct because meter identity is effectively immutable once assigned.

---

## File Map

### New files

| File | Responsibility |
|------|---------------|
| `infra/daq/data_pipeline/glue/late_recomputation.py` | PySpark Glue job: read raw_cdk, join identity, compute deltas, append to enriched table |
| `infra/daq/data_pipeline/lambda/late_arrival_trigger/handler.py` | Lambda: consume LATE_ARRIVAL Kinesis, batch by daq_id, trigger Glue job |
| `infra/daq/data_pipeline/lib/late-recomputation-stack.ts` | CDK stack: Glue job, Lambda trigger, IAM roles |

### Modified files

| File | Change |
|------|--------|
| `infra/daq/data_pipeline/lib/flink_transforms-stack.ts` | Export LATE_ARRIVAL stream ARN for cross-stack reference |

---

## Implementation Steps

### Step 1: Glue Spark Recomputation Job

- [ ] **1.1** Create `glue/late_recomputation.py` PySpark script
  - Accept job parameters: `--daq_ids` (comma-separated), `--time_range_start`, `--time_range_end`, `--region`, `--meter_identity_table`, `--table_bucket_arn`
  - Read from `raw_cdk` Iceberg table filtered by `daq_id IN (...)` and `timestamp BETWEEN start AND end`
  - Read meter identity from DynamoDB using `dynamodb.scan` with filter on relevant daq_ids
  - Join raw records with meter identity on `daq_id`
  - Partition by `meter_id`, order by `timestamp`, compute delta using `LAG()` window function:
    ```python
    window = Window.partitionBy("meter_id").orderBy("timestamp")
    df = df.withColumn("prev_value", F.lag("value").over(window))
    df = df.withColumn("delta", F.col("value") - F.col("prev_value"))
    # First record per meter has no delta — filter out
    df = df.filter(F.col("prev_value").isNotNull())
    ```
  - Flag negative deltas as anomalies (log warning, still append)
  - Set `created = current_timestamp()` on all output rows
  - Append to `nested_meter_readings_cdk2` Iceberg table via `df.writeTo(...).append()`
  - Output schema matches enriched table: `meter_id, timestamp, value (delta), unit, created, partner_id, company_id, property_id, building_id, area_id, group_id`

- [ ] **1.2** Add unit tests for the delta computation logic
  - Extract the window function logic into a testable transform function
  - Test: in-order sequence produces correct deltas
  - Test: negative delta flagged but still appended
  - Test: single record per meter produces no output

### Step 2: Lambda Trigger

- [ ] **2.1** Create `lambda/late_arrival_trigger/handler.py`
  - Kinesis event source: reads from LATE_ARRIVAL stream
  - Batch incoming late arrival records by `daq_id`
  - For each batch, compute the time range (min/max timestamp)
  - Deduplicate: check if a Glue job for the same daq_id + time range is already running (via Glue `get_job_runs` API with status filter)
  - Start Glue job run with parameters: `--daq_ids`, `--time_range_start`, `--time_range_end`
  - Add a debounce window (e.g., collect for 5 minutes via SQS buffer or Lambda batching window) to avoid triggering a Glue job per individual late record

- [ ] **2.2** Add error handling
  - If Glue job start fails, log to CloudWatch and send to a DLQ
  - Idempotent: re-triggering the same time range just appends duplicate corrections (consumers deduplicate via `ROW_NUMBER()` query pattern)

### Step 3: CDK Infrastructure

- [ ] **3.1** Create `lib/late-recomputation-stack.ts`
  - Glue job definition:
    - PySpark 3.x, Glue 4.0
    - Worker type: G.1X, 2 workers (small jobs, targeted time ranges)
    - Script location: S3 (deployed from `glue/late_recomputation.py`)
    - Extra JARs: Iceberg runtime, S3 Tables catalog JAR
    - Default arguments: `--region`, `--meter_identity_table`, `--table_bucket_arn`
  - Lambda function:
    - Runtime: Python 3.12
    - Event source: LATE_ARRIVAL Kinesis stream (batch size 100, batch window 300s)
    - Environment: Glue job name, region
  - IAM roles:
    - Glue role: S3 Tables read/write, DynamoDB read (meter-identity table), CloudWatch logs
    - Lambda role: Kinesis read (LATE_ARRIVAL stream), Glue StartJobRun, CloudWatch logs

- [ ] **3.2** Export LATE_ARRIVAL stream ARN from `flink_transforms-stack.ts` for cross-stack reference

### Step 4: Historical Backfill Support

- [ ] **4.1** Add a CLI/script mode for the Glue job that accepts broader parameters
  - `--full_backfill=true` flag: reprocess all records in `raw_cdk` (no daq_id filter)
  - `--daq_ids=*` or specific list
  - This enables manual backfill when needed (e.g., after a code fix, schema migration)

- [ ] **4.2** Document the backfill procedure
  - How to trigger manually via AWS Console or CLI
  - Expected runtime estimates based on data volume
  - How to verify results in Athena

### Step 5: Consumer Query Pattern

- [ ] **5.1** Document the deduplication query pattern for downstream consumers
  ```sql
  -- Get latest version of each reading
  SELECT * FROM (
    SELECT *,
      ROW_NUMBER() OVER (
        PARTITION BY meter_id, timestamp
        ORDER BY created DESC
      ) AS rn
    FROM "all"."nested_meter_readings_cdk2"
  ) WHERE rn = 1
  ```

- [ ] **5.2** Create an Athena named query / view for the deduplicated view
  - `CREATE VIEW enriched_readings_latest AS ...`
  - This shields consumers from the event-sourcing complexity

---

## Testing Strategy

1. **Unit tests** for Spark delta computation logic (local PySpark)
2. **Integration test**: inject a known late record into LATE_ARRIVAL stream, verify Glue job runs and appended record appears in enriched table with correct delta
3. **Idempotency test**: run recomputation twice for same range, verify consumer query returns same results (ROW_NUMBER dedup)
4. **Backfill test**: run full backfill on a small dataset, compare total deltas against raw cumulative differences

---

## Rollout

1. Deploy Glue job + Lambda trigger to staging
2. Manually inject test late arrivals, verify end-to-end
3. Deploy to production with Lambda trigger disabled (manual-only mode initially)
4. Enable Lambda trigger after confidence period
5. Create the Athena view for consumers
