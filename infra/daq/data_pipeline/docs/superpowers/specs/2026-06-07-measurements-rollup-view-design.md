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
pk = "HN2#<hn2>"                               # company node, hierarchy-segment form. Partition = one company.
sk = "<full hierarchy path from HN2>#<purpose>#<gran>#<bucket>"
```

- the **full hierarchy path** is `|`-joined hierarchy segments starting at the company `HN2#<id>`
  (consistent with the rest of the hierarchy — no bare `#` start), then deeper nodes, then the leaf:
  - company (`hn2`) itself → `"HN2#<id>"`
  - `hn3=9` → `"HN2#<id>|HN3#9"`
  - `hn4=456` under `hn3=9` → `"HN2#<id>|HN3#9|HN4#456"`
  - leaf meter `10009` under `hn4=456` → `"HN2#<id>|HN3#9|HN4#456|L#10009"`
  - a meter produces a rollup row at each populated level between `hn2` and its deepest
    non-null `hn` node, **plus** the leaf `L#<logical_id>` row.
- `purpose`: e.g. `"Electricity"`.
- `gran`: `"h"` (hour) | `"d"` (day) — lives in the sk only, not stored as a separate attribute.
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
pk="HN2#2"  sk="HN2#2#Electricity#d#2026-06-07"                       # company hn2=2
pk="HN2#2"  sk="HN2#2|HN3#9#Electricity#d#2026-06-07"                 # hn3 node
pk="HN2#2"  sk="HN2#2|HN3#9|HN4#456#Electricity#d#2026-06-07"         # hn4 building
pk="HN2#2"  sk="HN2#2|HN3#9|HN4#456|L#10009#Electricity#d#2026-06-07" # the meter (leaf)
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
| `purpose` | string | e.g. `"Electricity"` (also encoded in the sk; kept for convenience) |
| `bucket` | string | the bucket label (also in the sk; kept for convenience) |
| `updated_at` | string | when the job last wrote this item |
| `ttl` | number | epoch seconds: bucket-end + 90d (hourly) / + 730d (daily) |

(`level` and `gran` are deliberately **not** stored — `gran` is in the sk and `level` is derivable
from the path. `unit` is **not** carried yet; it isn't redundant, so it's a candidate to add to the
read if consumers need it to interpret `sum`.)

### TTL / retention

- Hourly buckets: `ttl` ≈ bucket-end **+ 3 months** (90 days).
- Daily buckets: `ttl` ≈ bucket-end **+ 2 years** (730 days).

## Query patterns

All issued from a known node path (the reader always knows where it is):

```
# point: one node, one bucket
GetItem pk="HN2#2", sk="HN2#2|HN3#9|HN4#456#Electricity#d#2026-06-07"

# interval: one node's series over a date range (ONE query)
query pk="HN2#2",
      sk BETWEEN "HN2#2|HN3#9|HN4#456#Electricity#d#2026-06-01"
             AND "HN2#2|HN3#9|HN4#456#Electricity#d#2026-06-07"

# the 24 hourly buckets of a day
query pk="HN2#2",
      sk BETWEEN "HN2#2|HN3#9|HN4#456#Electricity#h#2026-06-07T00"
             AND "HN2#2|HN3#9|HN4#456#Electricity#h#2026-06-07T23"

# ancestor breadcrumb (node + ancestors), one bucket
BatchGetItem [ "HN2#2#Electricity#d#D", "HN2#2|HN3#9#Electricity#d#D",
               "HN2#2|HN3#9|HN4#456#Electricity#d#D", ... ]
```

Deliberately **not** supported in a single query: "a whole subtree of different nodes,
date-ranged." Not needed — each node already carries its own pre-summed total, so you query the
node, not its children.

## Population (Glue batch)

**Trigger:** EventBridge schedule runs a PySpark Glue job **hourly** (shortly after the hour
closes).

**Each run:**
1. Read a **configurable lookback** of `logical_meter_data`, expressed as a number of trailing
   **whole UTC days** `N` (default `1` ⇒ recompute *today + yesterday* each run), filtering by
   **`resample_timestamp >= 00:00 UTC of (today − N)`** (the bucket axis). `N` is a Glue job
   argument wired from CDK context. **Whole-day alignment** means each recomputed daily bucket is
   summed from all its points — closed days complete, the current day a correct running partial.
   Restatements of points whose `resample_timestamp` is older than the window aren't picked up;
   widen `N` to recompute them (the documented hook).
2. **Take the newest `ingested_time` per `(logical_id, resample_timestamp)`** — `logical_meter_data`
   is event-sourced (append-only; restatements are appended), so superseded rows must be dropped
   before aggregating, matching every other consumer. Then keep **counters** only
   (`resample_method = 'time_proportional'`, non-null `resample_value`, non-null `hn2`).
3. Derive each row's UTC hour bucket and day bucket from `resample_timestamp` (each row contributes
   to one hourly and one daily group).
4. **Explode each row into its ancestor node keys** (company `HN2#<id>` → … → leaf `L#<id>`) and
   group by `(node_path, purpose, gran, bucket)`, computing `sum/count/min/max` of `resample_value`
   and `last_value`/`last_ts` from the reading at `max(timestamp)`. (Ancestor-explode emits a row
   only for populated levels — equivalent to per-level `GROUPING SETS` without null groups.)
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
- **Aggregation:** given sample rows, assert the ancestor-explode rollup produces correct per-level
  sums/stats, that `latest_counters` keeps the **newest `ingested_time`** per point (and drops
  gauges / null-`hn2`), and that a re-run yields identical items (idempotency).
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
