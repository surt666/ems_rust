# Node-Formula Roll-up — Design Spec

**Date:** 2026-07-28
**Status:** Draft — approved for planning
**Components:** `crates/model`, `crates/services/hierarchy`, `crates/services/aggregations`,
`infra/hierarchy` (bridge + cross-account reader role), `infra/daq/data_pipeline`
(Flink, S3 Tables, Glue), `frontend`

---

## 1. Context

Talking to actual end users produced a different model for rolling up consumption:
**remove formulas from sensors and put them on hierarchy nodes.** A sensor becomes a
plain raw value; whether its contribution is zero, identity, or part of an expression
is decided by the nodes above it. The concept was presented as an interactive HTML
demo (`docs/hierarchy-presentation.html`).

Two pieces of ground truth shaped the design.

### 1.1 Sensor formulas are already dead weight

`crates/model/src/domain/formula.rs` (≈695 lines: `Expr`, recursive-descent parser,
unparser, `Formula`) is stored on the sensor, settable from the UI, and copied
cross-account by the bridge — **but it is never evaluated**. `logic/sensors.rs::evaluate`
has exactly one caller: its own unit test. Nothing in Flink, Glue, or the aggregations
lambda reads the `formula` attribute.

Consequence: deleting sensor formulas changes **no** live number. Every part of this
change is net-new work, not a migration of behaviour.

### 1.2 The existing roll-up already implements the demo's default

`infra/daq/data_pipeline/glue/measurements_aggregate.py` explodes each meter row into
its ancestor node paths (`ancestor_keys`) and does `groupBy(...).sum()`. That is
literally "Σ everything below". What is genuinely new is **subtract / exclude / weight**
and the **purpose axis**.

### 1.3 Evidence from Energihåndbogen 2019 (`docs/energih-ndbogen-2019.pdf`)

Scanned for how Danish energy accounting actually splits consumption. Three findings
drive the model:

- **The handbook's chapter list is a purpose taxonomy** — varmeproduktion, varmesystemer
  (rumopvarmning), varmt brugsvand, ventilation, køle-/fryseanlæg, belysning,
  bygningsautomatik, solceller. None of these are energy carriers; several are served by
  more than one carrier, and electricity serves nearly all of them. `resource` (energiart)
  and `purpose` (formål) are independent axes.
- **Purpose attribution is done by weighted subtraction and by fractions.** The *bimåler*
  rule (p. 96 ff): if annual DHW + circulation exceeds 10 000 kWh, the building
  regulations require a submeter on the heat supply to hot-water production — so DHW is
  metered and **space heating = total heat − DHW** (coefficient −1). Where no submeter
  exists, the **GUF/GAF** split (p. 83) assigns the degree-day-independent part (DHW +
  distribution and standby losses) a **standard 28–30 % of annual heat** (coefficient
  0.28). Both mechanisms are pure weighted linear terms.
- **Cross-resource conversion is required.** Årsvirkningsgrader for boilers (87–104 %
  depending on øvre vs nedre brændværdi), COP/SCOP for heat pumps and chillers, and
  brændværdi for gas m³ → kWh. Turning a *measured carrier* into *delivered energy for a
  purpose* is a coefficient that can also change the unit family.

---

## 2. Decisions

