# From distributed monolith to a lean measurement platform — a walkthrough

*A narrative to read through in a room. Detail and evidence live in [`counter-measurement-migration.md`](./counter-measurement-migration.md); section refs (§) point there. Headlines (the **bold** lead of each part) are the slide-worthy lines.*

---

## 1. The problem: a distributed monolith around measurements

Today ~60 services call each other without clear bounded contexts. For measurement data specifically, two services sit at the centre — **`analysis_service`** (reads the `counter_measurements` DB and does all the consumption maths) and **`meter_service`** (the meter/hierarchy identity registry). **~15 services fan out to these two**, and the frontend reaches them through BFFs (`yggdrasil`) and the legacy `.NET` monolith (`ems-backend`).

On top of that, the **user-facing meter model is confusing**: main meters, sub meters, calculation meters, and a "part of summation" flag with query-time summation contexts — four concepts users must juggle to avoid double-counting.

**Headline:** *the measurement domain is tangled at both ends — service-to-service coupling below, a confusing meter model above.*

---

## 2. The target: read from one place, compute in the data

We want the shape already emerging in `ems_rust`: **CQRS read services on top of a data pipeline**. The frontend (and `yggdrasil` temporarily) calls **one or two common services** — `aggregations` for measurement data, `hierarchy` for identity — instead of the myriad services.

The guiding principle: **do the heavy work once, in the pipeline, and denormalise the result so reads are single-key and fast.** The read service stays thin.

**Headline:** *one read surface for the frontend; the pipeline owns the computation.*

---

## 3. What already exists (we're not starting from zero)

- A **Spark/Glue rollup** turns `logical_meter_data` into a DynamoDB materialised view (`measurements_aggregate`) — **hour + day periods, pre-aggregated at every hierarchy level** (company → building → meter). Group-by is already done (§3).
- A **Rust CQRS read API** (`aggregations`) serves it, plus raw-data reads via Athena for the "datatilegnelse" view.
- A **hierarchy service** + **`meter-identity`** table own identity/hierarchy; a **cross-account CDC bridge** already keeps them fed.

**Headline:** *the rollup, the read API, and the identity plane already exist — the gap is the compute layer and getting all data in.*

---

## 4. Measurements *in*: finish the ingestion story

The new pipeline ingests through a **Kinesis** stream. Everything still arriving on the legacy **eventlogger** Kafka topics must be redirected into it via **EventBridge Pipes** — exactly as **me2-events** already does. The remaining parser/route work (Track I):

| Source | Plan |
|---|---|
| **Electrocom** | dedicated parser — the hardest, but python/ts reference code exists to port |
| **Catch-all MQTT** | reroute through **AWS IoT Core** (native path), not a bespoke parser |
| **Brunata · Datahub · Aalborg Forsyning · Danfoss · Techem** | move to the **multi-tenant API** (API ingestion, no parser) |
| **CSV** | the new **`csv_parser`** |
| **Manual meter updates** | come **through the pipe** |
| **Kinect** | dead — decommissioned, nothing to migrate |

**Headline:** *every source lands in the new pipeline via Pipes/parsers/IoT Core — then the legacy ingestion services retire.*

---

## 5. Measurements *out*: the compute model, in three lanes

The old `values` endpoint runs ~35 compute **handlers** per request — a *handler* is the unit of computation for one derived metric (energy, cost, CO2, degree-days, …), and they compose (cost = energy × price). We split them by *where the work belongs* (§5):

- **Lane A — Spark rollup:** deterministic aggregates of the readings (energy, flow, temperatures, cooling). Materialised columns.
- **Lane B — Flink stream:** things that need finer-than-period (sub-hour) resolution (active-hours, standby). Pre-aggregated to daily columns in the stream.
- **Lane C — denormalised reference data:** cost, CO2, degree-days/climate-correction. We **own** weather (Weatherbit HDD/CDD), CO2e factors, and prices as *tiny* reference tables, **broadcast-join them once in Spark**, and write the results as **columns** on the aggregate row.

The rule that makes reads fast: **never join at read time** (S3/Athena joins are seconds-slow). Join once in the pipeline; the read is a single DynamoDB key. Weather that finalises late is handled by the rollup's existing look-back window (≥5 days) — **no bespoke machinery**.

