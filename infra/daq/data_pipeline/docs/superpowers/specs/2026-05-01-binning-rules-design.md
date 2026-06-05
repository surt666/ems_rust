# Binning Rules for Irregular Time-Series Measurements — Design Spec

**Date:** 2026-05-01
**Status:** Implemented (deployed to production v49 on 2026-05-02)
**Supersedes (binning section of):** `2026-03-27-meter-enrichment-design.md`

## Context

Sensor measurements arrive at irregular intervals but consumers (dashboards, aggregations, billing) need values aligned to fixed bin boundaries (`:00`, `:15`, `:30`, `:45` for 15-min bins, whole hours, etc.). Today the Flink pipeline floors raw timestamps to the nearest bin and the Glue late-recomputation job does no binning at all — they are not aligned, and the floor-only approach loses information.

This spec defines a single set of binning rules applied identically by both the Flink streaming job (`flink_app_scala`) and the Glue batch late-recomputation job (`glue/late_recomputation.py`).

## Binning Rules

Gauges and counters use **different bin-enumeration rules** because they answer different questions:

- **Gauge** answers *"what was the instantaneous value at bin boundary B?"* — only emit bins whose boundary has been *passed* by the current reading (i.e., `B in (prev_ts, current_ts]`).
- **Counter** answers *"how much was consumed during bin window [B − binSize, B]?"* — emit every bin whose **window** overlaps with the reading's period `[prev_ts, current_ts]`. A single reading may contribute to multiple bins, **and a single bin may receive contributions from multiple readings** when consecutive reading boundaries fall inside the same bin window.

### Gauge values (instantaneous: temperature, voltage, power in W)

- `bin_timestamp` — each bin boundary `B` in `(prev_ts, current_ts]`
- `bin_value` — linear interpolation between `(prev_ts, prev_value)` and `(current_ts, current_value)` evaluated at `B`
- Single-neighbor edge case (first/last reading per meter, no counterpart) → no row emitted for that bin

In normal operation (one reading per bin period) there is exactly one `B` in the period and it equals `round(current_ts, bin_size)` to within the round-half-up convention.

### Counter values (accumulated delta: kWh since last reading)

- `bin_timestamp` — each bin boundary `B` such that the bin window `[B − bin_size, B]` overlaps with `[prev_ts, current_ts]`. **A single reading may contribute to multiple bins; a single bin may receive contributions from multiple readings.**
- `bin_value` — time-proportional share of the **delta** (`current_value − prev_value`):
  - `overlap = min(current_ts, B) − max(prev_ts, B − bin_size)`
  - `bin_value = delta × overlap / (current_ts − prev_ts)`
