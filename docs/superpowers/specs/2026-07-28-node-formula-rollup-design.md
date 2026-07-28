# Node-Formula Roll-up — Design Spec

**Date:** 2026-07-28
**Status:** Draft — approved for planning
**Components:** `crates/model`, `crates/services/hierarchy`, `crates/services/aggregations`,
`infra/hierarchy` (bridge + cross-account reader role), `infra/daq/data_pipeline`
(Flink, S3 Tables, Glue), `frontend`

---

## 1. Context

Talking to actual end users produced a different model for rolling up consumption:
**there is one kind of sensor, and all the intelligence lives on the hierarchy node.**

Today a customer is expected to understand main meters, sub meters, calculation meters
and sum meters — and they can't. So the sensor keeps only what it measures, and a node
says how its inputs combine: **a node's value is Σ everything below it, unless the node
says otherwise.**

`docs/hierarchy-presentation.html` is a runnable model of the arithmetic and the reference
for every number in this spec.

### 1.1 The per-sensor formula code is dead; the concept isn't

`crates/model/src/domain/formula.rs` (≈695 lines: `Expr`, recursive-descent parser,
unparser, `Formula`) is stored on the sensor and copied cross-account by the bridge — but
**never evaluated**. `logic/sensors.rs::evaluate` has exactly one caller: its own unit test.

So deleting it changes no live number. But the *concept* is live: technicians create
"calculation meters" to express things like `main − sub`. That capability must survive —
it moves onto the node, where it is expressed once as a formula instead of as a meter type
the customer has to classify.

### 1.2 The existing roll-up implements only the default

`infra/daq/data_pipeline/glue/measurements_aggregate.py` explodes each meter row into its
ancestor node paths and does `groupBy(...).sum()` — a flat Σ over sensors. That is the
default case and nothing more. What is new is subtract / exclude / weight, the purpose
axis, and **recursive** evaluation (§3.5).

### 1.3 Evidence from Energihåndbogen 2019 (`docs/energih-ndbogen-2019.pdf`)

- **The handbook's chapter list is a purpose taxonomy** — varmeproduktion, varmesystemer
  (rumopvarmning), varmt brugsvand, ventilation, køle-/fryseanlæg, belysning,
  bygningsautomatik, solceller. None are energy types; several are served by more than one,
  and electricity serves nearly all. `energy_type` (energitype) and `purpose` (formål) are
  independent axes.
- **Purpose attribution is weighted subtraction and fractions.** The *bimåler* rule
  (p. 96 ff): above 10 000 kWh/year the building regulations require a submeter on the heat
  supply to hot-water production, so space heating = total heat − hot water (coefficient
  −1). Without a submeter, the **GUF/GAF** split (p. 83) assigns the degree-day-independent
  part a standard **28–30 %** (coefficient 0.28).
- **Cross-type conversion is required.** Årsvirkningsgrader (87–104 % depending on øvre vs
  nedre brændværdi), COP/SCOP, brændværdi for gas m³ → kWh.

---

## 2. Decisions

| # | Decision | Rationale |
|---|---|---|
| **D1** | Two axes: **`energy_type`** (what the sensor measures) and **`purpose`** (what the energy is spent on). **No "metric" axis.** | Matches the handbook's formålsopdeling. CO₂ and cost stay derived at query time, so no formula is duplicated per metric. |
| **D2** | Terms are **weighted linear**: `(reference, coefficient)`. No `abs`, no division, no min/max. | Covers include (1), exclude (0), subtract (−1), apportion (0.28), COP (3.2), brændværdi × virkningsgrad (10.45). Linear ⇒ commutes with time-bucketing, and the whole tree flattens to a coefficient matrix (§3.7). |
| **D3** | Formulas store **overrides only** — anything unlisted contributes at weight 1. | Attaching a sensor always changes the numbers; nothing is ever silently dropped, and formulas need no maintenance on attach. |
| **D4** | `purpose` is **declared on the formula head**, together with the output `energy_type`. | The only option that supports both bimåler subtraction and COP/brændværdi conversion, and gives a cross-cutting end-use axis without forcing purposes into the tree shape. |
| **D5** | **One kind of sensor.** It carries `energy_type` and `reading_kind` and nothing else — no meter class, no containment, no direction. | The premise of the whole change: customers cannot classify main/sub/calc/sum meters, so don't ask them to. Every case those classes encoded is a node formula instead (§2.2). |
| **D6** | Evaluation is **recursive over node values**: a node sums its *children's values*, not the raw sensors beneath them. | A node that corrects itself is then right at every ancestor automatically, with nothing to restate upward. This is what makes D5 possible. |
| **D7** | Terms may reference **any sensor in the company**, but only **direct child nodes**. | Sensors are leaves with no formula, so sideways references cannot loop; `B1 = P − B2` while `P = ΣB` genuinely can, so node composition stays downward. Direct-child-only keeps "override" unambiguous — a deeper node is already counted through the chain. Required for the main-and-sub-in-different-buildings case (§3.6). |
| **D8** | The Glue job reads a **materialised coefficient matrix** from `hierarchy_new`, cross-account. | `crates/model` flattens the recursion once; Glue becomes one join and one grouped sum, holding **zero** formula semantics. |
| **D9** | Dev system: **rename outright, no back-compat scaffolding.** | No real users depend on stored history; read-fallbacks and dual-writes permanently muddy the model. |

