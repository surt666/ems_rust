# DAQ Pipeline — System Design

> Markdown companion to `docs/system-design-presentation.html` (the slide deck). This describes
> the **DAQ / measurement pipeline** (account `891377204778`) as it works *now*. The OCaml
> **hierarchy service** (account `339712745226`) is documented separately in
> [`architecture.md`](architecture.md), [`hierarchy-and-sensors.md`](hierarchy-and-sensors.md),
> and [`api.md`](api.md). Deploy procedures live in the repo-root `CLAUDE.md`.

## 1. Overview

Measurements flow from devices into a streaming pipeline that writes two Iceberg tables
(`raw_data`, `logical_meter_data`) and, hourly, rolls counter consumption into a DynamoDB
**materialized view** (`measurements_aggregate`) for fast hierarchy-path queries.

```
sources ─▶ Kinesis ─▶ Flink (parse ─▶ split) ─┬─▶ Raw Row Mapper ─▶ Iceberg raw_data
                                               └─▶ Enrich ─▶ Resample ─▶ Iceberg logical_meter_data
                                                                                  │ (hourly Glue)
                                                                                  ▼
                                                                    DynamoDB measurements_aggregate
```

The whole pipeline is **event-sourced / append-only**: rows are appended with
`ingested_time = now()`, never updated or deleted. Consumers take the **newest `ingested_time`**
per `(logical_id, resample_timestamp)`.

## 2. Ingestion

- **IoT devices** (EMU, FlowIQ, MC603, Pulse, Electrocom, GWB-143, …) publish via LoRaWAN / MQTT /
  HTTPS to **AWS IoT Core**, whose rules engine forwards to a **Kinesis** source stream.
- **Pull APIs** (DataHub, Brunata, Techem, Danfoss, …) and **CSV import** write to the same Kinesis
  stream directly — manual/CSV readings enter the *same* pipeline, not a side-channel.

## 3. Flink job (`flink-iceberg-processor`, MSF)

Scala fat-JAR on Managed Service for Flink. After a **Payload Processor** parses each record
(rejecting corrupt payloads to `PARSE_ERROR`), the stream **splits** into two branches:

**Raw branch** — a Raw Row Mapper writes every reading verbatim to Iceberg `raw_data`.

**Enrichment branch**
1. **Watermark** — 1h bounded out-of-orderness; a 6h buffer absorbs late arrivals.
2. **Enrichment / identity lookup** — joins the reading with its `MeterMapping` (logical id, meter
   type, hierarchy path `hn1..hn9`, purpose, `resample_minutes`, formula). The mapping comes from
   the **`meter-identity`** DynamoDB table, broadcast into the operator via a **DDB CDC stream**
   (and bootstrap-scanned in `open()`). No mapping ⇒ `DEAD_LETTER`.
3. **Resample** (`ResampleFunction`, replaced `CounterDeltaFunction`) — see §4.
4. Writes enriched rows to Iceberg `logical_meter_data`. Side outputs: `ANOMALY` (negative counter
   delta), `LATE_ARRIVAL` (predecessor already purged) → an **error Kinesis sink** / DLQ.

## 4. Resampling

For a meter with `resample_minutes = m` set on its `meter-identity` row, `ResampleFunction` holds a
one-reading lag and, when the next reading arrives, emits **one row per resample grid point** in
`(prev_ts, current_ts]`:

- `resample_value = delta × overlap / total_period` (counters: time-proportional split of the
  delta across overlapping grid windows; gauges: linear interpolation).
- The original `timestamp` and `value` are preserved; `resample_timestamp`, `resample_value`,
  `resample_method` are added.
- **Energy conservation:** `SUM(resample_value)` for one reading equals its delta; consumers
  `SUM` per `(logical_id, resample_timestamp)` after newest-`ingested_time` dedup.
- A meter **without** `resample_minutes` passes through raw (no resample columns).

**Naming (2026-06):** the attribute is `resample_minutes`, the operator is `ResampleFunction`
(uid `"resample"`, keyed-state `"resample-*"`), and the Iceberg columns are
`resample_value / resample_method / resample_timestamp`. The earlier `binning` attribute and its
fallback reads were **removed** — there is no `binning` fallback anywhere; the bridge and pipeline
are a strict `resample_minutes` contract. (Internal algorithm code still uses "bin" for the grid
points/windows — `binSizeMs`, `computeBins` — but nothing named "bin" is persisted or part of any
contract.)

## 5. Iceberg tables (S3 Tables, namespace `all`)

- **`raw_data`** — `daq_id, timestamp, value, unit, ingested_time` (partition: month + daq_id
  bucket). Raw readings, kept.
- **`logical_meter_data`** — `logical_id, timestamp, value, unit, ingested_time, hn1..hn9, purpose,
  resample_value, resample_method, resample_timestamp` (partition: month + hn2 bucket). The
  enriched/derived table; source for the aggregate view.

> `AWS::S3Tables::Table` **cannot be replaced in place** — renaming columns requires a two-step
> delete+recreate (see `CLAUDE.md`). That's how `logical_meter_data` got its `resample_*` columns.

## 6. Late recomputation (Glue)

