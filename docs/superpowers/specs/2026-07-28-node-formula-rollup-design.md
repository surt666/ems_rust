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
demo (`docs/hierarchy-presentation.html`), which is kept in sync with this spec and is
a runnable model of the arithmetic.

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
  bygningsautomatik, solceller. None of these are energy types; several are served by
  more than one, and electricity serves nearly all of them. `energy_type` (energitype)
  and `purpose` (formål) are independent axes.
- **Purpose attribution is done by weighted subtraction and by fractions.** The *bimåler*
  rule (p. 96 ff): if annual DHW + circulation exceeds 10 000 kWh, the building
  regulations require a submeter on the heat supply to hot-water production — so DHW is
  metered and **space heating = total heat − DHW** (coefficient −1). Where no submeter
  exists, the **GUF/GAF** split (p. 83) assigns the degree-day-independent part (DHW +
  distribution and standby losses) a **standard 28–30 % of annual heat** (coefficient
  0.28). Both mechanisms are pure weighted linear terms.
- **Cross-type conversion is required.** Årsvirkningsgrader for boilers (87–104 %
  depending on øvre vs nedre brændværdi), COP/SCOP for heat pumps and chillers, and
  brændværdi for gas m³ → kWh. Turning a *measured* energy type into *delivered energy
  for a purpose* is a coefficient that can also change the unit family.

---

## 2. Decisions

| # | Decision | Rationale |
|---|---|---|
| **D1** | Two axes: **`energy_type`** (what the sensor measures) and **`purpose`** (what the energy is spent on). **No "metric" axis.** | Matches the handbook's formålsopdeling. CO₂ and cost stay derived at query time, so no formula is duplicated per metric. |
| **D2** | Terms are **weighted linear**: `(reference, coefficient)`. No `abs`, no division, no min/max. | Covers include (1), subtract (−1), apportion (0.4), CO₂/COP/brændværdi factors, unit conversion. Linear ⇒ commutes with time-bucketing, so the roll-up keeps its one-pass shape. |
| **D3** | Formulas store **overrides only** — an unlisted descendant contributes at weight 1. | Attaching a sensor always changes the numbers; nothing is ever silently dropped. Formulas need no maintenance on attach. |
| **D4** | `purpose` is **declared on the formula head**, together with the output `energy_type`. | The only option that supports both bimåler subtraction and COP/brændværdi conversion, and gives a cross-cutting end-use axis without forcing purposes into the tree shape. |
| **D5** | The job emits implicit **`total`** (Σ) and **`unallocated`** (`total − Σ claimed`) alongside declared purposes. | "Øvrigt/fælles forbrug" is a real reported category, not an error state; a node may legitimately be only partly instrumented. Coverage gaps become visible instead of invisible. |
| **D6** | The Glue job reads **`hierarchy_new` directly cross-account** — reading the **materialised weight matrix**, not raw formulas (§4.2, §8). | No new artifact or write path, always current by construction, and Glue holds **zero** formula semantics: one GSI query, one join, one weighted sum. |
| **D7** | Dev system: **rename outright, no back-compat scaffolding.** | No real users depend on stored history. Read-fallbacks and dual-writes buy nothing and permanently muddy the model (the current `purpose` attribute holding an energy type is exactly that scar). |
| **D8** | **Overlapping readings** (`contained_in`) and **flow direction** (`flow`) are facts on the sensor, not formulas. | See §3.8. Overlap handling keeps double counting structurally impossible, which the flat explode gives today and a per-node override would have surrendered. Direction said once on the export sensor is automatically right at every level; said as a node formula it needs `Purpose::Total` declarable again plus an ancestor-propagation rule that can be silently forgotten. |

### 2.1 Naming

`Resource` was renamed to **`EnergyType`** (`energy_type` on the wire, *Energitype* in
the Danish UI) because `Resource` carries no meaning in energy management and did not
translate from *energiart* in either direction. `EnergyType` is slightly generous about
`Water`, which is not energy; the model handles that honestly through `Dimension`
(`Water` and `Gas` accumulate as `Volume`, everything else as `Energy`), and the
industry uses "energiarter" the same loose way. `Medium` (EN 13757 / M-Bus) was the
standards-exact alternative and was not chosen.