### 2.1 Naming

`Resource` → **`EnergyType`** (`energy_type` on the wire, *Energitype* in the Danish UI);
`Resource` carried no meaning in energy management and did not translate from *energiart*.
`EnergyType` is slightly generous about `Water`, which is not energy; `Dimension` keeps that
honest (`Water`/`Gas` accumulate as `Volume`), and the industry uses "energiarter" the same
loose way. `MeterType { Counter, Gauge }` → **`ReadingKind`** — not a kind of *thing* but how
a sensor's readings accumulate. `logical_meter_data` → **`logical_data`**, `meter-identity` →
**`sensor-identity`**, `MeterMapping` → **`SensorMapping`**. The `/meterdata/` route prefix is
deliberately left alone — a public URL and a bounded-context label, with its own blast radius.

**Sensor, not meter.** The system's inputs are *sensors* — one per channel or register a
device exposes (`daq:<type>:<customer>:<device>:<channel>`). A *meter* is a physical thing a
technician installs; in the hierarchy it exists only as a **node type**. An accumulating
channel and its per-phase channels are registers on one device and hang off one node — the
hierarchy never nests a meter node inside a meter node.

### 2.2 What D5 replaces

Every meter class the customer was expected to understand becomes a node formula:

| Was | Now |
|---|---|
| "sum meter" | the default — no formula at all |
| "sub meter" nested in a main | `cL1×0, cL2×0, cL3×0` on the node holding both |
| production vs export | `EXP×−1` on the site node |
| "calculation meter" for `main − sub` | `SUB×−1` on the building holding the main |

It also removes the demo's per-sensor emission factor: `generation` is a purpose, and
emission/tariff factors key on `(energy_type, purpose)` with an energy-type-only fallback.

---

## 3. Domain model — `crates/model`

### 3.1 Deleted

- `domain/formula.rs` in full (`Expr`, `Formula`, parser, unparser, `expr_aliases`).
- `Sensor.formula`.
- `logic/sensors.rs`: `set_formula`, `evaluate`, `has_cycle` and the post-allocation cycle
  check + rollback in `attach` that exists only to serve it.

### 3.2 Renamed

`Sensor.purpose: Resource` → `Sensor.energy_type: EnergyType`;
`Sensor.meter_type: MeterType` → `Sensor.reading_kind: ReadingKind` (same `counter`/`gauge`
tokens, so no stored value changes — only the attribute name).

### 3.3 The sensor, in full

```rust
pub struct Sensor {
    pub id: SensorId,
    pub created: DateTime<Utc>,
    pub daq_id: String,
    pub path: String,
    pub energy_type: EnergyType,     // what it measures — the only classification
    pub reading_kind: ReadingKind,   // counter | gauge
    pub unit: Option<String>,
    pub resample_minutes: Option<i32>,
}
```

That is the whole thing. Nothing on a sensor says how it combines with any other sensor.

### 3.4 New types

