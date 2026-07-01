# Counter-measurement migration — working notes

> **Status:** working notes (not the final report). Last updated 2026-07-01.
> **Goal:** consolidate everything that uses *counter measurements* in the `emswt/main` distributed monolith into a single CQRS read/aggregation service in `ems_rust` (mirroring the existing `aggregations` service), serving the **frontend (primary)** and some reporting. Track which request/response patterns exist today and which are already covered by the new pipeline.

## 1. Context & mapping

| emswt/main (old) | ems_rust (target) |
|---|---|
| `sensor_measurements` DB (raw) | S3 Iceberg `raw_data` |
| `counter_measurements` DB (logical) | S3 Iceberg `logical_meter_data` |
| `analysis_service` (reads `counter_measurements` via `@enity/counter-query`) | `aggregations` Rust lambda (CQRS read, `crates/services/aggregations`) |
| in-app + per-request compute | Spark/Glue rollup → DynamoDB `measurements_aggregate` + request-time compute |

**Key migration seam:** `@enity/counter-query` has only **2 importers** (`analysis_service`, `counter-ingestion`). `counter-ingestion` is the raw→logical transform (already has an Athena/S3 client).

The old read side is `analysis_service`. All its endpoints are **POST query-by-body** (already CQRS-read-shaped). Canonical API defs: `emswt/main/packages/analysis-client/src/api-endpoints/`. It turns cumulative counter readings into consumption **in-app** (`RunningTotal`/`RunningArea` — time-weighted), then runs a 3-phase `prep → data → compute` pipeline.

## 2. Endpoint request/response patterns (analysis_service)

Classification: **pass-through** = straight DB read (rename/omit columns) · **processing** = aggregation/computation.

| # | Endpoint (POST) | Request (key inputs) | Response | Class | Notes |
|---|---|---|---|---|---|
| 1 | `api/counters/reading-count` | `counterIds`, `intervals[]` | per-counter reading counts | pass-through | DB `COUNT` per interval |
| 2 | `api/counters/reading-bounds` | `counterIds` | per-counter `{first,last}` | pass-through | DB `MIN/MAX` reading ts |
| 3 | `api/counters/latest-reading` | `counterIds` | per-counter `{latest}` | pass-through | derived from #2 (`.last`) |
| 4 | `api/meter-data/filter-query` | `meterFilter` (→meter_service), `interval`, `resolution?` | per-meter output series (`intervals[]`) | processing | resolve meters → fetch counters → in-app delta + time-weighted rollup → expression handlers |
| 5 | `api/meter-data/details-query` | `meterDetails` (pre-resolved), `interval`, `resolution?` | = #4 response | processing | #4 without meter resolution |
| 6 | `api/meter-data/filter-groupby` | `meterFilter`, `groupBy[]`, `requests[]` | nested `groups[]` (values/intervals/aggregations) | processing | rollup + hierarchy group-aggregation |
| 7 | `api/meter-data/details-groupby` | `meterDetails`, `groupBy[]` | = #6 response | processing | #6 without meter resolution |
| 8 | `api/meter-data/aggregate` | `meters?`, `aggregations: Record<key, {interval,resolution?,…}>` | `aggregations: Record<key, {interval,resolution?,…}>`, `time?` | processing | rollup + sum/avg/min/max over interval |
| 9 | `api/meter-data/values` | `meters`, `requests: [{interval,resolution?}]` | per-meter interval values | processing (richest) | full compute pipeline — see §4 |
| 10 | `api/key-values/sum` | `keys` | sum | processing — **likely out of domain** | uses `auxDataLayer`, not `counter_measurements`; confirm & exclude |
| 11 | `legacy/consumption/query` | legacy consumption body | consumption series | processing (legacy) | older consumption path |
| 12 | `legacy/zoom/query` | legacy zoom body | zoomed series | processing (legacy) | time-resolution drill-down |

Totals: **3 pass-through**, **8 processing** (+1 aux/out-of-domain to confirm).

## 3. Already implemented in ems_rust

**Spark/Glue rollup** — `infra/daq/data_pipeline/glue/measurements_aggregate.py` (`MeasurementsAggregateStack`, daq account `891377204778`, hourly, `LookbackDays` default 1). Design spec: `infra/daq/data_pipeline/docs/superpowers/specs/2026-06-07-measurements-rollup-view-design.md`.

- **Source:** `all.logical_meter_data` (event-sourced/append-only; `resample_value` where `resample_method='time_proportional'`, dedup newest `ingested_time` per `(logical_id, resample_timestamp)`).
- **Sink:** DynamoDB `measurements_aggregate` (TTL 90d hourly / 730d daily; GSI by dimension `energy|volume|other`).
- **Granularities:** **hour + day only.**
- **Per `(node, purpose/resource, gran, bucket)`:** `sum, count, min, max, last_value, last_ts`.
- **Hierarchy pre-aggregation:** emits a row at **every ancestor level** (company `HN2#` → … → leaf meter `L#`) via `ancestor_keys` → **group-by-hierarchy is already materialized**.

