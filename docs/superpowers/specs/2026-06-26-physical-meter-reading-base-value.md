# Spec/plan — absolute "physical meter reading" via an auto-captured base value

Date: 2026-06-26 · Status: **PARKED — awaiting real requirements before implementation.** Design is
settled; do not build until the user has gathered actual requirements. Supersedes the manual-calibration
draft. One open implementation choice when resumed: capture via Glue (Option A, recommended) vs Flink
(Option B).

## Real-EMS reference (captured 2026-06-26)

The Enity EMS already models this whole domain on the per-meter **Datatilegnelse** page
(`/App/company/997/datasource?maalerId=<id>`; see `frontend/docs/ems-recreation/05-maalere.md §5.2`).
Map our concepts to theirs when requirements are gathered:

- **`Startværdi`** (start value, per counter register) ≈ our `base_value`.
- **`Ans. tællerst.`** (anslået tællerstand = estimated counter reading) = the reconstructed **odometer**
  shown to users — i.e. exactly the "absolute" output this plan produces.
- **`Målerskifte`** (meter change) and **`Tællervending`** (counter rollover) = the events that reset the
  base; **`Justér anslået tællerstand`** = a manual odometer correction. (Per-reading `...` menu.)
- Readings carry `Aflæsning` (raw reading) + `Forbrug` (consumption) per hour, plus multiplier
  (`Gange-faktor`) and conversion (`Omregningsfaktor`) factors and a `DAQ ID` of form
  `logger:box:box:register`.

Implication: the eventual feature should probably surface absolute readings the way EMS does (an odometer
column/series + explicit meter-change & rollover handling), not just a chart toggle. Revisit Decisions
2–4 against these real operations before building.

## Problem

Operators want measurement values that **match the physical meter's odometer reading**, not just
consumption. Today the pipeline stores **consumption deltas** everywhere, never the cumulative reading:

- Flink `ResampleFunction.scala:226-241` — for a `counter`, writes `value = delta`
  (`current.cumulativeValue − prev.cumulativeValue`) and `resample_value` = the time-proportional
  share of that delta. The raw cumulative reading survives only in `raw_data`; it is **dropped** from
  `logical_meter_data` (`Main.scala:289-308`).
- Glue `measurements_aggregate.py` — `SUM(resample_value)` per node/purpose/bucket → `sum` attribute =
  **consumption** in that bucket. (`last_value` is the last *delta*, not an odometer value.)
- Aggregations Lambda `crates/services/aggregations/src/main.rs` — returns `value = sum` (consumption),
  reading `measurements_aggregate` (DynamoDB) directly.

So an absolute reading must be **reconstructed** at query time: keep stored deltas pure, add a per-meter
base on demand behind a Lambda flag.

## Key insight — the base telescopes to the exact odometer

For a counter, the raw reading **is** the cumulative odometer value, and deltas telescope:

```
absolute(t) = base + Σ resample_value over (base_as_of, t]   (one logical meter, one purpose)
            = reading(base_as_of) + ( reading(t) − reading(base_as_of) )
            = reading(t)                                       ← the true device odometer
```

This holds for **any** real `(reading, timestamp)` anchor — it need not be the device's first reading.
Using the **first** reading just maximizes how far back absolute is valid.

## Decisions (locked)