```rust
/// Formål — what the energy is spent on. Independent of `EnergyType`.
pub enum Purpose {
    SpaceHeating, Dhw, Ventilation, Cooling, Lighting,
    PlugLoads, EvCharging, Process, Common,
    Generation,     // egenproduktion — reported, but never reduces Unallocated
    Total,          // the node's own value; declarable (this IS the node formula)
    Unallocated,    // Total − allocated; derived by the job, never declarable
}

pub enum Reference {
    Sensor(SensorId),   // any sensor in the company (D7)
    Node(NodeId),       // direct children only (D7)
}

pub struct Term { pub reference: Reference, pub coefficient: f64 }

pub struct NodeFormula {
    pub node: NodeId,
    pub energy_type: EnergyType,
    pub purpose: Purpose,          // Total is legal; Unallocated is not
    pub terms: Vec<Term>,          // overrides only (D3)
    pub note: Option<String>,
}
```

### 3.5 Evaluation

Recursive, per `(node, energy_type, purpose)`. `w(x)` is the formula's coefficient for `x`,
or the default.

**`Total`** — default 1 for every child node and every own sensor of that energy type:

```
value(N, E, Total) = Σ_children C   w(C) · value(C, E, Total)
                   + Σ_own sensors S of type E   w(S) · reading(S)
                   + Σ_sensors S named by N's formula but not under N   w(S) · reading(S)
```

**A named purpose** — default 1 for every child node; **sensors count only if named**,
because a sensor does not belong to a purpose by default (otherwise attaching a meter would
silently claim it as lighting):

```
value(N, E, P) = Σ_children C   w(C) · value(C, E, P)
               + Σ_sensors S named by N's formula   w(S) · reading(S)
```

**`Unallocated`**:

```
value(N, E, Unallocated) = value(N, E, Total) − Σ_{P allocating}  value(N, E, P)
```

A purpose **allocates** when it is neither derived (§3.8) nor an outflow (`Generation`).

The two defaults differ deliberately, and that asymmetry is the whole of D3 in practice: a
sensor is part of what the node *consumed* automatically, but part of a *purpose* only when
someone says so.

### 3.6 Why sideways sensor references (D7)

Technicians routinely install the main meter in one building and the submeter in another,
then express the first building's own use as `main − sub`. With `MAIN` on C1 and `SUB` on
C2, C1 declares `electricity/total = SUB × −1` and the numbers come out right at every
level:

| Node | Value |
|---|---|
| C1 | `MAIN − SUB` = 100 − 30 = **70** |
| C2 | `SUB` = **30** |
| Property C | C1 + C2 = **100** = MAIN ✓ |

No calculation meter, no new sensor kind, no double count. This is the case that requires
company-wide sensor references; restricting terms to descendants would forbid it.

It also makes **shared plant apportionable across siblings**, which a descendants-only rule
had foreclosed: a chiller sensor on the property, `cooling = CHILLER × 0.6` on B1 and
`× 0.4` on B2, summing to exactly 1.0 at the property.

### 3.7 Flattening — `logic/formulas.rs`

Because every term is linear, the whole recursion collapses to a coefficient matrix:

```rust
pub struct MatrixRow {
    pub node_path: String,
    pub energy_type: EnergyType,
    pub purpose: Purpose,        // Total, each declared purpose, and Unallocated
    pub sensor: SensorId,
    pub coefficient: f64,
    pub allocates: bool,         // !derived && !outflow — see §3.8
}

pub fn flatten(company: &CompanyGraph) -> Vec<MatrixRow>;
```

Computed bottom-up: a node's coefficient vector is the weighted sum of its children's
vectors plus its own named terms. `Unallocated` rows are pure arithmetic on the rows already
computed — `coeff(Total) − Σ_{P allocating} coeff(P)` — so the roll-up job never has to
subtract anything itself.

`value(N,E,P) = Σ_S coeff(N,E,P,S) · reading(S)`, which is exactly one join and one grouped
sum (§8).

### 3.8 Derived rows

A formula is **derived** when its output `energy_type` differs from the `energy_type` of a
sensor it names directly. Node references resolve to the formula's own energy type and never
make it derived.

| Kind | Example | Enters `Total`? | Reduces `Unallocated`? |
|---|---|---|---|
| Physical | `electricity/cooling = ACC × 1` | — (purposes never do) | yes |
| Physical | `district_heating/space_heating = HM1×1 + DHW×−1` | — | yes |
| Derived | `heat/space_heating = GAS × 10.45` | no | no |
| Derived | `district_cooling/cooling = ACC × 3.2` | no | no |
| Outflow | `electricity/generation = EXP × 1` | — | no |