**Sensor, not meter.** The system's inputs are *sensors* — one per channel or register a
device exposes (`daq:<type>:<customer>:<device>:<channel>`). A *meter* is a physical
thing a technician installs; in the hierarchy it exists only as a **node type** grouping
sensors. Every rule below is therefore stated over sensors. This matters most for
`contained_in`: an accumulating channel and its per-phase channels are usually registers
on **one** device, as are OBIS 1.8.0 and 2.8.0 (import and export), so the relation is
"this sensor's reading already includes that one's" — a statement about values, not about
hardware.

The same correction applies to three existing names, all renamed here (§6, §7):

| Before | After | Why |
|---|---|---|
| `logical_data` (Iceberg) | **`logical_data`** | Pairs with `raw_data`; the axis that differs is device-addressed vs hierarchy-addressed, not "meter" |
| `meter-identity` (DynamoDB) | **`sensor-identity`** | It resolves an incoming device channel to a hierarchy sensor |
| `MeterType { Counter, Gauge }` | **`ReadingKind { Counter, Gauge }`** | Not a kind of *thing* — it is how a sensor's readings accumulate. `SensorType` would repeat the `Resource` mistake |

Deliberately **not** renamed here: the `/meterdata/` route prefix. It is a public URL
touching the frontend build, the OpenAPI spec and the CQRS-convention docs, and as a
bounded-context label it is far less wrong than a type name. Worth doing separately.

### 2.2 What D1 kills

The demo's per-sensor emission factor `f`. Under `(energy_type, purpose)` it has no
reason to exist:

- `EXP` (solar export, `f = 0`) is not "a metric with a different sign" — it is
  `electricity / generation`, and the emission-factor table returns 0 for that purpose.
- `SIGNAGE` (`f = 0.05`) was simply mis-modelled; signage is `electricity / plug_loads`
  at the grid factor.

Emission and tariff factors therefore key on **`(energy_type, purpose)` with an
energy-type-only fallback**, which is a three-line change to the existing lookup tables
and requires no per-sensor coefficient anywhere.

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

`Sensor.purpose: Resource` → `Sensor.energy_type: EnergyType`, and the `Resource` type
itself → `EnergyType`. This frees the word `purpose` for its real meaning and propagates
to the wire (§6, §7).

`Sensor.meter_type: MeterType` → `Sensor.reading_kind: ReadingKind`, same `counter` /
`gauge` wire tokens, so no stored value changes — only the attribute name.

### 3.3 New

```rust
/// Formål — what the energy is spent on. Independent of `EnergyType`.
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
    // Reserved — emitted by the roll-up job, never declarable:
    Total,
    Unallocated,
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
    pub node: NodeId,
    pub energy_type: EnergyType,   // declared output
    pub purpose: Purpose,          // declared output (never Total/Unallocated)
    pub terms: Vec<Term>,
    pub note: Option<String>,      // "COP 3,2 jf. datablad", "GUF 28 % jf. Energihåndbogen"
}
```

and on the sensor, two facts about the installation (§3.8):

```rust
/// Which way energy flows through a sensor. Import, production and submeter
/// channels are `In`; grid export and PV feed-in are `Out` and contribute
/// negatively.
pub enum Flow { In, Out }

pub struct Sensor {
    …
    /// The sensor whose reading already includes this one's, if any.
    pub contained_in: Option<SensorId>,
    /// Direction of flow through this sensor. Defaults to `In`.
    pub flow: Flow,
}
```

**Neither can be inferred.** A survey of the live `raw_data` corpus (2026-07-28)
found channel names to be vendor-specific and semantically opaque — `a04` alone spans
42 330 devices, `volume` carries `kWh` while `energy` carries `J`, and a search of the
whole corpus for `import|export|deliver|receiv|feed|neg|pos|1_8|2_8|produc|generat`
matched only `valve_position_*` and `temp_in`/`temp_out`. Whether direction is
determinable at all depends on the meter make and on whether it arrives by CSV or API.
Both fields are therefore **entered at onboarding**, and both are prompted for on the
add-sensor form (§5.2).