`late-data-recomputation` (PySpark) recomputes deltas + resample rows for late/restated data using
the **identical** formula + unit normalisation as Flink (bit-for-bit parity for the same input),
appending corrected rows (event-sourcing — never merged). Triggered by the error stream and by
`meter-identity` inserts (backfill).

## 7. Measurements Aggregate — materialized view (added 2026-06)

A DynamoDB table (`measurements_aggregate`, account `891`, on-demand, TTL) holding **pre-aggregated
counter consumption** per node / purpose / hour & day, so dashboards answer *"consumption for node
X over period P"* without scanning Iceberg. Spec:
[`docs/superpowers/specs/2026-06-07-measurements-rollup-view-design.md`](../infra/daq/data_pipeline/docs/superpowers/specs/2026-06-07-measurements-rollup-view-design.md)
(under `infra/daq/data_pipeline/`).

### Table

```
pk = "HN2#<company>"                                  # partition = one company
sk = "<full hierarchy path>#<purpose>#<gran>#<bucket>"
       company : "HN2#10003#Energy#d#2026-06-07"
       hn4     : "HN2#10003|HN3#9|HN4#456#Energy#h#2026-06-07T08"
       leaf    : "HN2#10003|HN3#9|HN4#456|L#10009#Energy#d#2026-06-07"
   gran = "h" | "d";  bucket = UTC "YYYY-MM-DDThh" | "YYYY-MM-DD"
```

- **One item at every level** (company `HN2` → intermediate `HN` nodes → leaf `L#<logical_id>`), so
  any node total is an O(1) read — the rollup job does the fan-out, not the reader.
- **Delimiter invariant:** a node's own rows append `#<purpose>…`; descendants extend the path with
  `|…`. Since `#`(0x23) < `|`(0x7C), a node's own rows sort *before* any descendant — so
  `begins_with`/`BETWEEN` on the sk give subtree and time-interval queries cleanly.
- **Attributes:** `sum, count, min, max, last_value, last_ts, unit, purpose, bucket, updated_at,
  ttl`. (`level`/`gran` are *not* stored — `gran` is in the sk, `level` derivable.)
- **TTL:** hourly buckets 90 days, daily buckets 2 years.

### Query patterns (reader always knows its path)

```
GetItem  pk="HN2#10003", sk="HN2#10003|HN3#9|HN4#456#Energy#d#2026-06-07"        # one node/bucket
query    pk=…, sk BETWEEN "…#Energy#d#2026-06-01" AND "…#Energy#d#2026-06-07"     # interval series
BatchGetItem  [node + ancestor sks]                                              # breadcrumb totals
```

### Population (hourly Glue, `measurements-aggregate`)

1. Read `logical_meter_data` windowed by **`resample_timestamp ≥ 00:00 UTC of (today − N days)`**
   (`--lookback_days`, default 1) — whole UTC days, so each recomputed bucket is summed from *all*
   its points (closed days complete; the current day a correct running partial). Restatements with
   `resample_timestamp` older than the window are a documented hook (widen `N`).
2. **Take the newest `ingested_time`** per `(logical_id, resample_timestamp)` — the table is
   append-only, so superseded rows are dropped before aggregating (no double-counting). Then keep
   **counters** only (`resample_method = 'time_proportional'`, non-null `resample_value`/`hn2`).
3. Explode each reading into its ancestor node keys; group by `(node, purpose, gran, bucket)`;
   compute `sum/count/min/max` of `resample_value`, `last_value`/`last_ts` from the latest reading,
   carry `unit`.
4. **Upsert** (`PutItem` overwrite → idempotent; recompute = restate, never double-count). Batched.

### Storage characteristics

Every measurement is written at **every hierarchy level**, so one meter's series is multiplied by
its path depth. Leaf storage grows with *#meters × buckets*; upper-level storage grows with
*#nodes × buckets* (not #meters) and there the rows are *real sums*, not duplicates — so the
multiplier is bounded by path depth (~4–8), not sensor count. TTL bounds growth over time.

*(As of 2026-06-07, the backfill over ~38 days of one Energy meter produced ~3,676 items / ~0.8 MB —
919 buckets × 4 levels.)*

## 8. Cross-account bridge

A Lambda in the **hierarchy account** reacts to `hierarchy_new` sensor changes and writes the
per-sensor `resample_minutes` (plus identity/path/formula) to the **`meter-identity`** table in the
DAQ account. The pipeline reads **only** `resample_minutes` — strict contract, no fallback. See
`infra/daq/data_pipeline/memory/cross_account_bridge.md` and `CLAUDE.md` for the field contract and
deploy ordering.

## 9. Stacks (CDK, Go — account `891`)

| Stack | Owns |
|---|---|
| `DaqPipelineStack` | the MSF Flink app, Kinesis streams, `meter-identity` table |
| `LateRecomputationStack` | the `late-data-recomputation` Glue job + triggers |
| `S3TablesStack` | the Iceberg tables (`raw_data`, `logical_meter_data`, `hierarchy`) |
| `OcamlBridgeWriterRoleStack` | IAM role the cross-account bridge assumes |
| `MeasurementsAggregateStack` | `measurements_aggregate` table + hourly `measurements-aggregate` Glue job + schedule |

Build + deploy commands and the MSF/S3Tables gotchas are in the repo-root **`CLAUDE.md`**.