**Read API** — Rust CQRS lambda `measurements-aggregations-api` (`crates/services/aggregations`), API Gateway HTTP API, `GET /meterdata/query/{action}`:
- `get_aggregations` → reads DynamoDB `measurements_aggregate` (Resource-Insights chart).
- `get_measurements` → reads `all.raw_data` via **Athena** (Datatilegnelse; dedup `GROUP BY` + `max_by(value, ingested_time)`).
- `+ /meterdata/openapi.json`, `/meterdata/docs`; `?format=html|json`.

## 4. Coverage overlay — old endpoints vs new rollup

| # | Endpoint | Covered by existing rollup? | Gap |
|---|---|---|---|
| 1 | reading-count | ✅ `count` | — |
| 2 | reading-bounds | 🟡 `last_ts`=last; **first** not materialized | need a first-bound query |
| 3 | latest-reading | ✅ `last_value`/`last_ts` | — |
| 4/5 | filter/details-query | ✅ at hour/day (`sum` series) | off-grid resolutions (§5) |
| 6/7 | filter/details-groupby | ✅ **strong** — hierarchy pre-agg, hour/day | off-grid resolutions only |
| 8 | aggregate | ✅ `sum/min/max`; **avg = sum/count** derivable | arbitrary interval boundaries compose from buckets (month/year = Σ days ✅; partial buckets 🟡) |
| 9 | values | 🟡 base energy `sum` only | **compute pipeline not in rollup** — §4a |
| 11/12 | legacy consumption/zoom | ✅ hour/day equivalent | finer zoom levels |
| 10 | key-values/sum | out of counter domain | confirm & exclude |

**Takeaway:** the existing rollup already collapses #1–#3 and the hour/day base of #4–#8/#11–#12 (including hierarchy grouping) into DynamoDB point-reads. Remaining real work = the `values` compute layer (§4a) + off-grid resolutions (§5).

### 4a. `values` (#9) compute handlers — the request-time layer

3-phase dispatcher (`analysis_service/src/lib/handlers/dispatcher.ts`, `prepare/` modules). Handlers: energy, degree-days, normal-degree-days, climate-correct, GUF, CO2 / emission-scope, energy-budget (+ dda), active-hours / operational-hours, reading-count.

- **Movable to rollup / precompute** (candidate): base energy `sum` (done); degree-day / weather series could be a precomputed series joined at read time.
- **Irreducibly request-time** (parameterized/tenant/contextual): energy-budget (per-user budget), active/operational-hours (threshold param), normal-degree-days & climate-correct (climate-normal + weather), CO2/emission-scope (contextual scope), group-by (live meter hierarchy).

> **TODO (next deep-dive):** split each §4a handler into "move to Flink/Spark rollup" vs "stays request-time" — the last decision before the aggregation service scope is fixed.

## 5. Resolution strategy (decisions)

- **hour + day:** materialized in DynamoDB (done).
- **week / month (and year):** **compute in-service from the daily rollup — do NOT materialize.** A 2-year daily series is only 730 datapoints; conflating to weekly (~÷7) or monthly (~÷12) in service memory is cheap. Not worth the DynamoDB write/storage cost.
- **15-minute (sub-hour):** **deferred / open** — purely a DynamoDB storage-cost decision, gated on whether the frontend actually needs sub-hour. Revisit only if a real need appears.

## 6. Adjacent context: `meter_service` (identity plane) — not part of the aggregation service

`meter_service` is a **different bounded context** from counter measurements — the meter/asset **identity, hierarchy & metadata registry**, orthogonal to the measurement time-series. (Correction to earlier notes: it does **not** read `sensor_measurements` or any measurement DB — a full-tree grep finds zero references.)

- **Data sources:** `me2db` (legacy ME2 monolith DB) + its own `meter` DB. No measurement DB.
- **Owns (CRUD master data, not a read model):** meter / sensor / physical-counter / physical-meter / counter, building / address / hierarchy-elements, tags, units / reading-types, energy-form/class/main-group, operational-hours, documents, custom fields, `sensor-data-ownership`, meter-sorting. Emits change events via `kafka_rest_url`. (`meterRouter` alone: 17 GET / 18 POST / 7 PUT / 4 DELETE.)
- **Consumers:** 15 services via `@enity/meter-client` (most-consumed service in the system): alarm-management, alarm_runner, analysis_service, climate_reporting_service, consumption-api, ems-backend/Web, energy-cost, energy-model-service, energy-model-v2, export, import-export, missing-manual-readings, ok-carwash-api, report-runner, yggdrasil.
- **Role in the measurement path:** resolve a `meterFilter` → concrete meters + hierarchy + unit, which `analysis_service` then uses to query `counter_measurements`. The "which meters" resolver behind the "what values" queries.