1. **Auto-captured base, no manual calibration.** `base = first raw reading seen for a device`,
   `base_as_of = that reading's timestamp`. The hierarchy never touches it (it can't know an odometer);
   base lives purely in the DAQ data plane.
2. **Set-once per device; reset on daq_id change.** A logical meter keeps the same `logical_id` when its
   device is swapped (`ReplaceSensorDevice`). A new `daq_id` ⇒ re-anchor base to the new device's first
   reading. Absolute is then correct **from the swap forward** (accepted).
3. **Leaf logical meters only, counters only.** Absolute is a per-device concept; parent nodes and
   gauges return consumption only (`absolute = null`).
4. **Computed at query time in the aggregations Lambda**, anchored at `base_as_of` (excludes the prior
   device's deltas and the cross-device discontinuity).

## Storage — a dedicated `meter_base` table (DAQ account 891377204778)

Keyed by **`logical_id`** so the Lambda (which has the logical_id leaf) reads it with a plain GetItem —
no GSI, and no interaction with the bridge's `put_item` (which would otherwise clobber an attribute it
doesn't know about, since the bridge overwrites the whole meter-identity row).

| Attribute | Type | Meaning |
|---|---|---|
| `logical_id` (PK) | N | the logical meter |
| `base_value` | N | first raw reading of the current device |
| `base_as_of` | S | ISO-8601 timestamp of that reading (the sum anchor) |
| `daq_id` | S | which device this base belongs to (drives reset-on-change) |

On-demand billing, `RETAIN`. New CDK stack/resource (e.g. extend `MeasurementsAggregateStack` or a small
`MeterBaseStack`).

**Conditional write (set-once + reset-on-change):**
```
UpdateItem meter_base[logical_id]
  SET base_value=:v, base_as_of=:t, daq_id=:d
  CONDITION attribute_not_exists(daq_id) OR daq_id <> :d
```
Same device every run ⇒ condition false ⇒ no-op. New device ⇒ condition true ⇒ re-anchor.

## Capture mechanism (pick one)

### Option A — Glue batch from `raw_data` (recommended: zero streaming-app risk)

A small hourly Glue job (sibling of `measurements-aggregate`, or a step in it):
1. Read `raw_data` lookback window (`daq_id, timestamp, value`) and `meter-identity` (`daq_id → logical_id`,
   `meter_type`); join on `daq_id`, keep counters.
2. Per `logical_id`, pick the **current** device = `daq_id` with `max(timestamp)`; compute that device's
   **earliest-in-window** reading → `(base_value, base_as_of)`.
3. Conditional `UpdateItem` above.
4. **One-time backfill** for already-running devices: a full-history pass computing
   `min(timestamp)→value per (daq_id)` so existing meters get their *true* first reading once.

Freshness ≈ 1 h, which is fine — absolute reads are not real-time. Reuses the existing Glue/raw_data
pattern; **no change to the Flink app** (avoids the operator-state/snapshot risk called out in CLAUDE.md).

> Note: telescoping is correct even if "earliest-in-window" isn't the true first reading — any real anchor
> works (see insight above). The one-time backfill only serves to push `base_as_of` as far back as
> possible so absolute is valid over more history.

### Option B — Flink new operator (real-time, more invasive)

A new operator **after enrichment, before resample**, keyed by `logical_id`, with a `ValueState[String]`
holding `currentDaqId`. On first record or a `daq_id` change, emit `(logical_id, daq_id, value, ts)` to a
side output whose sink does the conditional `UpdateItem`. It has its **own `uid`** (e.g.
`"meter-base-capture"`), independent of the `"resample"` operator's keyed state, so the resample
snapshot/restore is unaffected (adding a new operator restores cleanly; only renaming existing uids/state
breaks). Exact first-value and exact change detection, minimal writes — at the cost of touching the
critical streaming job and adding a DynamoDB sink (make it async/best-effort to avoid backpressure).

**Recommend A** unless sub-hour base freshness is required.

## Aggregations Lambda (account 891377204778)

`crates/services/aggregations/src/main.rs`:

1. **New query param** `absolute` (`true|false`, default `false`).
2. **Leaf guard.** Resolve `logical_id` from the trailing segment of `level_id`. If not a single leaf
   meter ⇒ ignore the flag, `absolute = null`.
3. **Base fetch.** `GetItem meter_base[logical_id]` → `base_value, base_as_of, daq_id`. Missing, or the
   meter is a gauge ⇒ `absolute = null`, consumption unchanged.
4. **Prefix sum.** One extra query `(base_as_of, window_start)` on this leaf+purpose+gran, summing `sum`.
   Bounded by `base_as_of` and by TTL (hourly ≤ 90 d, daily ≤ 730 d).
5. **Compute.** Running cumulative across the window; per bucket `b ≥ base_as_of`
   `absolute = base_value + prefix + Σ(sum up to & incl. b)`; for `b ≤ base_as_of` ⇒ `null`. Use a strict
   `> base_as_of` boundary so the cross-device anomaly delta is excluded.
6. **Response.** Add `absolute: Option<f64>` to `Row`; keep `value` = consumption unchanged
   (purely additive, backward-compatible). Frontend draws the `absolute` series when present.
7. **Tests.** Prefix+cumulate math, leaf detection, base-missing / gauge fallbacks, the `base_as_of`
   boundary and a simulated daq-change.

## Frontend (later, small)

A per-meter chart toggle "Vis som målerstand (absolut)" that flips `absolute=true` and renders the
returned `absolute` series (ECharts line). HTML/CSS + inert `data-endpoint` now; wire when the
meter-detail view is backend-integrated.

## Deployment order

1. **`meter_base` table** (DAQ) — new resource; non-destructive.
2. **Capture** (Option A Glue job + one-time backfill, or Option B Flink operator) — populates base.
   Verify a few rows: `base_value`/`base_as_of`/`daq_id` look right; swap a test sensor's `daq_id` and
   confirm re-anchor.
3. **Aggregations Lambda** — `cdk deploy MeasurementsAggregateStack`; test `absolute=true` on a known
   counter leaf and check `absolute ≈ device reading`.

No hierarchy-account change. No `logical_meter_data` / Iceberg schema change. No Flink change if Option A.

## Caveats / future

- **Two distinct "from that time on" limits:** (a) after a daq swap, absolute is valid only forward from
  the new device's first reading (inherent); (b) for a device **older than the aggregate's TTL retention**,
  the sum from `base_as_of` is incomplete and absolute drifts — fix later with a non-TTL'd running
  cumulative checkpoint (per device) so the Lambda sums only from a recent checkpoint, not from install.
- **Late recomputation** (`late_recomputation.py`) rewrites `resample_value`; absolute is derived at query
  time, so it self-corrects once aggregates are recomputed. No extra work.
- **Multi-purpose meters:** the sum is purpose-filtered, so calibrate/return per purpose; revisit when
  wiring the frontend.
- **Orphan meter-identity rows** on daq swap (bridge `put_item` at the new `sk`, old row lingers) are a
  pre-existing concern and don't affect `meter_base` (keyed by `logical_id`, reset by the `daq_id <>`
  condition).

Related: `infra/daq/data_pipeline/docs/superpowers/specs/2026-06-07-measurements-rollup-view-design.md`,
`memory/cross_account_bridge.md`, `crates/services/aggregations/src/main.rs`,
`flink_app_scala/.../Main.scala` (raw_data schema, enrichment), `glue/measurements_aggregate.py`.