Derived rows are delivered energy, not metered consumption. Outflow rows report exported
energy, which is not a slice of consumption. Neither reduces `Unallocated`; both still roll
up the tree like any other purpose.

**Derived-ness must be uniform per `(energy_type, purpose)` within a company** (§5.3) — a
series that mixed derived and physical contributions would make `Unallocated` ambiguous.

### 3.9 Termination

Node references are direct children, so the term graph is the hierarchy tree itself and the
bottom-up fold terminates in one pass. Sensors are leaves with no formulas, so sideways sensor
references cannot participate in a cycle at all. `has_cycle` is deleted rather than ported.

---

## 4. Storage — `hierarchy_new`

### 4.1 Formula items

| Field | Value |
|---|---|
| `pk` | `<NodeId>` (same partition as the node item) |
| `sk` | `formula#<energy_type>#<purpose>` |
| `gsi1pk` | `F#HN2#<company_id>` |
| `gsi1sk` | `<node_path>#<energy_type>#<purpose>` |
| `terms` | `L` of `M{ ref: S, coefficient: N }` |
| `note` | `S`, optional |
| `updated` | `S` (RFC 3339) |

Deleting a node cascades its formula items — same partition, already enumerated by the
existing delete path.

### 4.2 Materialised coefficient matrix

`flatten`'s output (§3.7), stored as derived items and recomputed by the hierarchy service
whenever the matrix can change (§5.2). **This is what the Glue job reads.**

| Field | Value |
|---|---|
| `pk` | `HN2#<company_id>` |
| `sk` | `weight#<node_path>#<energy_type>#<purpose>#<sensor>` |
| `gsi1pk` | `W#HN2#<company_id>` |
| `node_path`, `energy_type`, `purpose`, `sensor_id`, `coefficient`, `allocates` | as in `MatrixRow` |

One GSI query returns a whole company's matrix. Staleness is the cost: recompute runs inside
the command handler so a failure surfaces as a command error, and `rebuild_company_matrix`
is the manual repair.

---

## 5. Hierarchy service

### 5.1 Removed

`run_attach_sensor` drops all formula parameters; `api_json` stops reading and emitting
`formula`; `html/node.rs` loses the whole `formula-dialog` block and the company-sensor
`<option>` fragment endpoint that fed it.

### 5.2 Added

Commands on `/hierarchy/command`, gated by the **`writes`** edge:

- `set_node_formula` — `{ node, energy_type, purpose, terms, note? }` (upsert)
- `delete_node_formula` — `{ node, energy_type, purpose }`
- `rebuild_company_matrix` — `{ company }` (operator escape hatch)

**Matrix recompute** runs after any command that can change it: the three above plus
`attach_sensor`, `replace_sensor_device`, `delete_sensor`, `add_node`, `delete_node`.
(`update_node` touches only metadata.)

Query: `GET /hierarchy/query/node_formulas?node=<NodeId>` → HTML fragment.

UI: a **"Formler"** tab on the node panel — one card per formula, headed by its
`(energy_type, purpose)`, with rows of `[reference ▾] × [coefficient]`. The reference picker
offers **every sensor in the company** plus the node's **direct children**, matching D7, and
shows which node each sensor is attached to so a sideways reference is legible. A node with
no formula for the selected pair shows the default Σ read-only, so "nothing declared" is
visibly different from "declared as Σ".

### 5.3 Validation

| Rule | Failure |
|---|---|
| `coefficient` is finite (0 is legal — it is how you exclude) | 400 |
| `purpose` ≠ `unallocated` | 400 |
| `Reference::Node` is a **direct child** of the declaring node | 400 |
| `Reference::Sensor` is in the same company | 400 |
| for `purpose = Total`, a sensor term names either one of the node's **own** sensors or a sensor **outside its subtree** | 400 — a deeper descendant is already counted through the child chain; override the child instead |
| at most one formula per `(node, energy_type, purpose)` | upsert, not an error |
| for each `(sensor, energy_type)`, the signed sum of coefficients across all **allocating** claims in the company is ≤ 1 | 400 with the running total |
| derived-ness is uniform per `(energy_type, purpose)` in the company | 400 |

The ≤ 1 rule is what prevents a sensor being allocated more than it measured, while still
permitting a heat pump split across purposes (0.7 + 0.3), the bimåler pattern (+1 and −1
netting to 0), and shared plant apportioned across siblings (0.6 + 0.4).