**Target mapping:** → `ems_rust` **hierarchy service** (`rust-lambda-hierarchy` + `hierarchy_new`) + the **`meter-identity`** table. Not the aggregations service.

**Direction — the measurement path should read from `meter-identity`, not `meter_service`.** The identity/hierarchy the read path needs is *already denormalized into the data*: `hn2..hn9`, `logical_id`, `purpose`, `unit` are stamped into `logical_meter_data` (and thus `measurements_aggregate`) by the Flink enrichment step, sourced from `meter-identity`. So the old request-time "call meter_service to resolve filter + hierarchy" is pushed **upstream into ingestion/enrichment**; the aggregation read side just ranges on precomputed hierarchy keys and never calls a meter service at query time.

**Adding metadata to `meter-identity` — yes, this makes sense, with two guardrails:**
1. **Keep it a lean read projection, not a second registry.** Add only what the measurement read/enrichment path needs — candidates beyond today's identity/hierarchy/purpose/unit: `reading-type`, `operational-hours` (for §4a active-hours), possibly `tag`s for filtering. The rest of `meter_service` (documents, custom fields, building CRUD, ownership, sorting) stays in the hierarchy/registry service as the **write model / source of truth**; `meter-identity` is a CQRS **read projection** fed by its change events.
2. **Decide per field: stamped-at-write vs looked-up-at-read.** Hierarchy path is stamped point-in-time today (historical rollups keep the hierarchy as-of ingestion). Metadata used for *current* filtering (tags, current hierarchy) is "as-of-now" and must be resolved at read time — baking it into history means a re-tag or hierarchy move silently rewrites the past. Classify each added field's temporal semantics deliberately.

Net: sound direction — move the measurement path's identity/metadata reads onto `meter-identity` and enrich it as needed — **provided** `meter-identity` stays a curated projection of the registry and each field's point-in-time-vs-current semantics is chosen on purpose.

## 7. Raw side: `sensor_measurements` / `raw_data` readers

The main analytics/frontend path does **not** read raw — `analysis_service` and `consumption-api` don't hold the `sensor_measurements` secret; the whole consumption/analysis path is on `counter_measurements` (logical). Consumer read of raw = the **datatilegnelse** view only → served by `get_measurements` over `raw_data` via **Athena** in ems_rust.

**Direct DB readers (all pipeline/quality — → Flink/Glue in ems_rust):**
- `counter-ingestion` — raw → logical transform (→ Flink resample/enrichment)
- `sensor-measurements-stat-manager` — stats over raw (→ Flink/Glue)
- `sensor-measurements-missing-readings-manager` — gap detection (→ late-recomputation / monitoring)
- `sensor-measurements-management` — owns + serves raw via HTTP (→ `raw_data` + Athena serve)
- `sensor-measurements-ingestion` — writes raw (→ Flink/Kinesis ingest)

**HTTP consumers of the raw-serve API (`sensor-measurements-management`):**
- `sensor-measurements-stat-manager` — internal.
- ⚠️ **`ok-carwash-api` — MARKED FOR UPDATE (migration).** The one *consumer* reading raw outside the datatilegnelse view: `fetchAdjustedReadings(...)` pulls raw readings directly to compute per-car-wash consumption. **Migration action:** repoint onto the ems_rust `raw_data` path (`get_measurements` via Athena), or rework onto the logical / aggregations path. Only non-pipeline external raw reader.

## 8. Open questions / next steps

1. **Dissect §4a handlers** → movable vs request-time (the key remaining decision).
2. **Confirm frontend call pattern:** does the frontend (via `yggdrasil` / `consumption-api`) actually hit `values` (#9), or mostly `aggregate` (#8) / `groupby` (#6/7)? Decides how much of §4a is on the critical path.
3. **Confirm `key-values/sum` (#10)** is aux data → exclude from counter domain.
4. **First-reading bound (#2)** — decide whether to add to the rollup or query on demand.
5. **`meter-identity` enrichment (§6)** — enumerate which `meter_service` fields the measurement read path actually needs in `meter-identity` (reading-type, operational-hours, tags?), and for each decide stamped-at-write (point-in-time) vs looked-up-at-read (current). Confirm the registry/hierarchy service remains the write-model source of truth.
6. **`ok-carwash-api` migration (§7) — MARKED FOR UPDATE** — repoint its direct raw-reading (`fetchAdjustedReadings` via `sensor-measurements-management`) onto the ems_rust `raw_data` / Athena `get_measurements` path, or rework onto the logical / aggregations path. The only non-pipeline consumer of raw `sensor_measurements`.