- Energy is conserved per reading: `sum(bin_value)` across all bins emitted by one reading equals that reading's `value` (delta) field
- Energy is also conserved per bin across readings: total energy in bin `B` equals `sum(bin_value)` across every reading whose period overlaps `[B − bin_size, B]`. **Consumers must sum after dedup**, see [Consumer query](#consumer-query) below.

### `bin_method` audit string

- `"linear_interpolation"` — gauge
- `"nearest_neighbor"` — defined in the rules for the single-neighbor edge case (first/last reading per meter without a counterpart). **Not emitted by the current implementation** — both Flink and Glue skip bins that have no bracketing pair. Reserved for a future enhancement.
- `"time_proportional"` — counter

## Output Schema

### `logical_meter_data` — three columns added

The schema migration must run via Athena (the CDK `CfnTable` resource cannot do schema evolution; deploying the S3TablesStack with new columns triggers a table replacement → data loss):

```sql
ALTER TABLE all.logical_meter_data ADD COLUMNS (
  bin_value     double,
  bin_method    string,
  bin_timestamp timestamp
);
```

Notes:
- All three are nullable.
- **Athena `ADD COLUMN`/`ADD COLUMNS` drops the `WITH TIME ZONE` clause** — `bin_timestamp` ends up as plain `timestamp` in the Iceberg schema regardless of how it's declared. The Flink row mapper writes `LocalDateTime` (UTC) accordingly.
- **Field-ID order matters.** Iceberg matches by field ID, not name. If `bin_timestamp` is dropped and re-added, it gets a new field ID and lands at the end of the table. The CDK schema and the Flink row mapper must list columns in the same physical order: `bin_value, bin_method, bin_timestamp`.

### Existing column semantics

| Column | Gauge | Counter |
|---|---|---|
| `timestamp` | original reading time, un-floored (behavior change — old `binTimestamp` floor removed) | original reading time, un-floored |
| `value` | instantaneous reading (after unit normalization) | computed delta (after unit normalization) |
| `bin_timestamp` | each bin boundary in `(prev_ts, current_ts]` | each bin boundary whose **window** overlaps `[prev_ts, current_ts]` |
| `bin_value` | linearly interpolated (unit-normalized) | time-proportional share of delta (unit-normalized) |
| `bin_method` | `"linear_interpolation"` | `"time_proportional"` |

### Meters with `binning IS NULL`

Bin columns written as `NULL`. The raw row is still emitted (preserves backward compatibility for unconfigured meters).

### CDK / table definition

Update the Iceberg table definition in CDK to include the three new columns. The `Main.scala` Iceberg sink schema and the Glue job's `build_output` helper both extend to write the new columns.

## Topology

### Flink

```
Kinesis raw → JSON parse → SensorRecord
   │
   ├─ raw branch ──→ raw_data Iceberg sink (unchanged, immediate write)
   │
   └─ enrichment branch ──→ MeterEnrichmentFunction (broadcast: meter-identity)
                              │   produces (EnrichedRecord, MeterMapping) tuples
                              │   timestamp NO LONGER floored here
                              ▼
                            keyBy(logicalId) ──→ BinningFunction (replaces CounterDeltaFunction)
                              │   per-meter keyed state: prev reading + buffer for out-of-order
                              │   emits 0..N rows per input reading
                              ▼
                            logical_meter_data Iceberg sink
                              │
                              ├─ side output: ANOMALY (negative counter delta)
                              ├─ side output: LATE_ARRIVAL (predecessor purged)
                              └─ side output: DEAD_LETTER (unchanged)
```

### Glue (late recomputation)

```
raw_data → join meter-identity (DDB scan) → window per logical_id ordered by timestamp
   │   compute prev/next via LAG/LEAD; for each reading emit bin rows
   ▼
logical_meter_data (append with new ingested_time)
```

Both paths produce identical output rows for the same input (see [Output parity invariant](#output-parity-invariant)).

### Consumer query

Bins can have multiple contributing rows. The dedup key is `(logical_id, timestamp, bin_timestamp)` (newest `ingested_time` wins for a given source-reading + bin), then sum `bin_value` per `(logical_id, bin_timestamp)`:

```sql
SELECT logical_id, bin_timestamp, unit, SUM(bin_value) AS energy
FROM (
  SELECT logical_id, timestamp, bin_timestamp, bin_value, unit,
         ROW_NUMBER() OVER (
           PARTITION BY logical_id, timestamp, bin_timestamp
           ORDER BY ingested_time DESC
         ) AS rn
  FROM all.logical_meter_data
  WHERE bin_timestamp IS NOT NULL
) WHERE rn = 1
GROUP BY logical_id, bin_timestamp, unit;
```

For gauges (where each reading contributes one row per bin and bins don't accumulate), `MAX(bin_value)` is equivalent to `SUM(bin_value)` after dedup. The query above works for both meter types.

## Flink: `BinningFunction` Operator

Replaces `CounterDeltaFunction`. Signature:

```scala
class BinningFunction(bufferRetentionMs: Long)
    extends KeyedProcessFunction[String, (EnrichedRecord, MeterMapping), EnrichedRecord]
```

### Keyed state per `logicalId`

- `MapState[Long, BufferedReadingV2]` — out-of-order buffer (same pattern as today's `CounterDeltaFunction`), keyed by event-time millis
- `ValueState[Long]` — `lastEmittedTs` to avoid re-emission

`BufferedReadingV2` carries the raw value and the full `EnrichedRecord` plus the `MeterMapping` (for `meterType` and `binning`).

### Algorithm (on input `(record, mapping)`)

```
eventTs = parseTimestamp(record.timestamp)
buffer.put(eventTs, BufferedReadingV2(record.value, record, mapping))
register event-time timer at eventTs

emitFromBuffer(eventTs, ctx, out):
  prev = findPredecessor(eventTs)   // largest ts < eventTs in buffer
  if prev is None:
    if lastEmittedTs > MIN:         // predecessor was purged → late arrival
      side-output LATE_ARRIVAL
    return                          // first reading per meter — just buffer

  binSize = mapping.binning
  if binSize is None:
    out.collect(record with bin_* = null)
    update lastEmittedTs
    return

  if meterType == "counter" and record.value < prev.value:
    side-output ANOMALY
    update lastEmittedTs            // still advance, don't get stuck
    return

  match meterType:
    case "gauge":
      // Bins B in (prev.ts, eventTs] — one per bin boundary that's been "passed"
      binBoundaries = enumerateBinsIn(prev.ts, eventTs, binSize)
      for B in binBoundaries:
        binValue = prev.value + (record.value - prev.value) * (B - prev.ts) / (eventTs - prev.ts)
        out.collect(record.copy(bin_timestamp = B, bin_value = binValue, bin_method = "linear_interpolation"))

    case "counter":
      delta = record.value - prev.value
      totalPeriod = eventTs - prev.ts
      // Bins whose window [B-binSize, B] overlaps the period [prev.ts, eventTs]
      binBoundaries = enumerateOverlappingBins(prev.ts, eventTs, binSize)
      for B in binBoundaries:
        binStart = max(prev.ts, B - binSize)
        binEnd   = min(eventTs, B)
        overlap  = binEnd - binStart
        binValue = delta * (overlap / totalPeriod)
        out.collect(record.copy(value = delta, bin_timestamp = B, bin_value = binValue, bin_method = "time_proportional"))

  update lastEmittedTs
```

### Bin enumeration

Two helpers, used by different meter types:

```scala
/** For gauges: B in (prevTs, currentTs] — only emit bins that have been "passed". */
def enumerateBinsIn(prevTs: Long, currentTs: Long, binSizeMs: Long): Seq[Long] =
  val first = ((prevTs / binSizeMs) + 1) * binSizeMs
  if first > currentTs then Seq.empty
  else (first to currentTs by binSizeMs).toSeq

/** For counters: B such that bin window [B-binSize, B] overlaps with [prevTs, currentTs]. */
def enumerateOverlappingBins(prevTs: Long, currentTs: Long, binSizeMs: Long): Seq[Long] =
  val first = ((prevTs / binSizeMs) + 1) * binSizeMs
  val last  = ((currentTs - 1) / binSizeMs + 1) * binSizeMs
  if first > last then Seq.empty
  else (first to last by binSizeMs).toSeq
```

The gauge enumeration is right-inclusive of `currentTs` and left-exclusive of `prevTs`. The counter enumeration extends past `currentTs` to include any bin whose window straddles the right edge of the period. Both walk the bin grid (multiples of `binSize` from epoch UTC).

### Unit normalization in the row mapper

`Main.scala` calls `Extensions.normalizeUnit(record.unit, record.value)` to convert the raw sensor unit (e.g., `"Energy (100 Wh)"`) to a canonical unit (`"Wh"`) and scale the value by the same factor. `bin_value` **must** use the same factor — otherwise `value` and `bin_value` end up in different units and consumer math breaks. Implemented as:

```scala
val (normalizedUnit, normalizedValue) = Extensions.normalizeUnit(record.unit, record.value)
val normalizedBinValue =
  if record.binValue == null then null
  else Extensions.normalizeUnit(record.unit, record.binValue.doubleValue())._2
```

### Out-of-order handling

Same MapState buffer pattern as today's `CounterDeltaFunction`. On event-time timer fire:
- Replay sorted entries from buffer
- Emit per-bin rows for any consecutive pair `(prev, reading)` where `reading.ts > lastEmittedTs` and `reading.ts <= currentWatermark`
- Purge entries older than `currentWatermark - bufferRetentionMs`, keeping the most recent purged entry as predecessor anchor

If a reading arrives after its predecessor has been purged, it routes to `LATE_ARRIVAL` for Glue recomputation.

### Side outputs

- `ANOMALY` — counter `value < prev.value` (negative delta). `lastEmittedTs` still advances.
- `LATE_ARRIVAL` — predecessor purged. Both gauge and counter (today only counter).
- `DEAD_LETTER` — unchanged (raised by `MeterEnrichmentFunction` when DDB mapping missing).
- `PARSE_ERROR` — unchanged.

## Glue: `late_recomputation.py`

### Per-meter window

```python
window = Window.partitionBy("logical_id").orderBy("timestamp")
joined = joined.withColumn("prev_ts",    F.lag("timestamp").over(window)) \
               .withColumn("prev_value", F.lag("value").over(window)) \
               .withColumn("next_ts",    F.lead("timestamp").over(window)) \
               .withColumn("next_value", F.lead("value").over(window))
```

### Counters

```python
counters = joined.filter(F.col("meter_type") == "counter")
counters = counters.withColumn("prev_ts",    F.lag("timestamp").over(window))
counters = counters.withColumn("prev_value", F.lag("value").over(window))
counters = counters.filter(F.col("prev_value").isNotNull())  # drop first reading per meter
counters = counters.withColumn("delta", F.col("value") - F.col("prev_value"))
counters = counters.filter(F.col("delta") >= 0)              # drop anomalies (alerting stays in Flink)
counters = counters.withColumn("value", F.col("delta"))      # value column = delta

binned = counters.filter(F.col("binning").isNotNull()) \
    .withColumn("bins", enumerate_overlapping_bins_udf(
        (F.unix_timestamp("prev_ts") * 1000).cast("long"),
        (F.unix_timestamp("timestamp") * 1000).cast("long"),
        F.col("binning"),
    )) \
    .withColumn("bin_timestamp_ms", F.explode("bins")) \
    .withColumn("bin_timestamp", (F.col("bin_timestamp_ms") / 1000).cast(TimestampType()))

bin_size_ms = F.col("binning").cast("long") * F.lit(60 * 1000)
prev_ts_ms  = F.unix_timestamp("prev_ts") * 1000
cur_ts_ms   = F.unix_timestamp("timestamp") * 1000
binned = binned.withColumn(
    "bin_value",
    F.col("delta") *
    (F.least(cur_ts_ms, F.col("bin_timestamp_ms")) - F.greatest(prev_ts_ms, F.col("bin_timestamp_ms") - bin_size_ms))
        .cast("double") /
    (cur_ts_ms - prev_ts_ms).cast("double"),
).withColumn("bin_method", F.lit("time_proportional"))
```

### Gauges

```python
gauges = joined.filter(F.col("meter_type") == "gauge")
gauges = gauges.filter(F.col("prev_ts").isNotNull())   # first reading per meter has no prev → no bins emitted

# Fan out across bins in (prev_ts, current_ts] with linear interpolation
gauges = gauges.withColumn("bins", F.expr("enumerate_bins(prev_ts, timestamp, binning)")) \
               .withColumn("bin_timestamp", F.explode("bins"))

gauges = gauges.withColumn("bin_value",
    F.col("prev_value") + (F.col("value") - F.col("prev_value")) *
    (F.unix_timestamp("bin_timestamp") - F.unix_timestamp("prev_ts")) /
    (F.unix_timestamp("timestamp") - F.unix_timestamp("prev_ts"))
)
gauges = gauges.withColumn("bin_method", F.lit("linear_interpolation"))
```

This matches the Flink operator exactly: each bin in `(prev_ts, current_ts]` emits one row with linear interpolation. A meter with only a single reading in the dataset produces no bin rows (consistent with Flink's one-reading-lag model). The `nearest_neighbor` method is defined in the rules for future use (e.g., a batch-tail edge-fill enhancement) but is not actively emitted by either Flink or Glue under the current design.

### Output union & normalization

```python
result = counters.unionByName(gauges, allowMissingColumns=True)
output = build_output(result)   # applies normalize_unit to value AND bin_value
output.writeTo("all.logical_meter_data").append()
```

`build_output` projects the final schema and applies `normalize_unit` (Python port of Scala `Extensions.normalizeUnit`) to both `value` and `bin_value` with the same factor. The `unit` column is replaced with the canonical unit name. This mirrors what Flink's row mapper does — required by the [output parity invariant](#output-parity-invariant).

### Meters with `binning IS NULL`

Skip the bin enumeration; emit one row per reading with the three bin columns NULL. For counters, still apply delta + anomaly filter as today.

### UDFs

- `enumerate_overlapping_bins(prev_ts, current_ts, binning_minutes)` — for counters; bins `B` whose window `[B-binSize, B]` overlaps with `[prev_ts, current_ts]`. Mirrors `BinningFunction.enumerateOverlappingBins`.
- `enumerate_bins(prev_ts, current_ts, binning_minutes)` — for gauges; bins `B` where `prev_ts < B <= current_ts`. Mirrors `BinningFunction.enumerateBinsIn`.
- `_normalize_unit_name`, `_normalize_unit_factor` — port of Scala `Extensions.normalizeUnit`; applied to both `value` and `bin_value` in `build_output`.

### Output parity invariant

**For a given `(raw_data row, meter mapping)`, Glue and Flink must produce bit-identical output rows in `logical_meter_data`.**

This is non-negotiable: late-arriving rows reprocessed by Glue replace Flink's earlier writes (newest `ingested_time` wins). If the two paths produce different values, consumers see oscillating values whenever a backfill runs. Both paths share:

- The same bin enumeration (`enumerate_overlapping_bins` for counters; `enumerate_bins` for gauges)
- The same overlap formula (`min(currentTs, B) - max(prevTs, B - binSize)`)
- The same unit-normalization table (Scala `Extensions.UnitConversions` ↔ Python `_UNIT_CONVERSIONS`)
- The same per-bin row layout (`(timestamp, value=delta, bin_timestamp, bin_value, bin_method)`)

Any change to one side must be mirrored in the other in the same commit.

## Testing Strategy

Three layers extend the existing scenario-test framework (`docs/superpowers/specs/2026-04-05-scenario-test-automation-design.md`).

### Harness tests (Scala, ~5s)

- `BinningFunction.computeBins` — pure function; no Flink state
- `enumerateBinsIn` (gauge) — single bin, multi-bin gap, exact boundary alignment, empty period
- `enumerateOverlappingBins` (counter) — same set of cases plus *period straddles a bin boundary mid-period* (one reading contributes to two bins)
- Linear interpolation: known prev/current pairs; expected `bin_value` at known boundaries
- Time-proportional split: assert `sum(bin_values) == delta` per reading (energy-conservation invariant)
- Bin-boundary alignment: reading whose `timestamp` is exactly on a bin boundary (right-inclusive: included as the bin emitted by that reading, excluded as predecessor anchor for the next)
- Meter with `binning = null` → raw row only, bin columns null
- First reading per meter → no emission, state populated
- Counter negative delta → ANOMALY, state advances

### Mini-cluster tests (Scala, ~30s)

Full operator chain via `MiniClusterWithClientResource`:

- Gauge end-to-end (3 readings, normal spacing) → 2 emitted bins after first; correct interpolation
- Counter end-to-end (3 readings, normal spacing) → 2 emitted bins; sum invariant per period
- Gauge with gap (prev 10:00, next 10:45, 15-min bins) → 3 bins (10:15, 10:30, 10:45), each linearly interpolated
- Counter with gap → 3 bins; `bin_value` summed equals delta
- Out-of-order within watermark → buffered, emitted correctly post-watermark
- Late arrival (predecessor purged) → `LATE_ARRIVAL` side output for both gauge and counter
- Counter negative delta → `ANOMALY`
- Meter with `binning = null` → raw row only

### Glue tests (Python, local SparkSession)

- Same scenarios as mini-cluster, in batch
- Single-reading-only meter → no bin rows emitted (consistent with Flink behavior)
- Energy conservation invariant on counters
- Idempotency: running Glue twice on the same input produces identical `bin_value`s (only `ingested_time` differs)

### Cross-implementation parity

A parametrised test feeds the same `(prev, current, binning, meter_type)` tuples through both:
- The Scala `BinningFunction.computeBins` pure function
- The Python equivalent used by Glue UDFs

Asserts equal `bin_timestamp` / `bin_value` / `bin_method` outputs. Catches drift between implementations.

### Smoke tests

Extend the existing 4 smoke scenarios:
- Inject a counter with a 45-min gap → query Athena → assert 3 bin rows; `SUM(bin_value)` equals delta
- Inject a gauge → query Athena → assert correct interpolated `bin_value`

## Documentation Updates

Implementation plan must include updates to:

- `docs/system-design.md` — binning section
- `docs/data-ingestion-scenarios.md` + `.html` — three new scenarios (gauge interpolation, counter time-proportional split, gap handling)
- `docs/03-flink-internals.png` (regenerate via `generate_diagrams.py`) — rename `CounterDeltaFunction` → `BinningFunction`
- `docs/superpowers/specs/2026-03-27-meter-enrichment-design.md` — header note: "Binning section superseded by 2026-05-01-binning-rules-design.md"

## Migration & Rollout

1. **Iceberg schema migration via Athena** (do NOT redeploy `S3TablesStack` — `CfnTable` doesn't support schema evolution and would replace the table → data loss). Run:
   ```sql
   ALTER TABLE all.logical_meter_data ADD COLUMNS (
     bin_value     double,
     bin_method    string,
     bin_timestamp timestamp
   );
   ```
   Update the CDK schema in the same column order so future fresh deploys stay in sync.

2. **Build the Flink JAR** (`sbt assembly`).

3. **Deploy `DaqPipelineStack`** with the new JAR. The MSF app rolls out via the `DeployFlinkApp` custom resource.
   - Operator UIDs and keyed-state names changed (`counter-delta` → `binning`, `reading-buffer` → `binning-reading-buffer`). The previous checkpoint cannot be restored, so MSF must start fresh — the deploy restart automatically uses `SKIP_RESTORE_FROM_SNAPSHOT`. Each meter loses up to one bin's worth of attribution on the first reading after restart (the new buffer is empty until the second reading arrives).
   - If the deploy fails and rolls back but gets stuck (`UPDATE_ROLLBACK_IN_PROGRESS` past 30 min), `aws kinesisanalyticsv2 stop-application --force` unsticks MSF and lets the rollback finish. Then redeploy.

4. **Deploy `LateRecomputationStack`** with the updated Glue script.

5. **Optional**: schedule a one-time Glue job over historical `raw_data` to backfill `bin_*` columns for existing rows (out of scope for this spec).

## Out of Scope

- Backfilling bin columns for already-written historical rows in `logical_meter_data`
- Changing the bin-size definition for any meter (operational concern, separate workflow)
- Real-time alerts on missing bins (consumers can detect via gaps in `bin_timestamp` series)
- Sub-minute bin sizes (current `binning` column is integer minutes; sufficient for the foreseeable need)