| # | Decision | Rationale |
|---|---|---|
| **D1** | Two axes: **`resource`** (what the meter measures) and **`purpose`** (what the energy is spent on). **No "metric" axis.** | Matches the handbook's formålsopdeling. CO₂ and cost stay derived at query time, so no formula is duplicated per metric. |
| **D2** | Terms are **weighted linear**: `(reference, coefficient)`. No `abs`, no division, no min/max. | Covers include (1), subtract (−1), exclude (0), apportion (0.4), CO₂/COP/brændværdi factors, unit conversion. Linear ⇒ commutes with time-bucketing, so the roll-up keeps its one-pass shape. |
| **D3** | Formulas store **overrides only** — an unlisted descendant contributes at weight 1. | Attaching a meter always changes the numbers; nothing is ever silently dropped. Formulas need no maintenance on attach or move. |
| **D4** | `purpose` is **declared on the formula head**, together with the output `resource`. | The only option that supports both bimåler subtraction and COP/brændværdi conversion, and gives a cross-cutting end-use axis without forcing purposes into the tree shape. |
| **D5** | The job emits **`total`** and **`unallocated`** (`total − Σ claimed`) alongside declared purposes. `total` honours weight overrides and excludes `generation` — see §3.8. | "Øvrigt/fælles forbrug" is a real reported category, not an error state; a node may legitimately be only partly instrumented. Coverage gaps become visible instead of invisible. |
| **D6** | The Glue job reads **`hierarchy_new` directly cross-account** and builds the weight matrix in the driver. | Chosen over a versioned S3 snapshot and over a bridge-mirrored DynamoDB table. No new artifact or write path; always current by construction. Costs are accepted in §10. |
| **D7** | Dev system: **rename outright, no back-compat scaffolding.** | No real users depend on stored history. Read-fallbacks and dual-writes buy nothing and permanently muddy the model (the current `purpose` attribute holding a `Resource` token is exactly that scar). |

### 2.1 What D1 kills

The demo's per-sensor emission factor `f`. Under `(resource, purpose)` it has no reason
to exist:

- `EXP` (solar export, `f = 0`) is not "a metric with a different sign" — it is
  `electricity / generation`, and the emission-factor table returns 0 for that purpose.
- `SIGNAGE` (`f = 0.05`) was simply mis-modelled; signage is `electricity / plug_loads`
  at the grid factor.

Emission and tariff factors therefore key on **`(resource, purpose)` with a
resource-only fallback**, which is a three-line change to the existing lookup tables and
requires no per-sensor coefficient anywhere.

---

## 3. Domain model — `crates/model`

### 3.1 Deleted

- `domain/formula.rs` in full: `Expr`, `Formula`, `parse_expr_str`, `expr_to_string`,
  `expr_aliases` (≈695 lines).
- `Sensor.formula`.
- `logic/sensors.rs`: `set_formula`, `evaluate`, `has_cycle` (and the post-allocation
  cycle check + rollback in `attach` that exists only to serve it).
- The `formula` attribute in the DynamoDB codec.

### 3.2 Renamed

`Sensor.purpose: Resource` → `Sensor.resource: Resource`. This frees the word `purpose`
for its real meaning and propagates to the wire (§6, §7).

### 3.3 New

```rust
/// Formål — what the energy is spent on. Independent of `Resource`.
pub enum Purpose {
    SpaceHeating,   // rumopvarmning
    Dhw,            // varmt brugsvand
    Ventilation,
    Cooling,        // køl / frys
    Lighting,       // belysning
    PlugLoads,
    EvCharging,
    Process,        // produktion
    Common,         // fællesforbrug
    Generation,     // egenproduktion (PV export) — an outflow, see §3.8
    Total,          // declarable, but only as weight overrides — see §3.8
    Unallocated,    // reserved: emitted by the job, never declarable
}

pub enum Reference {
    Sensor(SensorId),
    Node(NodeId),
}

pub struct Term {
    pub reference: Reference,
    pub coefficient: f64,
}

pub struct NodeFormula {
    pub resource: Resource,        // declared output carrier
    pub purpose: Purpose,          // declared output end-use (never Total/Unallocated)
    pub terms: Vec<Term>,
    pub note: Option<String>,      // "COP 3,2 jf. datablad", "GUF 28 % jf. Energihåndbogen"
}
```

Wire tokens are lower-case `snake_case` (`space_heating`, `dhw`, …), same convention as
`Resource`, and are keyed verbatim into the roll-up sort key.

### 3.4 Flattening — `logic/formulas.rs`