**Headline:** *Spark for aggregates, Flink for fine-resolution, denormalised columns for weather/cost/CO2 — the read never joins.*

---

## 6. The meter model, radically simplified

The big conceptual win. In the target there is **no "meter" entity** — just **hierarchy nodes** and **sensors**, where a sensor carries a **formula**. That single mechanism replaces all four old concepts:

- **sub-meter** → a sensor with a *part-of-main* reference (the system auto-generates the netting `main = self − Σ subs`);
- **main meter** → the netting parent;
- **calculation meter** → a formula expression (and the old calc-meters are *already* stored as formulas);
- **"part of summation"** → just whether a sensor is included in the netting.

Crucially we **do not build a second hierarchy**. Sub/main can cross buildings, so the relationship is a lightweight **per-sensor reference**, not a parallel tree — and all the confusing machinery (summation flags, query-time contexts) disappears. Users declare relationships by picking from the company tree; **formulas are generated underneath, never hand-typed for the common case** (§4d, §11).

**Headline:** *four confusing meter types collapse into one idea — a sensor with a formula — with double-counting handled automatically.*

*(One honest gap: the formula engine exists and is tested, but nothing evaluates it against the time-series yet — wiring that into the pipeline is the main new build.)*

---

## 7. Who calls what — before and after

We classified every analysis/meter consumer (§9). The result is **real but narrow consolidation**:

- **Subsume (2):** `consumption-api` (it *is* a consumption read API) and `computed-benchmark`.
- **Repoint (~8):** energy-model, alarms, reporting, export, climate, ok-carwash — they keep their domain logic but stop calling analysis/meter and read the common service instead.
- **Out of scope:** manual-readings, back-office, the legacy monolith.
- **Special:** `energy-cost` owns the price/factor data — which we plan to own anyway, so it becomes a reference-data owner and a *later* retirement candidate.

Their direct `me2db` reads turn out to be **hierarchy + meter identity** — which we already have — so those are repoint targets, not blockers. Users are owned by the hierarchy service; a **`me2-events` CDC sync** keeps everything current during switchover.

**Headline:** *the win is killing the analysis/meter fan-out — most services survive but stop reaching into each other.*

---

## 8. How we prove it before we build

Two **diff harnesses** are the go/no-go gates (§10), both comparing the new model to *current* `analysis_service` output on real data:

1. **Meter-hierarchy harness** — derive formulas from the old DB, recompute totals, diff against current summation per company.
2. **Reference-data harness** — old `values` (cost/CO2/degree-days) vs the new denormalised columns.

The hierarchy harness runs **offline** (no pipeline changes needed) and validates the biggest assumption.

**Headline:** *verify by replaying the old answers — no cutover until the numbers match.*

---

## 9. The path ahead

Stepwise, verify-before-build, two parallel build tracks, cutover last (§12):

- **Phase 0 — Verify & size** *(now, read-only):* run both harnesses, size the data, scope the critical path.
- **Track I — Ingestion:** parsers/Pipes/IoT Core so all sources are in the pipeline (§4).
- **Phase 1 — Extend the data** *(additive):* weather/cost/CO2 + aggregate columns; rename → `logical_sensor_data`.
- **Phase 2 — Formula & hierarchy:** wire formula eval into the pipeline; migrate the meter hierarchy per-company (dual-run).
- **Phase 3 — Consolidate & cut over:** extend `aggregations`; subsume/repoint consumers; move `yggdrasil` → frontend per company; retire legacy endpoints.

**Recommended first move:** the **meter-hierarchy diff harness** — it validates the biggest thesis, runs offline, and gates the largest build.

**Headline:** *verify first, extend the data and wire formulas in parallel, cut over per company.*

---

## 10. The open forks (decisions to make together)

- **User-facing meter model** — auto-netting + one unified concept (recommended) vs exposing formulas.
- **Data-volume sizing** — confirm the denormalised DynamoDB item is affordable before schema-freeze.
- **Own prices** — when to retire `energy-cost`.
- **Mechanical vs dual-run** per company for the hierarchy migration — decided by the harness.

**Headline:** *a handful of real decisions; the analysis has narrowed everything else.*