Wire tokens are lower-case `snake_case` (`space_heating`, `dhw`, `district_heating`, …),
same convention as before, and are keyed verbatim into the roll-up sort key.

### 3.4 Flattening — `logic/formulas.rs`

```rust
pub struct Claim {
    pub declaring_node: String,     // node_path of the node whose formula this is
    pub energy_type: EnergyType,
    pub purpose: Purpose,
    pub sensor: SensorId,
    pub coefficient: f64,
    pub derived: bool,              // §3.7
    pub allocates: bool,            // !derived && !purpose.is_outflow() — see §8.2
}

/// A `(node, sensor)` pair whose weight in `total` is not the default 1.
pub struct TotalOverride {
    pub node_path: String,
    pub energy_type: EnergyType,
    pub sensor: SensorId,
    pub coefficient: f64,
}

pub struct Matrix {
    pub claims: Vec<Claim>,
    pub total_overrides: Vec<TotalOverride>,
}

pub fn flatten(company: &CompanyGraph) -> Matrix;
```

A term referencing a child node expands to that node's sensors of the formula's
`energy_type`, each scaled by the coefficient and by its own total weight; a term
referencing a sensor emits one claim. Coefficients multiply through nesting.

`total_overrides` is deliberately an **exception list**: only `(node, sensor)` pairs
whose weight differs from 1 appear, so the roll-up job left-joins and defaults to 1.
For a typical company it is a handful of rows.

### 3.5 The subtree rule

**A formula's terms may only reference the declaring node's own descendants** (child
nodes and descendant sensors).

Every example in the presentation obeys it. The rule buys three things at once:

- the reference graph is **acyclic by construction**, so `has_cycle` is deleted rather
  than ported;
- **reparenting is safe** — a subtree carries its formulas with it;
- flattening is a single downward walk.

### 3.6 Claim propagation

A sensor claimed for purpose `P` at any node is claimed for **every ancestor** of that
node too. So a `lighting` claim declared on an area appears in its building's, property's
and company's `lighting` series — purpose series roll up the tree exactly like `total`
does, and `unallocated` at the company reflects everything claimed anywhere below it.

**One sensor may feed many purposes.** A heat pump's electricity channel, serving both
space heating and hot water with its delivered heat reported alongside, is three formulas
over one sensor:

```
electricity / space_heating = HP_EL × 0.7
electricity / dhw           = HP_EL × 0.3
heat        / space_heating = HP_EL × 3.5     ← derived (SCOP), outside every total
```

`total` counts `HP_EL` once at weight 1, the two physical claims sum to exactly 1, so
`unallocated` is 0.

Two invariants keep propagation sound (§5.3):

- **All claims naming a sensor are declared on one node.** Splitting them across a node
  and its ancestor is arithmetically consistent but reports nonsense at the descendant:
  claims travel up only, so the lower node would show the ancestor's share as
  `unallocated` when it is in fact allocated. (Siblings cannot both reference a sensor —
  the subtree rule (§3.5) already means any two nodes referencing the same sensor are in
  an ancestor/descendant relationship, which is exactly the double-count case.)
- **A sensor's physical coefficients for one `energy_type` sum to at most 1.** You cannot
  allocate more of a sensor than it measured. Derived claims are a different `energy_type`
  and sit outside the sum; the bimåler pattern sums to 0 for the submeter's sensor (`−1`
  in space heating, `+1` in DHW) and 1 for the main.

### 3.7 Physical vs derived claims

A formula is **derived** iff its declared output `energy_type` differs from the
`energy_type` of any sensor it references directly. This is computable from the data
with no extra user input.

| Kind | Example | Part of `total`/`unallocated`? |
|---|---|---|
| Physical | `electricity / cooling = CHILLER_EL × 1` | yes |
| Physical | `district_heating / space_heating = MAIN × 1 + DHW × −1` | yes |
| Derived | `heat / space_heating = GAS_M3 × 10.45` | no |
| Derived | `district_cooling / cooling = CHILLER_EL × 3.2` | no |