```rust
pub struct WeightRow {
    pub declaring_node: String,   // node_path of the node whose formula this is
    pub resource: Resource,       // declared output
    pub purpose: Purpose,
    pub sensor: SensorId,
    pub coefficient: f64,
    pub derived: bool,            // see §3.6
}

pub fn flatten(company: &CompanyGraph) -> Vec<WeightRow>;
```

A term referencing a child node expands to that child's rows scaled by the coefficient;
a term referencing a sensor emits one row. Coefficients multiply through nesting.

### 3.5 The subtree rule

**A formula's terms may only reference the declaring node's own descendants** (child
nodes and descendant sensors).

Every example in the presentation obeys it — MSB's three phases, CHILL's accumulator and
excluded phases, A1b's import and export. The rule buys three things at once:

- the reference graph is **acyclic by construction**, so `has_cycle` is deleted rather
  than ported;
- **reparenting is safe** — a subtree carries its formulas with it;
- flattening is a single downward walk, which keeps the PySpark re-implementation (D6)
  trivial enough not to drift.

### 3.6 Physical vs derived rows

A formula is **derived** iff its declared output `resource` differs from the `resource`
of any sensor it references. This is computable from the data with no extra user input,
in both Rust and PySpark.

| Kind | Example | Part of `total`/`unallocated`? |
|---|---|---|
| Physical | `electricity / cooling = CHILLER_EL × 1` | yes |
| Physical | `district_heating / space_heating = MAIN × 1 + DHW × −1` | yes |
| Derived | `heat / space_heating = GAS_M3 × 11.0 × 0.95` | no |
| Derived | `district_cooling / cooling = CHILLER_EL × 3.2` | no |

Derived rows are a *delivered-energy* view, not a metered consumption, so folding them
into a physical total would double count. They still roll up the ancestor chain like any
other purpose series.

### 3.7 Claim propagation

A sensor claimed for purpose `P` at any node is claimed for **every ancestor** of that
node too. So a `lighting` claim declared on an area appears in its building's, property's
and company's `lighting` series — purpose series roll up the tree exactly like `total`
does, and `unallocated` at the company reflects everything claimed anywhere below it.

The invariant that keeps this sound: **a sensor may be claimed for a given
`(resource, purpose)` by at most one node in the company** (§5.3). Otherwise a shared
ancestor would count it twice.

## 3.8 What `total` actually sums

D5 originally said `total` is a *blind* Σ of every descendant sensor. Working the
handbook examples through showed that is wrong in two ways that matter, so `total` is
defined as follows instead.

**(a) `total` honours weight overrides.** Nested meters are the norm, not the exception:
the chiller's accumulator already contains its three phase meters, and the DHW submeter
already sits inside the building's main heat meter. A blind Σ double counts both — the
chiller reads 80 kWh instead of 40, and the building reads 155 kWh of heat instead of 120.

So `Purpose::Total` **is** declarable, carrying override terms only:

```
electricity / total      = cL1×0, cL2×0, cL3×0   "phases are inside the accumulator"
district_heating / total = DHW×0                 "submeter is inside the main meter"
```

This is just D3 (override-only) applied to the default series: a sensor nobody mentions
still counts at weight 1. An override declared at node *N* applies at *N* **and every
ancestor of *N*** — the same propagation rule as §3.7, and correct, since a meter nested
inside another is nested all the way up the tree.

**(b) `generation` is an outflow and is removed from `total`.** A PV export meter measures
energy leaving the site. Summing it into consumption inflates the total and — because
tariff and emission factors are applied to `total` — bills and emits CO₂ for exported
energy. A sensor claimed by `Purpose::Generation` therefore contributes **0** to `total`;
its own `generation` series reports it, and a net figure is `total − generation` at read
time.

Together:

```
total(node, resource)        = Σ over descendant sensors of that resource
                                 × (nearest declared total-override, default 1)
                                 × 0 if claimed as generation
unallocated(node, resource)  = total − Σ(declared purposes, excluding derived and generation)
```

