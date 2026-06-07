# Measurements Rollup Materialized View — Design

**Date:** 2026-06-07
**Status:** Approved (design); implementation plan pending
**Account:** DAQ / pipeline (`891377204778`), `eu-central-1`

## Goal

Provide a DynamoDB "materialized view" that pre-aggregates counter consumption from
`logical_meter_data` into **hourly and daily buckets**, queryable fast by **hierarchy path at
any level** and over **time intervals**. Replaces ad-hoc scans of the Iceberg table for
dashboard-style "how much did node X consume per hour/day over period P" queries.

## Source

`logical_meter_data` (Iceberg, S3 Tables, namespace `all`, account `891`). Relevant columns:
`logical_id`, `timestamp`, `value`, `unit`, `ingested_time`, `hn1..hn9` (int node ids per
level; `hn1`=partner, `hn2`=company, `hn3..hn9` nullable), `purpose`, `resample_value`,
`resample_method`, `resample_timestamp`.

The aggregated quantity is **`sum(resample_value)`** — the per-resample-interval consumption
(the delta attributed to each grid point). **Counters only** for now, discriminated by
`resample_method = 'time_proportional'` (gauges use `linear_interpolation`; non-resampled
counters have neither and are excluded).

## Data model

**Table:** `measurements_aggregate` — DynamoDB, on-demand capacity, TTL enabled on attribute `ttl`.

**One item per `(node, purpose, granularity, time-bucket)`**, pre-aggregated at **every level**
from the company (`hn2`) down to the leaf meter:

```
pk = "<hn2>"                                   # company id, string. Partition = one company.
sk = "<path-below-hn2>#<purpose>#<gran>#<bucket>"
```

- `path-below-hn2`: hierarchy segments below the company, `|`-joined, then the leaf:
  - company (`hn2`) itself → empty → sk begins with `#`
  - `hn3=9` → `"HN3#9"`
  - `hn4=456` under `hn3=9` → `"HN3#9|HN4#456"`
  - leaf meter `10009` under `hn4=456` → `"HN3#9|HN4#456|L#10009"`
  - a meter produces a rollup row at each populated level between `hn2` and its deepest
    non-null `hn` node, **plus** the leaf `L#<logical_id>` row.
- `purpose`: e.g. `"Electricity"`.
- `gran`: `"h"` (hour) | `"d"` (day).
- `bucket`: **UTC** window start — hour `"YYYY-MM-DDThh"`, day `"YYYY-MM-DD"`.

### Delimiter invariant

A node's **own** dated rows append `#<purpose>…` to its path; its **descendants** extend the
path with `|HN…` / `|L#…`. Because `#` (0x23) < `|` (0x7C), all of a node's own rows sort
*before* any descendant row. This is what lets a `BETWEEN` range on
`"<path>#<purpose>#<gran>#"` return exactly the node's own series and never its children.

### Worked example

One electricity reading from meter `10009` (`hn2=2`, `hn3=9`, `hn4=456`) contributes to these
**daily** rows (and the 4 matching **hourly** rows):

```
pk="2"  sk="#Electricity#d#2026-06-07"                       # company hn2=2
pk="2"  sk="HN3#9#Electricity#d#2026-06-07"                  # hn3 node
pk="2"  sk="HN3#9|HN4#456#Electricity#d#2026-06-07"          # hn4 building
pk="2"  sk="HN3#9|HN4#456|L#10009#Electricity#d#2026-06-07"  # the meter (leaf)
```

A node's row is the sum across **all** meters beneath it (siblings collapse in the rollup).

### Attributes

| attr | type | meaning |
|---|---|---|
| `sum` | number | `sum(resample_value)` in the bucket (normalized unit) |
| `count` | number | number of resample points |
| `min`, `max` | number | min/max `resample_value` in the bucket |
| `last_value` | number | `value` (cumulative reading) at `max(timestamp)` in the bucket |
| `last_ts` | string | `max(timestamp)` in the bucket (ISO-8601) |
| `unit` | string | normalized unit for this purpose |
| `purpose` | string | e.g. `"Electricity"` (also in sk) |
| `level` | string | `"2".."9"` (hn level) or `"leaf"` |
| `updated_at` | string | when the job last wrote this item |
| `ttl` | number | epoch seconds: bucket-end + 90d (hourly) / + 730d (daily) |

### TTL / retention

- Hourly buckets: `ttl` ≈ bucket-end **+ 3 months** (90 days).
- Daily buckets: `ttl` ≈ bucket-end **+ 2 years** (730 days).

## Query patterns

All issued from a known node path (the reader always knows where it is):