Derived rows are a *delivered-energy* view, not a metered consumption, so folding them
into a physical total would double count. They still roll up the ancestor chain like any
other purpose series.

### 3.8 What `total` sums — nesting and outflow

`total` is not a blind Σ. Two facts change a sensor's weight, and **neither is a
formula** — both are properties of the installation, which is why they belong on the
data rather than in a per-node override that someone can forget to declare.

**(a) Overlapping readings — `Sensor.contained_in`.** Sensors whose readings overlap are
the norm, in two shapes. *Same node:* a chiller's accumulating channel already covers its
three phase channels — registers on one device, so they hang off one node (the hierarchy
never nests a meter node inside a meter node; node types come from the company schema).
*Across nodes:* a tenant submeter on its own area is already covered by the building's
main sensor one level up.

**A sensor attaches to the node whose consumption it measures — not where the device is
installed.** A main meter bolted in building B1's basement but covering the whole property
hangs off the **property** node. Physical location is not a modelling input, and getting
this wrong is silent: attach that main to B1 and B1's total claims the entire property's
consumption while B2's is double counted above. This is why the covering sensor must be at
or above the covered one (§5.3) — a sibling's subtree cannot contain the covered sensor, so
allowing it would produce wrong numbers rather than an error. A blind Σ reads 80 kWh instead of
40, and 155 kWh of heat instead of 120.

The fact is *"this sensor's reading is already included in that one's"*, so it is
recorded once, on the sensor:

```
Sensor cL1  contained_in = ACC      (phase channel, covered by the accumulating channel)
Sensor DHW  contained_in = HM1      (submeter, covered by the main heat sensor)
```

**Rule:** a covered sensor contributes **0** to `total` at any node where its
container is also present, and **1** where it is not. The container must be attached to
the contained sensor's own node or to an ancestor of it (§5.3), so the weight is 1 at
nodes strictly below the container and 0 from the container's node upward. That is the
correct answer at every level for free: the tenant's own area legitimately reports what
the tenant used, while the building counts that energy once via the main sensor. Where
both sensors sit on the same node — the chiller's channels — the weight is simply 0
everywhere, which is the common case.

Because the overlap is structural, **double counting stays structurally impossible** —
the property today's flat explode has, and the one a per-node weight override would have
surrendered. `Purpose::Total` therefore stays reserved: there is nothing left for a
`total` formula to express.

**(b) Direction — `Sensor::flow`.** Import and export are always *separate sensors* — two
registers on a bidirectional device, or two channels in a CSV — so the minus sign has to
live somewhere. Zeroing an export sensor is not enough: it only works when export is the
site's only PV channel, and breaks the moment a production sensor exists.

| Sensors | Correct answer | Zeroing export | Signed Σ |
|---|---|---|---|
| import 50, export 30 | 20 net grid | 50 | **20** ✓ |
| import 50, production 100, export 30 | 120 consumption | 150 ✗ | **120** ✓ |

So an `Out` sensor contributes **−1**, at its own node and every ancestor. Unlike
overlap this is absolute, not relational: exported energy leaves the site everywhere.

Together:

```
total(node, energy_type)        = Σ over descendant sensors of that energy type
                                    ×  0 if contained by a sensor present under `node`
                                    × −1 if flow is Out
                                    × +1 otherwise
unallocated(node, energy_type)  = total − Σ(claims whose `allocates` flag is set)
```

**`total` therefore means net energy across the node's boundary.** Where production is
metered that equals consumption; where it is not, it equals the net grid position — and
consumption is simply not recoverable without a production sensor, so the model does not
pretend otherwise.

`Purpose::Generation` stops being magic: it is now an ordinary reporting purpose (“how
much did we export”) whose sign is already carried by the sensor. It keeps exactly one
special property — it does not reduce `unallocated`, because exported energy is not a
slice of consumption.