---

## 6. Cross-account bridge — `infra/hierarchy/app.go`

The inlined Python `_item` builder drops the `formula` copy and renames `purpose` →
`energy_type`, `meter_type` → `reading_kind`. The **`meter-identity` table is replaced by
`sensor-identity`** — DynamoDB tables cannot be renamed, so this is a new table with a new
stream, and the bridge lambda, its DLQ and its alarm lose the old noun. The bridge repopulates
the table from `hierarchy_new` stream events, so re-saving each sensor is the whole migration.

Two consumers hang off the replaced stream, and they are not alike:

- **Flink has no event-source mapping.** It reads the Kinesis stream
  `flink-iceberg-processor-ddb-changes`, attached to the table by `KinesisStreamSpecification`,
  and learns the table name from the app property `METER_IDENTITY_TABLE`. Renaming is a property
  change plus a restart.
- **`backfill-trigger` does have an ESM** on the DynamoDB stream, declared in
  `LateRecomputationStack`. Replacing the table replaces that stream, so the ESM goes with it.

The table is `RemovalPolicy: RETAIN`, so CFN orphans `meter-identity` rather than deleting it.
The orphan keeps PITR billing and a live streaming destination into the same change stream, and
must be deleted by hand once the new table is verified.

**New CDK resource:** `HierarchyReaderRole` in `339712745226`, trusting the Glue job role in
`891377204778`, granting `dynamodb:Query` on `hierarchy_new` and its `gsi1`. That Glue role is
given a stable `RoleName` first — the CFN-generated name carries a random suffix, so trusting it
directly would let a future role replacement break the cross-account read silently.

---

## 7. Flink + Iceberg

- `MeterMapping` → `SensorMapping`; `.purpose` → `.energyType`, `.meterType` → `.readingKind`;
  follow through in both deserialisers, `MeterEnrichmentFunction` and `Main.scala`.
- The `purpose` column in `all.logical_meter_data` renames to `energy_type`, and the table to
  **`all.logical_data`**. **`raw_data` is not touched** — it has no `purpose` column, and
  `meter_type` is not an Iceberg column in either table.

`AWS::S3Tables::Table` cannot be replaced under an unchanged name (create-before-delete →
`409 … identical name already exists`), which is why CLAUDE.md documents a two-step deploy. A
**rename sidesteps it**: one deploy creates `logical_data` and orphans `logical_meter_data`.
Operator `uid` and keyed-state descriptors are untouched, so the Flink snapshot restores; only
the sink schema moves.

**Accepted data loss:** `logical_data` starts empty — about 26 k rows covering three months.
It is **rebuilt from `raw_data`** by a targeted `late-data-recomputation` over the mapped daqs,
which is the only correct recovery: `DAQ_INPUT_STREAM` retains just 24 h, and replaying it would
duplicate `raw_data` rather than restore anything. `raw_data` itself (311 M rows, 71 k daqs,
back to 2026-05-02) is never cleared.

**Flink is otherwise untouched** — it resamples per sensor and knows nothing about formulas.

---

## 8. Roll-up job — `glue/measurements_aggregate.py`

### 8.1 Keys

```
sk      = <node_path>#<energy_type>#<purpose>#<gran>#<bucket>
gsi1pk  = HN2#<id>#<dimension>#<purpose>
gsi1sk  = <node_path>#<gran>#<bucket>
```

The bucket stays last, so a fixed `(energy_type, purpose)` is a pure `BETWEEN` key-range.
`purpose` enters `gsi1pk` so the cross-type dimension view cannot sum `Total` together with
its own purpose breakdown.

### 8.2 The whole job

1. **Load the matrix.** One GSI query per company in the window (`gsi1pk = "W#HN2#<id>"`),
   via `HierarchyReaderRole`. Build a DataFrame; broadcast it.
2. **Join and sum.**
   `readings ⋈ matrix on logical_id = sensor_id`, then
   `groupBy(node_path, energy_type, purpose, gran, bucket).sum(resample_value × coefficient)`.
3. **Write.** Unchanged `foreachPartition` + `batch_writer(overwrite_by_pkeys=["pk","sk"])`.

That is the entire evaluation. `ancestor_keys` is **deleted** — ancestry is already baked
into the matrix. `Unallocated` needs no special stage; it arrives as ordinary matrix rows
(§3.7). There is no formula logic in PySpark: no recursion, no defaults, no derived
detection, no override handling.

