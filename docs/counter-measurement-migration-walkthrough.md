# Counter-measurement migration — a walkthrough

*A document to read through in a meeting. Detail and evidence are in [`counter-measurement-migration.md`](./counter-measurement-migration.md); section refs (§) point there. The "In short" line under each part is the one to lift to a slide.*

---

## 1. Where we are today

Around 60 services call each other without clear bounded contexts. For measurement data, two services sit at the centre: **`analysis_service`** (reads the `counter_measurements` DB and does the consumption calculations) and **`meter_service`** (the meter/hierarchy identity registry). About 15 services call these two, and the frontend reaches them through BFFs (`yggdrasil`) and the legacy .NET monolith (`ems-backend`).

The user-facing meter model is also hard to follow: main meters, sub meters, calculation meters, and a "part of summation" flag with query-time summation contexts — four concepts a user has to combine correctly to avoid double-counting.

**In short:** services are tightly coupled around measurements, and the meter model is hard to follow.

---

## 2. Where we want to be

The direction already taken in `ems_rust`: CQRS read services on top of a data pipeline. The frontend (and `yggdrasil` for a transition period) calls one or two common services — `aggregations` for measurement data, `hierarchy` for identity — instead of the many services it goes through today.

The principle: do the work once, in the pipeline, and store the result so reads are simple. The read service stays thin.

**In short:** the frontend reads from one service; the pipeline does the computation.

---

## 3. What already exists

- A **Spark/Glue rollup** turns `logical_meter_data` into a DynamoDB view (`measurements_aggregate`) — hour and day periods, pre-aggregated at every hierarchy level (company → building → meter). Group-by is already done (§3).
- A **Rust CQRS read API** (`aggregations`) serves it, plus raw-data reads via Athena for the datatilegnelse view.
- A **hierarchy service** and a **`meter-identity`** table own identity/hierarchy; a cross-account CDC bridge already keeps them fed.

**In short:** the rollup, the read API and the identity plane already exist; what is missing is the compute layer and getting all data in.

---

## 4. Measurements in: ingestion

The new pipeline ingests through a **Kinesis** stream. Sources still arriving on the legacy **eventlogger** Kafka topics are routed into it via **EventBridge Pipes**, the way **me2-events** already is. The remaining work (Track I):

| Source | Plan |
|---|---|
| **Electrocom** | dedicated parser — the most involved one, but python/ts reference code exists to port |
| **Catch-all MQTT** | route through **AWS IoT Core** (native path), not a bespoke parser |
| **Brunata · Datahub · Aalborg Forsyning · Danfoss · Techem** | move to the **multi-tenant API** (API ingestion, no parser) |
| **CSV** | the new **`csv_parser`** |
| **Manual meter updates** | come **through the pipe** |
| **Kinect** | dead — decommissioned, nothing to migrate |

**In short:** each source is routed into the pipeline; the legacy ingestion services can then be removed.

---

## 5. Measurements out: the compute model, in three lanes

The old `values` endpoint runs about 35 compute **handlers** per request — a handler is the unit of computation for one derived metric (energy, cost, CO2, degree-days, …), and they compose (cost = energy × price). We place them by where the work belongs (§5):

- **Lane A — Spark rollup:** aggregates of the readings. Energy, flow and temperatures each come from their own sensor; cooling and VWAT are derived (flow × temperature difference) and depend on the formula step. Stored as columns.
- **Lane B — Flink stream:** metrics that need finer-than-period resolution (active-hours, standby), pre-aggregated to daily columns in the stream.
- **Lane C — denormalised reference data:** cost, CO2, degree-days/climate-correction. We own weather (Weatherbit HDD/CDD), CO2e factors and prices as small reference tables and broadcast-join them once in Spark — weather on (location, date) using the node's location, factors on (resource, period) — writing the results as columns on the aggregate row.

Reads stay fast because nothing is joined at query time — joins from S3/Athena take seconds. The joins happen once in the pipeline, so a read is a single DynamoDB lookup. Weather that finalises late is handled by the rollup's existing look-back window (≥5 days), without extra machinery.

**In short:** Spark for aggregates, Flink for fine resolution, denormalised columns for weather/cost/CO2; the read does no joins.

---

## 6. The meter model, simplified

This is the main model change. In the target there is no "meter" entity — only hierarchy nodes and sensors, where a sensor carries a formula. One mechanism replaces the four old concepts:

- **sub-meter** → a sensor with a part-of-main reference (the system generates the netting `main = self − Σ subs`);
- **main meter** → the netting parent;
- **calculation meter** → a formula expression (and the old calc-meters are already stored as formulas);
- **"part of summation"** → whether a sensor is included in the netting.

We do not build a second hierarchy. Sub/main can cross buildings, so the relationship is a per-sensor reference, not a parallel tree, and the summation flags and query-time contexts are no longer needed. Users declare relationships by picking from the company tree; the formulas are generated underneath and are not hand-written for the common case (§4d, §11).

**In short:** the four meter types become one — a sensor with a formula — with double-counting handled automatically.

One gap: the formula engine exists and is tested, but nothing evaluates it against the time-series yet. Wiring that into the pipeline is the main piece still to build.

---

## 7. Consolidating the consumers — a two-step move

We classified every analysis/meter consumer (§9). It happens in two steps, and the distinction matters:

**Step 1 — repoint (migration).** Each service stops reaching into `analysis_service` / `meter_service` and reads the clean common API instead. This breaks the worst coupling — but the services still exist, so on its own it only moves the coupling one layer down.

**Step 2 — fold in or re-home (destination).** Each service's functionality then either:

- **folds into** `aggregations` / `hierarchy` when it is a measurement/identity concern (e.g. `consumption-api`, `computed-benchmark`);
- **re-homes** as a clean bounded-context service when it is a genuinely separate domain — alarms, reporting/CSRD, ML — owning its own data and loosely coupled (events/APIs), called by the frontend as a peer, not as a middle-tier pass-through;
- or **retires** (legacy monolith, manual-readings, back-office).

`energy-cost` is its own case: it owns the price/factor data, which we plan to own, so it becomes a reference-data owner and later retires. Their direct `me2db` reads are hierarchy and meter identity, which we already have — repoint targets, not blockers. Users are owned by the hierarchy service; a `me2-events` CDC sync keeps everything current during the switchover.

**In short:** repoint first to break the coupling, then fold in or re-home — stopping at repoint would just relocate the hard coupling.

---

## 8. How we validate before building

Two comparison tests ("diff harnesses") are the go/no-go gates (§10), both comparing the new model to the current `analysis_service` output on real data:

1. **Meter-hierarchy test** — derive formulas from the old DB, recompute totals, compare against current summation per company.
2. **Reference-data test** — old `values` (cost/CO2/degree-days) against the new denormalised columns.

The hierarchy test runs offline (no pipeline changes needed) and validates the main assumption.

**In short:** we replay the old results and compare; no cutover until the numbers match.

---

## 9. The path ahead

Stepwise, validate before building, two build tracks in parallel, cutover last (§12):

- **Phase 0 — Verify & size** (now, read-only): run both tests, size the data, scope the critical path.
- **Track I — Ingestion:** parsers / Pipes / IoT Core so all sources are in the pipeline (§4).
- **Phase 1 — Extend the data** (additive): weather/cost/CO2 and aggregate columns; rename to `logical_sensor_data`.
- **Phase 2 — Formula & hierarchy:** wire formula evaluation into the pipeline; migrate the meter hierarchy, validated company by company (offline).
- **Phase 3 — Consolidate & cut over:** extend `aggregations`; subsume / repoint / re-home consumers; cut reads over to the new services. The pipeline and services are multi-tenant, so cutover is a **shared switch** once validation passes — staged by capability/endpoint, not per company. Retire the legacy endpoints.

Recommended first step: the meter-hierarchy test — it validates the main assumption, runs offline, and comes before the largest piece of work.

**In short:** validate first (company by company), extend the data and wire formulas in parallel, then cut over as a shared switch once the numbers match.

---

## 10. Open decisions

- **User-facing meter model** — auto-netting and one unified concept (recommended) or exposing formulas.
- **Data-volume sizing** — confirm the denormalised DynamoDB item is affordable before schema-freeze.
- **Own prices** — when to retire `energy-cost`.
- **Mechanical vs dual-run** for the hierarchy migration (validated company by company), decided by the test.
- **Cutover model** — a shared switch, staged by capability (default), or per-tenant (needs dual ingestion + per-tenant frontend routing).

**In short:** a few decisions remain; the analysis has settled the rest.