```
# point: one node, one bucket
GetItem pk="2", sk="HN3#9|HN4#456#Electricity#d#2026-06-07"

# interval: one node's series over a date range (ONE query)
query pk="2",
      sk BETWEEN "HN3#9|HN4#456#Electricity#d#2026-06-01"
             AND "HN3#9|HN4#456#Electricity#d#2026-06-07"

# the 24 hourly buckets of a day
query pk="2",
      sk BETWEEN "HN3#9|HN4#456#Electricity#h#2026-06-07T00"
             AND "HN3#9|HN4#456#Electricity#h#2026-06-07T23"

# ancestor breadcrumb (node + ancestors), one bucket
BatchGetItem [ "#Electricity#d#D", "HN3#9#Electricity#d#D",
               "HN3#9|HN4#456#Electricity#d#D", ... ]
```

Deliberately **not** supported in a single query: "a whole subtree of different nodes,
date-ranged." Not needed — each node already carries its own pre-summed total, so you query the
node, not its children.

## Population (Glue batch)

**Trigger:** EventBridge schedule runs a PySpark Glue job **hourly** (shortly after the hour
closes).

**Each run:**
1. Read a **configurable lookback** of `logical_meter_data`, expressed as a number of trailing
   **whole UTC days** `N` (default `1` ⇒ recompute *today + yesterday* each run). The read window
   is `[00:00 UTC of (today − N), now]`, also filtered by `ingested_time` so late/restated
   readings within it are re-picked-up. `N` is a Glue job argument wired from CDK context.
   **Whole-day alignment is required for correctness:** because each run *overwrites* a daily
   bucket with the sum of every reading in the window that falls in that day, the window must
   cover each recomputed day in full. Closed days in the window are therefore complete; the
   current (still-open) day is overwritten with the correct running partial total.
2. Filter to counters: `resample_method = 'time_proportional' AND resample_value IS NOT NULL`.
3. Derive each row's UTC hour bucket and day bucket from `resample_timestamp` (each input row
   contributes to one hourly group and one daily group).
4. Roll up at every populated level in one pass with Spark `GROUPING SETS` over
   `(hn2), (hn2,hn3), …, (hn2..deepest hn), (…, logical_id)` × `purpose` × `gran` × `bucket`,
   computing `sum/count/min/max` of `resample_value` and `last_value`/`last_ts` from the reading
   at `max(timestamp)`. (A level is emitted only where its node id is non-null.)
5. Build the `pk`/`sk`/`ttl` for each group and **upsert** (`PutItem` overwrite → idempotent;
   recomputing a bucket restates it, never double-counts). Batched writes with retry/backoff.

**Idempotency / late data:** re-running any window overwrites the same items, so a failed run
self-heals on the next run (the window re-covers the gap), and late/restated readings whose
`ingested_time` lands in the last `N` days are absorbed automatically. The trade-off is write
amplification — each run rewrites the last `N` days' buckets even when unchanged; `N` is the
freshness-vs-cost dial (small `N` = cheaper, less late-data reach). Restatements **older** than
`N` days (the existing late-recomputation path can touch arbitrarily old data) are a documented
**hook** — a wider/targeted recompute — not built in this iteration.

## Infra

A new CDK stack in `infra/daq/data_pipeline` (Go CDK, matching the existing stacks), account
`891`:

- DynamoDB table `measurements_aggregate` (on-demand, TTL on `ttl`).
- Glue PySpark job reading `logical_meter_data` via the catalog and writing the table; lookback
  passed as a job argument from CDK context.
- EventBridge hourly schedule.
- IAM: Glue role read on the Iceberg table + write on the DynamoDB table.

## Error handling & ops

- Job idempotent; partial runs are safe (each written item is individually correct; the rerun
  completes the rest).
- DynamoDB on-demand → no throughput tuning; writes retry with backoff.
- CloudWatch alarm on Glue job failure.

## Testing

- **Pure functions:** UTC hour/day bucket derivation; `sk` construction incl. the `#`-vs-`|`
  delimiter invariant; `ttl` math (3-month / 2-year offsets).
- **Aggregation:** given sample `logical_meter_data` rows, assert `GROUPING SETS` produce correct
  per-level sums/stats, and that a **re-run yields byte-identical items** (idempotency).
- **Query invariant:** assert a node's `BETWEEN` range excludes descendant rows (the `#` < `|`
  ordering holds).

## Scope (YAGNI)

In scope: the `measurements_aggregate` table + the hourly Glue populator + tests.

Out of scope (deferred):
- **Gauges** (incl. energy gauges) — counters only for now; gauges slot in later as another
  value/purpose branch.
- **Non-resampled counters** — only rows with `resample_value` (resampled) are aggregated.
- **Partner (`hn1`) level** — rollups top out at the company (`hn2`), since `pk=hn2` makes each
  company its own partition; partner totals would be cross-partition.
- **Read URL/API** — the thin Lambda/API-GW in `891` mapping `path,purpose,gran,from,to` → the
  query is a follow-up spec; this spec delivers the table + populator it sits on.
- **Restatement older than the lookback window** — a documented hook, not built.