### 8.3 Stored attributes

`min`, `max` and `last_value` are dropped — the read side never reads them, and under
weighting and subtraction they are meaningless. `count` stays as a coverage signal.

Per-sensor leaf rows keep being emitted, keyed by the sensor's own path
(`<node_path>|S#<id>`) rather than the old synthetic `L#<logical_id>`.

---

## 9. Read side — `crates/services/aggregations`

- `parse_sk` gains a segment; `Row` gains `energy_type` and `purpose` now means purpose.
- The query prefix becomes `<sk_path>#<energy_type>#<purpose>#<gran>#`, defaulting to
  `purpose=total`, so every existing widget keeps its meaning at its current cost.
- New `?purpose=` filter, and a `get_purpose_split` action for the end-use breakdown.
- `query_dimension` passes `purpose` into `gsi1pk` (callers pass `total`).
- `tariff_dkk_per_unit` / `emission_kg_per_unit` take `(energy_type, purpose)` with an
  energy-type-only fallback; `generation` returns 0.
- `get_alarms` and `get_benchmark` pin to `purpose=total`.

---

## 10. Accepted properties

- **Negative values are expected** (`main − sub`, `import − export`). Cost and CO₂ on a
  negative series correctly yield a credit.
- **Recomputes restate history with today's formulas** — the matrix is read live. A
  versioned snapshot would avoid it and was not chosen.
- **A node's consumption cannot be derived by difference from an ancestor.** `B1 = P − B2`
  needs an upward node reference, which D7 forbids because `P = ΣB` makes it circular.
  Sideways *sensor* references cover the real case (§3.6); the genuinely unmeasured case is
  reported as `Unallocated`, correctly labelled, rather than invented.
- **`Total` can disagree with Σ(purposes)** when a node is only partly claimed — the gap is
  `Unallocated`, and `Total` stays authoritative for cost, CO₂, alarms and benchmark.
- **A wrong formula is silently wrong.** Nothing in the data can tell the system that a
  chiller's phase channels are inside its accumulating channel; if nobody writes `×0`, the
  node reads high. This is an onboarding-data problem and the Formler tab is the only
  mitigation.
- **The materialised matrix can go stale** if a code path that should recompute doesn't;
  `rebuild_company_matrix` is the repair.
- **Coefficients are time-invariant.** A reversible heat pump heats in January and cools in
  July, but one coefficient applies to every hour; the handbook's GUF 28 % has the same
  character. Time-varying coefficients would break the linearity D2 depends on.
- **Iceberg history is cleared** by the column rename (§7).

---

## 11. Testing

| Layer | Coverage |
|---|---|
| `model` domain | `Purpose`, `EnergyType`, `ReadingKind` wire tokens round-trip; `Unallocated` rejected as a formula head; `Total` accepted |
| `model` logic (§3.5) | the two defaults (Total takes own sensors, a purpose does not); overrides applied to children on both paths; nested coefficient multiplication |
| `model` logic (§3.6) | the main-and-sub case yields 70 / 30 / 100; shared plant split 0.6 + 0.4 sums to 1.0 at the parent |
| `model` logic (§3.7) | `flatten` reproduces the recursive values for the whole presentation fixture; `Unallocated` rows equal `Total − Σ allocating` |
| `model` logic (§3.8) | derived and outflow rows do not reduce `Unallocated`; the handbook cases partition exactly |
| `model` logic (§5.3) | node reference that is not a direct child rejected; `Total` term naming a deeper descendant sensor rejected; sensor outside the company rejected; coefficients summing past 1 rejected; mixed derived-ness rejected |
| `model` repository | formula + matrix codec round-trip; `F#HN2#…` and `W#HN2#…` GSI queries; cascade delete |
| `hierarchy` api | all three commands, every validation failure, `writes` gating, recompute fires on each triggering command |
| `hierarchy` html | Formler tab renders terms; picker offers company-wide sensors and descendant nodes, showing each sensor's node |
| Glue | join + grouped sum reproduces the fixture's values; no formula logic present |
| `aggregations` | `parse_sk`; `purpose=total` default; `?purpose=`; `(energy_type, purpose)` factors incl. `generation → 0` |

---

## 12. Deploy order