With those rules the building's heat partitions exactly — `space_heating` 85 + `dhw` 35 =
`total` 120, `unallocated` 0 — and the chiller's `cooling` 40 equals its `total` 40.

---

## 4. Storage — `hierarchy_new`

Formulas are **first-class items**, not an attribute on the node. This mirrors the
existing `has_sensor` edge (same partition as the node) and the sensor GSI, and it lets
the Glue job load a whole company's matrix in **one query**.

| Field | Value |
|---|---|
| `pk` | `<NodeId>` — e.g. `HN5#10042` (same partition as the node item) |
| `sk` | `formula#<resource>#<purpose>` |
| `gsi1pk` | `F#HN2#<company_id>` |
| `gsi1sk` | `<node_path>#<resource>#<purpose>` |
| `terms` | `L` of `M{ ref: S, coefficient: N }` |
| `note` | `S`, optional |
| `created` / `updated` | `S` (RFC 3339) |

Node and sensor items are unchanged apart from the `purpose` → `resource` rename.
Deleting a node cascades its formula items — they are in the node's own partition, which
the existing delete path already enumerates.

---

## 5. Hierarchy service — `crates/services/hierarchy`

### 5.1 Removed

`run_attach_sensor` drops all formula parameters; `api_json` stops reading and emitting
`formula`; `html/node.rs` loses the entire `formula-dialog` block (kind select,
expression input, alias→sensor ref rows, the three hidden `data.formula.*` inputs and
their inline JS) and the company-sensor `<option>` fragment endpoint that fed it.

### 5.2 Added

Commands on `/hierarchy/command`, gated by the **`writes`** edge (same gate as attaching
a sensor):

- `set_node_formula` — `{ node, resource, purpose, terms: [{ref, coefficient}], note? }`
  (upsert on `(node, resource, purpose)`)
- `delete_node_formula` — `{ node, resource, purpose }`

Query:

- `GET /hierarchy/query/node_formulas?node=<NodeId>` → HTML fragment

UI: a **"Formler"** tab on the node panel. One card per formula, headed by its
`(resource, purpose)`, with term rows of `[reference ▾] × [coefficient]` and a note
field. The reference picker offers only the node's descendants, so the subtree rule is
enforced in the UI as well as in the command. HTML-over-the-wire, server-rendered maud,
consistent with the rest of the panel.

### 5.3 Validation

| Rule | Failure |
|---|---|
| `coefficient` is finite; non-zero except on a `total` override | 400 |
| `purpose` ≠ `unallocated`; `purpose = total` is accepted but its terms are treated as weight overrides (§3.8) | 400 |
| every `reference` is a descendant of the declaring node | 400 |
| at most one formula per `(node, resource, purpose)` | upsert, not an error |
| a sensor is claimed for a given `(resource, purpose)` by at most one node in the company | 409 with the conflicting node |

The last rule is the one that keeps §3.7 sound; it needs a company-wide check on the
`F#HN2#<id>` GSI partition at command time.

---

## 6. Cross-account bridge — `infra/hierarchy/app.go`

The inlined Python `_item` builder:

- **drops** the `formula` copy entirely;
- **renames** `purpose` → `resource` in the `meter-identity` item.

Nothing else changes — the bridge does not carry formulas, because the Glue job reads
`hierarchy_new` directly (D6).

**New CDK resource:** a `HierarchyReaderRole` in account `339712745226`, trusting the
Glue job role in `891377204778`, granting `dynamodb:Query` on `hierarchy_new` and its
`gsi1` index. (This is the mirror of the existing `OcamlBridgeWriterRoleStack`, which
lets the hierarchy account write into daq.)

---

## 7. Flink + Iceberg — `infra/daq/data_pipeline`

- `MeterMapping.purpose` → `.resource`, plus `DdbBootstrapLoader`, `DdbStreamDeserializer`,
  `MeterEnrichmentFunction` and `Main.scala`'s table schema.