With these rules the building's heat partitions exactly — `space_heating` 85 + `dhw` 35 =
`total` 120, `unallocated` 0 — and the chiller's `cooling` 40 equals its `total` 40.

---

## 4. Storage — `hierarchy_new`

### 4.1 Formula items

Formulas are **first-class items**, not an attribute on the node. This mirrors the
existing `has_sensor` edge (same partition as the node) and the sensor GSI.

| Field | Value |
|---|---|
| `pk` | `<NodeId>` — e.g. `HN5#10042` (same partition as the node item) |
| `sk` | `formula#<energy_type>#<purpose>` |
| `gsi1pk` | `F#HN2#<company_id>` |
| `gsi1sk` | `<node_path>#<energy_type>#<purpose>` |
| `terms` | `L` of `M{ ref: S, coefficient: N }` |
| `note` | `S`, optional |
| `updated` | `S` (RFC 3339) |

Node and sensor items are unchanged apart from the `purpose` → `energy_type` rename and
the new optional `contained_in` attribute on sensors. Deleting a node cascades its
formula items — they are in the node's own partition, which the existing delete path
already enumerates.

### 4.2 Materialised weight matrix

The output of `flatten` (§3.4) is stored as derived items, recomputed by the hierarchy
service whenever the matrix can change (§5.2). **This is what the Glue job reads** — the
domain rules stay in `crates/model` and exist exactly once.

| Field | Claim row | Total-override row |
|---|---|---|
| `pk` | `HN2#<company_id>` | `HN2#<company_id>` |
| `sk` | `weight#claim#<declaring_node>#<energy_type>#<purpose>#<sensor>` | `weight#total#<node_path>#<energy_type>#<sensor>` |
| `gsi1pk` | `W#HN2#<company_id>` | `W#HN2#<company_id>` |
| `kind` | `"claim"` | `"total"` |
| `sensor_id` | `N` | `N` |
| `coefficient` | `N` | `N` |
| `energy_type`, `purpose`, `declaring_node`, `derived`, `allocates` | populated | absent |

One GSI query per company returns the whole matrix.

**Staleness** is the cost of materialisation. Recompute runs inside the command handler,
so a failure surfaces as a command error rather than silent drift, and a
`rebuild_company_matrix` command exists as the escape hatch.

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

- `set_node_formula` — `{ node, energy_type, purpose, terms: [{ref, coefficient}], note? }`
  (upsert on `(node, energy_type, purpose)`)
- `delete_node_formula` — `{ node, energy_type, purpose }`
- `rebuild_company_matrix` — `{ company }` (operator escape hatch)

`attach_sensor` and `replace_sensor_device` gain an optional `contained_in` and `flow`.
Because neither can be inferred from the device (§3.3), both are prompted for on the
add-sensor form rather than hidden behind an advanced section — a forgotten `flow: Out`
or `contained_in` silently inflates every total above that sensor.

**Matrix recompute** runs after any command that can change the flattened matrix:
`set_node_formula`, `delete_node_formula`, `attach_sensor`, `replace_sensor_device`,
`delete_sensor`, `add_node`, `delete_node`. (`update_node` touches only metadata.) The
handler loads the company graph, calls `flatten`, and replaces that company's `W#HN2#…`
rows.

Query:

- `GET /hierarchy/query/node_formulas?node=<NodeId>` → HTML fragment

UI: a **"Formler"** tab on the node panel. One card per formula, headed by its
`(energy_type, purpose)`, with term rows of `[reference ▾] × [coefficient]` and a note
field. The reference picker offers only the node's descendants, so the subtree rule is
enforced in the UI as well as in the command. The add-sensor form gains a **"Sidder
allerede i"** select listing sensors on the same node or an ancestor. HTML-over-the-wire,
server-rendered maud.

### 5.3 Validation