1. `crates/model` + `hierarchy` lambda, then `rebuild_company_matrix` for every company.
2. **Migrate the stored `hierarchy_new` sensor rows** to `energy_type`/`reading_kind` and
   **deploy the bridge in the same window**. The migration writes every sensor row, which fires
   the table's stream into the bridge; a bridge still reading the old names fails on every record.
3. Stable `RoleName` on the DAQ Glue role, then `HierarchyReaderRole` in `339712745226`, then the
   `sts:AssumeRole` grant back in `891377204778`. Strictly in that order — each step names the
   resource created by the one before.
4. Bridge + `sensor-identity` — must land **with** step 5; the pipeline has no fallback. Repoint
   `backfill-trigger`'s ESM and the Flink `SENSOR_IDENTITY_TABLE` property, restart Flink, then
   delete the orphaned `meter-identity` (it is `RETAIN`, so CFN leaves it behind).
5. Flink + the `logical_data` rename (account `891377204778`), then rebuild its history from
   `raw_data` via `late-data-recomputation`.
6. Glue roll-up job.
7. `aggregations` lambda.
8. Frontend rebuild + deploy (`PUBLIC_*` are baked in at build time).

Wipe `measurements_aggregate` and re-run the Glue job with a wide `LookbackDays` — the sort
key gained a segment, so old rows are unreadable.

**Every step that renames an attribute on either side of a stream must ship with its consumer.**
Steps 2 and 4 are the two places this bites, and step 2 already bit once.

---

## 13. Out of scope

- Graddagekorrigering (weather normalisation) — a read-side concern, not a formula term.
- Formula versioning / point-in-time restatement (§10).
- Non-linear operators (`abs`, division, min/max).
- Time-varying coefficients.

---

## 14. Acceptance (measured 2026-07-28)

Run against the deployed stack — real klepierre sensors under SeedCo01 (`HN2#10003`),
formulas declared through `POST /command`, roll-up via the `measurements-aggregate`
Glue job, read back through `GET /meterdata/query/get_purpose_split`.

**Scope.** The presentation's exact numbers (`docs/hierarchy-presentation.html`) are not
reproduced here: its sensors carry synthetic series that real devices cannot emit. That
arithmetic is verified separately and more strictly by
`crates/model::logic::formulas::presentation_fixture_reproduces_the_demo`, which replays
the whole demo tree against an independently written JavaScript model. What this section
verifies is the part a unit test cannot: that formulas reach Glue through the materialised
matrix and come back correct through the API.

| Invariant | Declared | Measured | Result |
|---|---|---|---|
| Exact partition | `HN4#10001` dh: dhw `10013×0.28`, space_heating `10013×0.72` | 12,600 + 32,400 = 45,000 = total; unallocated **0**; dhw share 0.2800 | PASS |
| One sensor, many purposes | `HN4#10006` el: lighting `10016×0.4`, plug_loads `10016×0.6` | 159,147 + 238,721 = 397,868 = total; unallocated **0** | PASS |
| Sideways reference (bimåler) | `HN4#10005` el total = `10014×1` + `10011×−1`, where 10011 sits in `HN4#10001` | property = 9,902 + 37,950 = 47,852, vs 57,754 if the main were double-counted | PASS |
| Recursion | — (default) | property total == Σ children, no restating | PASS |
| Purposes roll up | — | property lighting 159,147 == building lighting | PASS |

**The main cancels symbolically.** After flattening, the property's `electricity/total`
row set contains only sensor `10014` with coefficient 1 — `10011` has disappeared
(+1 from one child, −1 from the other). Nothing at the property restates anything; the
matrix simply no longer mentions the sensor.

### Bug found by this acceptance run

**The roll-up job could not retract rows.** `write_to_dynamo` uses `PutItem`, which is
idempotent for rows the job still writes but cannot remove rows it has *stopped* writing.
Declaring the 0.28/0.72 split made `unallocated` cancel to zero, so the (correctly sparse)
matrix stopped emitting those rows — and the previous run's `unallocated` rows survived and
kept being served, at exactly the pre-formula value. Their TTL is 90 days hourly / 730 daily,
so the API would have returned a wrong end-use breakdown for months.

Fixed by `prune_window`: the job recomputes a whole day-aligned window, so anything in that
window it did not just write is by definition obsolete and is deleted. This needed a
`dynamodb:Query` grant on the rollup table — the Glue role previously had write only.
