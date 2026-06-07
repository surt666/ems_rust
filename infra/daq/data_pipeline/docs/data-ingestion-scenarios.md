# Data Ingestion Scenarios

This document describes the full spectrum of data ingestion scenarios for the DAQ pipeline, from normal operation through failure modes. Each scenario traces the path a message takes from arrival through the Flink topology, detailing how the system handles it and what the downstream consumer sees.

## Pipeline Context

The pipeline processes IoT sensor data from two meter types:
- **Gauge meters** — instantaneous readings (e.g., temperature, power). The raw value is the final value.
- **Counter meters** — cumulative readings (e.g., total kWh, total m³). The system computes deltas between consecutive readings to derive per-interval consumption.

All raw records are written unconditionally to the `raw_data` Iceberg table. The enrichment branch resolves sensor identities, computes deltas for counters, and writes to `logical_meter_data`. For meters with a `resample_minutes` configuration on `meter-identity`, the same record produces one row per bin per the resampling rules (gauge: linear interpolation at each bin boundary in `(prev, current]`; counter: time-proportional split across every bin window the period overlaps). Consumers query `logical_meter_data` with newest-`ingested_time` dedup on `(logical_id, timestamp, resample_timestamp)`, then `SUM(resample_value) GROUP BY (logical_id, resample_timestamp)`. Full rules: `superpowers/specs/2026-05-01-resampling-rules-design.md`.

**Key parameters:**
- Watermark out-of-orderness: **1 hour** (`MAX_OUT_OF_ORDERNESS_MS`)
- Counter buffer retention: **6 hours** (`BUFFER_RETENTION_MS`)
- Checkpoint interval: **5 minutes**

---

## Scenario 1: Normal In-Order Delivery

**Trigger:** Messages arrive in timestamp order with minimal delay (< seconds).