| Rule | Failure |
|---|---|
| `coefficient` is finite and non-zero | 400 |
| `purpose` is declarable (not `total`, not `unallocated`) | 400 |
| every `reference` is a descendant of the declaring node | 400 |
| at most one formula per `(node, energy_type, purpose)` | upsert, not an error |
| every claim naming a sensor is declared on the **same node** | 409 with the conflicting node |
| a sensor's physical coefficients for one `energy_type` sum to ≤ 1 | 400 with the running total |
| `contained_in` names a sensor on the same node or an ancestor, of the same energy type | 400 — the message must name the fix: if the covering sensor measures a wider scope, move **it** up to the node it actually covers, rather than moving the submeter |
| `contained_in` does not form a cycle | 400 |

---

## 6. Cross-account bridge — `infra/hierarchy/app.go`

The inlined Python `_item` builder:

- **drops** the `formula` copy entirely;
- **renames** `purpose` → `energy_type` and `meter_type` → `reading_kind` in the
  `sensor-identity` item.

The **`meter-identity` table is replaced by `sensor-identity`** (§2.1). DynamoDB tables
cannot be renamed, so this is a new table with a new stream: the Flink event-source
mapping is repointed and the app re-bootstraps, and the bridge lambda, its DLQ and its
alarm lose the `meter-identity` name. The data migration is trivial — the bridge
repopulates the table from `hierarchy_new` stream events, so re-saving each sensor is
enough.

The bridge does not carry formulas or the weight matrix — the Glue job reads
`hierarchy_new` directly (D6). `contained_in` also stays hierarchy-side; the pipeline
never needs it, because the matrix already encodes its effect.

**New CDK resource:** a `HierarchyReaderRole` in account `339712745226`, trusting the
Glue job role in `891377204778`, granting `dynamodb:Query` on `hierarchy_new` and its
`gsi1` index. (The mirror of the existing `OcamlBridgeWriterRoleStack`.)

---

## 7. Flink + Iceberg — `infra/daq/data_pipeline`

- `MeterMapping` → `SensorMapping`, `.purpose` → `.energyType` and `.meterType` →
  `.readingKind`, plus `DdbBootstrapLoader`, `DdbStreamDeserializer`,
  `MeterEnrichmentFunction` and `Main.scala`'s table schema.
- The `purpose` column in **`all.raw_data`** and **`all.logical_data`** renames to
  `energy_type`, and the table itself to **`all.logical_data`** — free, because the
  column change already forces a delete/recreate.

Per the documented gotcha, `AWS::S3Tables::Table` cannot be replaced in place: this is a
**two-step deploy** — remove the table resource from `s3tables_stack.go` and deploy (CFN
deletes it, clearing the data), then restore it with the new column and deploy again.

The operator `uid` and keyed-state descriptor names are untouched, so the Flink snapshot
still restores; only the sink schema changes.

**Accepted data loss:** `raw_data` and `logical_data` history is cleared. Kinesis
retention is 24 h, so roughly one day is recoverable by replay.

---

## 8. Roll-up job — `glue/measurements_aggregate.py`

### 8.1 Keys

```
sk      = <node_path>#<energy_type>#<purpose>#<gran>#<bucket>
gsi1pk  = HN2#<id>#<dimension>#<purpose>
gsi1sk  = <node_path>#<gran>#<bucket>
```

The bucket stays last, so a fixed `(energy_type, purpose)` is still a pure `BETWEEN`
key-range — the common read path costs exactly what it does today. `purpose` enters
`gsi1pk` rather than `gsi1sk` so the cross-type dimension view cannot silently sum
`total` together with its own purpose breakdown.

### 8.2 Stages

1. **Matrix load.** For each distinct `hn2` in the window, the driver assumes
   `HierarchyReaderRole` and issues **one** GSI query (`gsi1pk = "W#HN2#<id>"`), then
   builds two DataFrames from the rows. No tree walking, no formula evaluation — the
   rows are already flat (§4.2).
2. **`total` rows.** `ancestor_keys` explode as today, left-joined to `total_overrides`
   on `(node_path, sensor_id)` with a default weight of 1, summing
   `resample_value × weight`.
3. **Declared purposes.** Join `claims` on `logical_id`, explode `declaring_node` into
   its ancestor paths (§3.6), then
   `groupBy(node_path, energy_type, purpose, gran, bucket).sum(resample_value × coefficient)`.