- The `purpose` column in **`all.raw_data`** and **`all.logical_meter_data`** renames to
  `resource`.

Per the documented gotcha, `AWS::S3Tables::Table` cannot be replaced in place: this is a
**two-step deploy** — remove the table resource from `s3tables_stack.go` and deploy (CFN
deletes it, clearing the data), then restore it with the new column and deploy again.

The operator `uid` and keyed-state descriptor names are untouched, so the Flink snapshot
still restores; only the sink schema changes, which needs a redeploy after the tables
are recreated.

**Accepted data loss:** `raw_data` and `logical_meter_data` history is cleared. Kinesis
retention is 24 h, so roughly one day is recoverable by replay.

---

## 8. Roll-up job — `glue/measurements_aggregate.py`

### 8.1 Keys

```
sk      = <node_path>#<resource>#<purpose>#<gran>#<bucket>
gsi1pk  = HN2#<id>#<dimension>#<purpose>
gsi1sk  = <node_path>#<gran>#<bucket>
```

The bucket stays last, so a fixed `(resource, purpose)` is still a pure
`BETWEEN` key-range — the common read path costs exactly what it does today. `purpose`
enters `gsi1pk` rather than `gsi1sk` so the cross-resource dimension view cannot silently
sum `total` together with its own purpose breakdown.

### 8.2 Stages

1. **Matrix load (new).** For each distinct `hn2` in the window, the driver assumes
   `HierarchyReaderRole` and issues two GSI queries: `gsi1pk = "F#HN2#<id>"` for the
   company's formulas, and `gsi1pk = "S#HN2#<id>"` for its active sensors (which supply
   each sensor's own `resource` and path). It flattens them into `WeightRow`s and
   broadcasts the result.
2. **`total` rows.** `ancestor_keys` explode as today, but each row carries the sensor's
   total-weight from the matrix instead of a hard-coded 1 (§3.8): the nearest declared
   override, 0 if claimed as `generation`, else 1.
3. **Declared purposes.** Join the matrix on `logical_id`, explode `declaring_node` into
   its ancestor paths (§3.7), then
   `groupBy(node_path, resource, purpose, gran, bucket).sum(resample_value * coefficient)`.
4. **`unallocated`.** Per `(node_path, resource, gran, bucket)`:
   `total − Σ(declared purposes, excluding derived and generation)`.
5. **Write.** Unchanged `foreachPartition` + `batch_writer(overwrite_by_pkeys=["pk","sk"])`.

### 8.3 Stored attributes

`min`, `max` and `last_value` are **dropped** — the read side never reads them (only
`sum` and `count` are consumed), and under weighting and subtraction they are
meaningless. `count` stays as a coverage signal.

### 8.4 Drift control

D6 puts a second implementation of the flattening rules in PySpark. Mitigation: a
**golden-fixture parity test** — one JSON fixture of nodes, formulas and sensors, run
through both `model::logic::formulas::flatten` and the PySpark matrix builder, asserting
identical `WeightRow` sets. It runs in both `cargo test` and the Glue job's local test
suite.

---

## 9. Read side — `crates/services/aggregations`

- `parse_sk` gains a segment; `Row.purpose` finally means purpose, and a new
  `Row.resource` carries the carrier.
- `query_one_resource`'s prefix becomes `<sk_path>#<resource>#<purpose>#<gran>#`,
  defaulting to `purpose=total` — so every existing widget keeps its current meaning at
  its current cost.
- New `?purpose=` filter on `get_aggregations`; "all purposes for one resource" fans out
  over the declared purposes concurrently, the same shape as today's fan-out over the six
  resources.
- New `get_purpose_split` action backing an end-use breakdown widget.
- `query_dimension` passes `purpose` into `gsi1pk` (callers pass `total`).
- `tariff_dkk_per_unit` and `emission_kg_per_unit` take `(resource, purpose)` with a
  resource-only fallback; `generation` returns 0 — this is what replaces the demo's
  per-sensor `f`.