**Path:**
1. Kinesis Source reads the JSON payload.
2. JSON Parser routes to the device-specific processor (EMU, FLOWIQ, etc.), producing a `SensorRecord`.
3. **Raw branch:** Record is mapped to an Iceberg `Row` and written to `raw_data` at next checkpoint.
4. **Enrichment branch:** Watermark assigner stamps the record. MeterEnrichmentFunction looks up the `daqId` in broadcast state (or bootstrap cache), resolves `logicalId`, hierarchy IDs, and `meterType`.
5. **Gauge:** `ResampleFunction` passes the record through immediately with raw value.
6. **Counter:** Record is added to the event-time buffer keyed by `logicalId`. `emitFromBuffer` finds the predecessor in the sorted buffer, computes `delta = current - previous`, and emits immediately. An event-time timer is also registered (no-op in this case since the watermark hasn't passed it yet; it will fire later and find the delta already emitted).
7. Enriched record is written to `logical_meter_data` at next checkpoint.

**Consumer sees:** One or more rows per reading in `logical_meter_data` (one per bin emitted; for an unconfigured meter, one row with `bin_*` = NULL). Counter rows carry the delta in `value` and the time-proportional share in `resample_value`. Gauge rows carry the raw reading in `value` and the linearly interpolated value at the bin boundary in `resample_value`. Data appears within one checkpoint interval (~5 min).

**Error outputs:** None.

---

## Scenario 2: Slightly Out-of-Order Delivery (Within Watermark Window)

**Trigger:** Messages arrive out of timestamp order but all within the 1-hour watermark window. Common with multi-gateway IoT networks where different LoRaWAN gateways have different transmission delays.

**Example timeline:**
```
Event timestamps:  T1=10:00  T2=10:15  T3=10:30
Arrival order:     T2 arrives first, then T1, then T3
All arrive within the same wall-clock hour
```

**Path:**
1. T2 arrives first → added to buffer, baseline (no predecessor), no delta emitted yet.
2. T1 arrives (earlier timestamp, but still within watermark) → added to buffer. Buffer now has `[T1, T2]` sorted. `emitFromBuffer(T1)` finds no predecessor → baseline. Timer registered for T1.
3. T3 arrives → added to buffer `[T1, T2, T3]`. `emitFromBuffer(T3)` finds predecessor T2, emits `delta(T3) = value(T3) - value(T2)`.
4. Watermark advances past T1 → `onTimer(T1)` fires. Finds pair `(T1, T2)` where T2 hasn't been emitted via timer yet → emits `delta(T2) = value(T2) - value(T1)`.
5. Watermark advances past T2 → `onTimer(T2)` fires. T2 already emitted → skip (lastEmittedTs check).

**Consumer sees:** All deltas eventually appear correctly. The order of appearance in `logical_meter_data` may not match event-time order, but this is irrelevant — consumers query by `(logical_id, timestamp)`.

**Error outputs:** None.

---

## Scenario 3: First Record for a New Meter

**Trigger:** A new meter is onboarded. The first reading arrives, but there is no previous value to compute a delta against.

**Path (counter):**
1. Record arrives, is enriched with identity.
2. `ResampleFunction.emitFromBuffer` finds the record at index 0 (no predecessor) and `lastEmittedTs` is uninitialized (`Long.MinValue`).
3. Since `lastEmitted == Long.MinValue`, this is recognized as the first-ever record → the record is silently absorbed as a baseline. No output, no error.
4. The next record for this meter will produce the first delta.

**Path (gauge):** Passes through immediately — no delta computation needed.

**Consumer sees:** Nothing for the first counter reading; all subsequent readings produce deltas. Gauges appear immediately.

**Error outputs:** None. The first counter reading being "lost" is by design — you need two points to compute a delta.

---

## Scenario 4: Meter Identity Not Found (Dead Letter)

**Trigger:** A `SensorRecord` arrives with a `daqId` that has no entry in the meter-identity DynamoDB table and no matching CDC event has been received.

**Common causes:**
- Meter was recently installed but the identity mapping hasn't been created yet.
- Typo in the device configuration (wrong `daqId`).
- DynamoDB bootstrap scan failed on startup and CDC stream hasn't caught up.

**Path:**
1. Record parsed successfully → written to `raw_data` (raw branch doesn't need identity).
2. `MeterEnrichmentFunction.processElement` checks broadcast state then bootstrap cache — neither contains the `daqId`.
3. Record is routed to `DEAD_LETTER` side output as an `ErrorRecord`.
4. Error record is serialized to JSON and written to the error Kinesis stream.

**Consumer sees:** Record exists in `raw_data` but NOT in `logical_meter_data`. Once the identity mapping is added (via DynamoDB), new records will be enriched correctly. Historical records from the dead-letter period must be recovered via the late recomputation Glue job.

**Error outputs:** `DEAD_LETTER` → error Kinesis stream.

---

## Scenario 5: Malformed / Unparseable Message

**Trigger:** The Kinesis payload is not valid JSON, has an unknown `schematype`, or is missing required fields.

**Common causes:**
- Firmware bug sending corrupt payloads.
- Network corruption.
- Unknown or newly deployed device type without a matching processor.

**Path:**
1. JSON Parser attempts to route the message to a device processor.
2. Parsing fails (invalid JSON, missing fields, unknown schema type).
3. Record is routed to `PARSE_ERROR` side output.
4. No record written to `raw_data` (the raw branch only receives successfully parsed `SensorRecord`s).
5. Error record written to the error Kinesis stream.

**Consumer sees:** Nothing — the message is effectively dropped. The error stream provides audit trail for investigation.

**Error outputs:** `PARSE_ERROR` → error Kinesis stream.

---

## Scenario 6: Negative Counter Delta (Anomaly)

**Trigger:** A counter meter reports a cumulative value that is *lower* than the previous reading, producing a negative delta.

**Common causes:**
- Counter rollover/reset (e.g., meter replaced, firmware reset, battery replacement).
- Meter malfunction reporting incorrect values.
- Historical correction data from the utility company that contradicts previously received readings.

**Path:**
1. Record is parsed and enriched normally.
2. `ResampleFunction` computes `delta = current_cumulative - previous_cumulative` and finds `delta < 0`.
3. Record is **suppressed** (not emitted downstream) and routed to the `ANOMALY` side output.
4. The `ErrorRecord` includes both the current and previous cumulative values for investigation.
5. The cumulative value IS stored in the buffer (it becomes the baseline for the next delta).

**Example:**
```
T1: cumulative = 1000 → delta from T0 = 50 (emitted)
T2: cumulative = 500  → delta = -500 (ANOMALY, suppressed)
T3: cumulative = 520  → delta = 520 - 500 = 20 (emitted normally)
```

**Consumer sees:** A gap in the data where the anomaly occurred. T2 is missing from `logical_meter_data`. The raw cumulative values are preserved in `raw_data` for manual investigation.

**Error outputs:** `ANOMALY` → error Kinesis stream.

**Note:** The system does NOT attempt to auto-correct counter rollovers. This is a deliberate design choice — automatic rollover detection is unreliable (is `1000 → 500` a rollover or a meter replacement?) and false positives corrupt downstream analytics.

---

## Scenario 7: Late Arrival Within Buffer Retention (1–6 hours late)

**Trigger:** A record arrives after the watermark has advanced past its timestamp, but the predecessor is still in the buffer (within 6-hour retention window).

**Common causes:**
- Device was offline and batch-uploads backlog when reconnecting.
- Network outage at the gateway level causing delayed forwarding.
- Kinesis shard iterator falling behind.

**Path:**
1. Record is parsed → written to `raw_data`.
2. Record reaches `ResampleFunction.processElement`.
3. `emitFromBuffer` adds the record to the buffer and finds its predecessor (still retained).
4. Delta is computed and emitted immediately.
5. An event-time timer is registered but will fire immediately (watermark already past) — `onTimer` finds the delta already emitted via `lastEmittedTs` check.

**Consumer sees:** The delta appears in `logical_meter_data`, possibly with a delay. Correctness is maintained because the predecessor was still in the buffer.

**Error outputs:** None — this is a success path, just delayed.

---

## Scenario 8: Late Arrival Beyond Buffer Retention (> 6 hours late)

**Trigger:** A record arrives after the watermark has passed AND the predecessor has been purged from the buffer (beyond 6-hour retention).

**Common causes:**
- Device was offline for days/weeks and uploads historical backlog on reconnection.
- Manual data import from a CSV or external system.
- Cross-system data reconciliation arriving late.

**Path:**
1. Record is parsed → written to `raw_data`.
2. `ResampleFunction.emitFromBuffer` adds the record to the buffer.
3. Looking for predecessor: index is 0 (nothing before it in the buffer) and `lastEmittedTs > Long.MinValue` (we've emitted deltas before, so this isn't the meter's first record).
4. Recognized as **late arrival with purged predecessor** → routed to `LATE_ARRIVAL` side output.
5. `ErrorRecord` written to error Kinesis stream.
6. `LateArrivalTrigger` Lambda consumes the error record, groups by `daq_id`, computes the time range.
7. Lambda starts a targeted Glue Spark job: `late-data-recomputation` with `--daq_ids=<affected_id> --time_range_start=<earliest> --time_range_end=<latest>`.
8. Glue job reads ALL raw records for this meter from `raw_data`, joins with meter identity from DynamoDB, computes deltas using `LAG()` window function over the full history, and appends corrected records to `logical_meter_data` with `created=now()`.

**Consumer sees:** Initially nothing for the late period. After the Glue job completes (minutes), corrected records appear with a newer `created` timestamp. Consumers using `SELECT ... WHERE created = MAX(created)` semantics pick up the corrected values automatically.

**Error outputs:** `LATE_ARRIVAL` → error Kinesis → Lambda → Glue batch job → appended corrections.

---

## Scenario 9: Massive Historical Backfill

**Trigger:** A large batch of historical data is ingested — e.g., onboarding a new customer with 12 months of historical readings, or replaying data from another system.

**Characteristics:**
- Thousands/millions of records arriving in a short wall-clock window.
- Event timestamps span months or years.
- The watermark jumps rapidly from old to new.

**Path:**
1. All records are written to `raw_data` (raw branch handles any volume).
2. For counters, `emitFromBuffer` handles the immediate delta computation:
   - Records arrive roughly in event-time order (typical for backfills) → buffer fills sequentially → deltas computed immediately via predecessor lookup.
   - Out-of-order records within the batch → buffer sorts them → deltas still correct.
3. Event-time timers fire as the watermark advances. The `lastEmittedTs` check prevents duplicate emissions (deltas already emitted by `emitFromBuffer`).
4. Buffer purging keeps memory bounded: entries older than `watermark - 6h` are cleaned up (keeping the most recent purged entry as predecessor reference).
5. Edge case: if the backfill data arrives so fast that the watermark jumps from, say, January to December in seconds, intermediate records may see "no predecessor" because earlier entries were purged before the next batch of records for that meter arrived. These go to `LATE_ARRIVAL` → Glue recomputation.

**Consumer sees:** Most deltas computed correctly in-stream. Any gaps from buffer purging races are filled by the Glue recomputation job. Final result is complete.

**Operational considerations:**
- The Flink job parallelism may need scaling for very large backfills.
- The Glue recomputation job should be run with `--daq_ids=*` after a full backfill to ensure completeness.
- Iceberg compaction may be needed after large writes (many small files).

**Error outputs:** Possible `LATE_ARRIVAL` for records caught by buffer purging during rapid watermark advancement.

---

## Scenario 10: Duplicate Messages

**Trigger:** Kinesis delivers the same record twice (at-least-once delivery), or a device sends the same reading twice.

**Path:**
1. Both copies written to `raw_data` (Iceberg append — no dedup at raw level).
2. For counters: `readingBuffer.put(eventTs, ...)` — the buffer is keyed by event timestamp (millis). The second record **overwrites** the first in the buffer. If the values are identical, the delta is unchanged. If values differ (different readings at the same millisecond), the last-write-wins.
3. For gauges: both copies pass through to `logical_meter_data`.

**Consumer sees:** Possible duplicate rows in `logical_meter_data` for gauges. Counter dedup is natural (same-timestamp overwrites in buffer). Downstream consumers should handle duplicates with `SELECT DISTINCT` or similar.

**Note:** True exactly-once is only guaranteed within the Flink checkpoint boundary. Kinesis source reprocessing after a checkpoint failure may cause limited duplicates.

---

## Scenario 11: Meter Identity Changes Mid-Stream

**Trigger:** A meter's hierarchy assignment changes — e.g., a sensor is moved from Building A to Building B, or reassigned to a different logical meter.

**Path:**
1. DynamoDB update triggers a CDC event → Kinesis DDB Change Stream.
2. `DdbStreamDeserializer` parses the event as an `IdMappingChange` with `eventType=MODIFY`.
3. `MeterEnrichmentFunction.processBroadcastElement` updates broadcast state with the new `MeterMapping`.
4. Subsequent `SensorRecord`s for this `daqId` are enriched with the **new** hierarchy IDs.

**Consumer sees:** Records before the change have old hierarchy IDs; records after have new ones. There is no retroactive correction — historical records keep their original enrichment. This is correct: the meter *was* in Building A at that time.

**Edge case:** If a DDB update and a sensor record arrive near-simultaneously, the enrichment result depends on which Flink operator processes first. This is a race condition inherent in broadcast joins, but operationally negligible (both values are "correct" at the boundary).

---

## Scenario 12: Meter Identity Deleted

**Trigger:** A meter mapping is removed from DynamoDB (device decommissioned).

**Path:**
1. CDC event with `eventType=REMOVE` updates broadcast state.
2. Subsequent records for this `daqId` find no mapping → `DEAD_LETTER`.

**Consumer sees:** Records stop appearing in `logical_meter_data` for this meter. Raw data continues to accumulate in `raw_data`. If the deletion was accidental, re-adding the mapping to DynamoDB restores enrichment for new records. Historical gap is recoverable via Glue recomputation.

**Error outputs:** `DEAD_LETTER` for all records after deletion.

---

## Scenario 13: Flink Job Restart / Checkpoint Recovery

**Trigger:** The Managed Flink application restarts from a checkpoint or savepoint (code deploy, scaling, failure recovery).

**Path:**
1. Flink restores from the latest checkpoint:
   - `readingBuffer` MapState is restored (counter buffers intact).
   - `lastEmittedTs` ValueState is restored.
   - Broadcast state (meter identity) is restored.
   - Kinesis consumer offsets are restored.
2. `MeterEnrichmentFunction.open()` performs a fresh DynamoDB bootstrap scan (independent of checkpoint state), refreshing `bootstrapCache`.
3. Processing resumes from the checkpointed Kinesis positions.
4. Any records between the last checkpoint and the restart are re-read → possible limited duplicates (see Scenario 10).

**Consumer sees:** Possible duplicate rows for the checkpoint-to-failure window. No data loss. Counter deltas remain correct because buffer state is checkpointed.

---

## Scenario 14: Kinesis Source Idle / No Data

**Trigger:** No sensor data arrives for an extended period (device offline, Kinesis stream empty).

**Path:**
1. The watermark assigner has a 24-hour idleness timeout. After 24 hours of no data on a partition, the partition is marked idle and stops holding back the global watermark.
2. Other partitions continue advancing the watermark normally.
3. When data resumes on the idle partition, the watermark assigner reactivates it.

**Consumer sees:** No new rows during the idle period (expected). No error outputs. Data resumes normally when the source reactivates.

**Risk:** If ALL partitions go idle for >24 hours and then a burst of data arrives, the watermark jumps forward significantly, potentially causing legitimate records to be classified as late arrivals (Scenario 8).

---

## Scenario Summary Matrix

| # | Scenario | Raw Table | Enriched Table | Error Output | Recovery Mechanism |
|---|----------|-----------|----------------|--------------|-------------------|
| 1 | Normal in-order | Written | Written (delta/gauge) | None | — |
| 2 | Slightly out-of-order (<1h) | Written | Written (correct delta) | None | Buffer reordering |
| 3 | First record (new meter) | Written | Skipped (baseline) | None | Next record produces delta |
| 4 | Identity not found | Written | Missing | `DEAD_LETTER` | Add mapping + Glue recompute |
| 5 | Malformed message | Not written | Not written | `PARSE_ERROR` | Fix source device |
| 6 | Negative counter delta | Written | Suppressed | `ANOMALY` | Manual investigation |
| 7 | Late arrival (1–6h) | Written | Written (correct delta) | None | Buffer still has predecessor |
| 8 | Late arrival (>6h) | Written | Initially missing | `LATE_ARRIVAL` | Auto Glue recomputation |
| 9 | Massive backfill | Written | Mostly written | Possible `LATE_ARRIVAL` | Glue recompute for gaps |
| 10 | Duplicate message | Duplicate rows | Overwrites (counter) / Duplicates (gauge) | None | Consumer-side dedup |
| 11 | Identity change | Written | New hierarchy IDs | None | By design (no retroactive) |
| 12 | Identity deleted | Written | Missing (post-delete) | `DEAD_LETTER` | Re-add mapping + Glue |
| 13 | Job restart | Re-read window | Possible duplicates | None | Checkpoint recovery |
| 14 | Source idle (>24h) | Nothing | Nothing | None | Resumes on reactivation |