4. **`unallocated`.** Per `(node_path, energy_type, gran, bucket)`:
   `total − Σ(claims whose `allocates` flag is set)`. The flag is computed in
   `crates/model` as `!derived && !purpose.is_outflow()`, so the job filters on a
   boolean and never on a purpose name — the last place a rule could have leaked
   into PySpark.
5. **Write.** Unchanged `foreachPartition` + `batch_writer(overwrite_by_pkeys=["pk","sk"])`.

### 8.3 Stored attributes

`min`, `max` and `last_value` are **dropped** — the read side never reads them (only
`sum` and `count` are consumed), and under weighting and subtraction they are
meaningless. `count` stays as a coverage signal.

### 8.4 No duplicated semantics

Because the job reads the materialised matrix rather than raw formulas, it holds none of
the flattening rules — no subtree walk, no containment logic, no direction handling, no
derived detection, no node-reference expansion, and not even the rule for which claims
reduce `unallocated` (that arrives as the `allocates` flag). `hierarchy_matrix.py` is a GSI query and two
`createDataFrame` calls. There is one implementation of the domain, in `crates/model`,
and therefore nothing to keep in parity.

---

## 9. Read side — `crates/services/aggregations`

- `parse_sk` gains a segment; `Row.purpose` finally means purpose, and a new
  `Row.energy_type` carries the carrier.
- `query_one_resource`'s prefix becomes `<sk_path>#<energy_type>#<purpose>#<gran>#`,
  defaulting to `purpose=total` — so every existing widget keeps its current meaning at
  its current cost.
- New `?purpose=` filter on `get_aggregations`; "all purposes for one energy type" fans
  out over the declared purposes concurrently, the same shape as today's fan-out over
  the six energy types.
- New `get_purpose_split` action backing an end-use breakdown widget.
- `query_dimension` passes `purpose` into `gsi1pk` (callers pass `total`).
- `tariff_dkk_per_unit` and `emission_kg_per_unit` take `(energy_type, purpose)` with an
  energy-type-only fallback; `generation` returns 0 — this is what replaces the demo's
  per-sensor `f`.
- `get_alarms` and `get_benchmark` pin to `purpose=total`; a median-spike test and an
  area-normalised peer comparison are both meaningless on a subtractive series.

---

## 10. Accepted properties

Consequences of the chosen options, documented rather than fixed:

- **Negative sums are expected** (`total − DHW`, `import − export`). Cost and CO₂ on a
  negative series correctly yield a credit.
- **Recomputes restate history with today's formulas.** D6 reads the live matrix, so
  widening `LookbackDays` after a formula edit rewrites past buckets. A versioned
  snapshot would have avoided this and was not chosen.
- **A missing submeter inflates a subtractive purpose** rather than under-reporting it —
  the opposite of today's fail-safe behaviour. `count` is the signal that a bucket is
  partial.
- **`total` can disagree with Σ(purposes)** when a node is only partly claimed (the gap
  is `unallocated`). `total` is authoritative for cost, CO₂, alarms and benchmark.
- **The materialised matrix can go stale** if a recompute is skipped by a code path that
  should have triggered one. Recompute lives in the command handler and
  `rebuild_company_matrix` is the manual repair.
- **Unrecorded overlap or direction still corrupts totals.** `contained_in` and `flow`
  make the *model* sound, but neither is inferable from the stream (§3.3) — a missing
  value silently inflates every total above that sensor. This is an
  onboarding-data problem, not a modelling one, and the add-sensor form is the only
  mitigation.
- **A node's consumption cannot be derived by difference from an ancestor.** "Building B1
  has no meter, so B1 = property main − B2's submeter" is a natural ask and the model
  refuses it three times over: the subtree rule (§3.5) stops B1 naming a sibling's sensor;
  `total` is not declarable, so no formula can set what B1 measured; and the quantity is
  not B1 anyway — it is B1 *plus* common areas, plant and losses. That number is already
  emitted, correctly labelled, as **`unallocated` at the property**. If it must be
  attributed to B1, that is an *allocation* — a purpose claim with a coefficient — not a
  total. Totals stay facts.