- `get_alarms` and `get_benchmark` pin to `purpose=total`; a median-spike test and an
  area-normalised peer comparison are both meaningless on a subtractive series.

---

## 10. Accepted properties

These are consequences of the chosen options, documented rather than fixed:

- **Negative sums are expected** (`total − DHW`, `import − export`). Cost and CO₂ on a
  negative series correctly yield a credit.
- **Recomputes restate history with today's formulas.** D6 reads the live table, so
  widening `LookbackDays` after a formula edit rewrites past buckets. A versioned
  snapshot would have avoided this and was not chosen.
- **A missing submeter inflates a subtractive purpose** rather than under-reporting it —
  the opposite of today's fail-safe behaviour. `count` is the signal that a bucket is
  partial.
- **`total` can disagree with Σ(purposes)** when a node is only partly claimed (the gap is
  `unallocated`) or when a meter is deliberately referenced twice at different weights.
  `total` is authoritative for cost, CO₂, alarms and benchmark.
- **Double counting is no longer structurally impossible.** Today's flat explode cannot
  double count; with nested meters it takes an explicit `total` override (§3.8) to avoid
  it. A node whose accumulator overlaps its submeters and declares no override reads high
  — the UI should surface this, but nothing enforces it.
- **Iceberg history is cleared** by the column rename (§7).
- **Two implementations of flattening** exist (Rust + PySpark), held together by the
  parity test in §8.4.

---

## 11. Testing

| Layer | Coverage |
|---|---|
| `model` domain | `Purpose` wire-token round-trip; reserved purposes rejected; `Term`/`NodeFormula` construction |
| `model` logic | `flatten`: nested coefficient multiplication, override-only defaults, subtree-rule rejection, derived detection, claim propagation to ancestors |
| `model` logic (§3.8) | `total` weight overrides propagate to ancestors (nested accumulator, nested DHW submeter); `generation` claims contribute 0 to `total`; the handbook cases partition exactly (`space_heating + dhw = total`, `unallocated = 0`) |
| `model` repository | formula item codec round-trip; `F#HN2#…` GSI partition query; cascade delete with the node |
| `hierarchy` api | `set_node_formula` / `delete_node_formula` happy paths and all five validation failures; `writes`-edge gating; `node_formulas` fragment |
| `hierarchy` html | Formler tab renders terms; reference picker lists only descendants |
| Glue | `build_rollups` with a weight matrix: subtraction, exclusion (weight 0), apportionment, `unallocated` arithmetic, derived rows excluded from totals, new sk/gsi1 shapes |
| Parity | golden fixture through Rust `flatten` and the PySpark builder (§8.4) |
| `aggregations` | `parse_sk` with the new segment; `purpose=total` default; `?purpose=` filter; `(resource, purpose)` factor lookup incl. `generation → 0` |

---

## 12. Deploy order

1. `crates/model` + `hierarchy` lambda (formula items readable/writable before anything
   reads them).
2. `HierarchyReaderRole` (account `339712745226`).
3. Bridge (`purpose` → `resource` in `meter-identity`) — must land **with** step 4, since
   the pipeline has no fallback.
4. Flink + S3 Tables two-step column rename (account `891377204778`).
5. Glue roll-up job.
6. `aggregations` lambda.
7. Frontend rebuild + deploy (`PUBLIC_*` are baked in at build time).

Wipe `measurements_aggregate` and re-run the Glue job with a wide `LookbackDays` over
whatever raw data survives step 4.

---

## 13. Out of scope

- Graddagekorrigering (weather normalisation of the GAF part) — a per-`(node, purpose)`
  read-side concern, not a formula term.
- Formula versioning / point-in-time restatement (see §10).
- Non-linear operators (`abs`, division, min/max).
- Tenant apportionment UI beyond a plain coefficient.