- **A sensor's coverage must correspond to a node.** A main sensor covering buildings B1
  and B2 but not B3 has nowhere to attach unless the schema has a node grouping exactly
  those two. Same family as the limitation below: the tree has to be able to express the
  grouping before a sensor can be scoped to it.
- **Shared plant cannot be apportioned across siblings.** The subtree rule (§3.5) means a
  node may only reference its own descendants, so a chiller sensor hanging off a property
  and serving two buildings 60/40 can only be claimed at the property — neither building
  can take a share. This is a real EMS requirement the model forecloses; lifting it would mean
  allowing cross-subtree references, which reintroduces cycles and unsafe reparenting.
- **Coefficients are time-invariant.** A reversible heat pump heats in January and cools
  in July, but `electricity/space_heating = HP × 0.7` applies the same split to every
  hour; the handbook's GUF 28 % has the same character. Time-varying coefficients would
  break the linearity (D2) that lets evaluation commute with bucketing, so this is a
  deliberate limit, not an oversight.
- **Iceberg history is cleared** by the column rename (§7).

---

## 11. Testing

| Layer | Coverage |
|---|---|
| `model` domain | `Purpose` and `EnergyType` wire-token round-trip; reserved purposes rejected; `Term`/`NodeFormula` construction |
| `model` logic | `flatten`: nested coefficient multiplication, override-only defaults, subtree-rule rejection, derived detection, claim propagation to ancestors |
| `model` logic (§3.8) | overlap yields 0 for channels sharing a node, and 0 at/above but 1 below for a tenant submeter under a building main; `flow: Out` yields −1 everywhere; the three-sensor PV case nets to 120 and the two-sensor case to 20; the handbook cases partition exactly (`space_heating + dhw = total`, `unallocated = 0`) |
| `model` logic (§3.6) | one sensor across several purposes on one node is accepted; claims split across a node and its ancestor are rejected; coefficients summing past 1 are rejected; a derived claim of a different `energy_type` does not count toward the sum |
| `model` repository | formula + weight-row codec round-trip; `F#HN2#…` and `W#HN2#…` GSI partition queries; cascade delete with the node |
| `hierarchy` api | `set_node_formula` / `delete_node_formula` happy paths and every validation failure; `writes`-edge gating; matrix recompute fires on each triggering command; `node_formulas` fragment |
| `hierarchy` html | Formler tab renders terms; reference picker lists only descendants; `contained_in` select lists only same-node/ancestor sensors |
| Glue | `build_rollups` with a matrix: containment weight 0, subtraction, apportionment, `unallocated` arithmetic, derived rows excluded from totals, new sk/gsi1 shapes |
| `aggregations` | `parse_sk` with the new segment; `purpose=total` default; `?purpose=` filter; `(energy_type, purpose)` factor lookup incl. `generation → 0` |

---

## 12. Deploy order

1. `crates/model` + `hierarchy` lambda (formula and weight items readable/writable before
   anything reads them), then `rebuild_company_matrix` for every company.
2. `HierarchyReaderRole` (account `339712745226`).
3. Bridge (`purpose` → `energy_type`, `meter_type` → `reading_kind`, and the
   `meter-identity` → `sensor-identity` table replacement) — must land **with** step 4,
   since the pipeline has no fallback.
4. Flink + S3 Tables two-step column rename (account `891377204778`).
5. Glue roll-up job.
6. `aggregations` lambda.
7. Frontend rebuild + deploy (`PUBLIC_*` are baked in at build time).

Wipe `measurements_aggregate` and re-run the Glue job with a wide `LookbackDays` over
whatever raw data survives step 4 — the sort key gained a segment, so old rows are
unreadable.

---

## 13. Out of scope

- Graddagekorrigering (weather normalisation of the GAF part) — a per-`(node, purpose)`
  read-side concern, not a formula term.
- Formula versioning / point-in-time restatement (see §10).
- Non-linear operators (`abs`, division, min/max).
- Inferring reading overlap or flow direction from the data.
