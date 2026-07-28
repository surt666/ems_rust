# Node-Formula Roll-up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move consumption formulas off sensors and onto hierarchy nodes, splitting the single mis-named `purpose` axis into `energy_type` (what a sensor measures) and `purpose` (what the energy is spent on), and make the Glue roll-up evaluate those formulas.

**Architecture:** A sensor becomes a raw value carrying its `EnergyType` and, where they apply, two facts about the installation — which other sensor's reading already includes its own, and which way energy flows through it. Hierarchy nodes hold `NodeFormula` items declaring an `(energy_type, purpose)` output as weighted linear terms over their own descendants. `crates/model` owns the flattening into a weight matrix; the hierarchy service **materialises** that matrix into `hierarchy_new`, and the Glue job reads the flat rows cross-account and joins them to `logical_meter_data`. Glue holds no formula semantics at all. Purely linear terms mean evaluation commutes with hour/day bucketing, so the roll-up keeps its single explode + groupBy shape.

**Tech Stack:** Rust (workspace: `model`, `api`, `services/hierarchy`, `services/aggregations`), maud + HTMX server-rendered HTML, DynamoDB (`hierarchy_new`, `measurements_aggregate`), Scala/Flink on MSF, Iceberg S3 Tables, PySpark on Glue, Go CDK, Astro frontend.

**Spec:** `docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md` — read it before starting. `docs/hierarchy-presentation.html` is a runnable model of the arithmetic; its numbers are the acceptance values used throughout this plan.

## Global Constraints

- Work directly on `main`. Do **not** create feature branches. Do **not** `git push` — commit locally only.
- This is a **dev system**: rename outright, no read-fallbacks, no dual-writes, no back-compat shims. Data loss is acceptable when chosen deliberately.
- Two AWS accounts, both `eu-central-1`: hierarchy/frontend `339712745226` (profile `stel-sb`), DAQ/pipeline `891377204778` (profile `daq_dev`).
- `unset GOROOT` before **every** `cdk` command. Always `cdk diff` before `cdk deploy`.
- Never lead a chained shell command with `pkill`/`pgrep` or anything whose non-zero exit is normal — it aborts the rest of the chain under `set -e` semantics.
- Rust: `cargo test` from the repo root must pass; `cargo clippy` must be warning-free.
- Wire tokens are lower-case `snake_case` and are keyed verbatim into DynamoDB sort keys. `Display` is the storage contract.
- UI is HTML-over-the-wire (HTMX). Never introduce client-side JSON rendering.
- CSS uses **grid**, never flexbox.
- Naming: the type is `EnergyType`, the wire token and field name is `energy_type`, the Danish UI label is *Energitype*.
- Vocabulary: the system's inputs are **sensors** (one per device channel/register). A *meter* is a physical device, and exists in the hierarchy only as a **node type**. State every rule over sensors — an accumulating channel and its phase channels are usually registers on one device, so `contained_in` means "this sensor's reading is already included in that one's", never "this meter sits inside that meter".
- Roll-up sort key after this change: `<node_path>#<energy_type>#<purpose>#<gran>#<bucket>`. GSI: `gsi1pk = HN2#<id>#<dimension>#<purpose>`, `gsi1sk = <node_path>#<gran>#<bucket>`.

---

# Phase 1 — Domain + hierarchy service

Ships independently: formulas can be authored, validated, listed, rendered, and flattened into the materialised matrix. Nothing downstream reads them yet.

---

### Task 1: `EnergyType` rename and the `Purpose` value type

**Files:**
- Modify: `crates/model/src/domain/values.rs` (the `Resource` block at ~lines 226-298, plus its `mod tests` section)
- Modify: every `Resource` reference the compiler flags across `crates/`

**Interfaces:**
- Produces: `EnergyType` (renamed from `Resource`, same six variants and wire tokens, same `dimension()`); `Purpose` with `as_str() -> &'static str`, `all() -> impl Iterator<Item = Purpose>`, `declarable() -> bool`, `is_outflow() -> bool`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/model/src/domain/values.rs`:

```rust
    // ---- Purpose ------------------------------------------------------------

    /// `Display` emits the exact lower-case wire token for every variant — the
    /// `measurements_aggregate` sort-key contract — and round-trips via parse.
    #[test]
    fn purpose_wire_tokens_round_trip() {
        for p in Purpose::iter() {
            assert_eq!(p.to_string().parse::<Purpose>().unwrap(), p);
        }
        assert_eq!(Purpose::SpaceHeating.to_string(), "space_heating");
        assert_eq!(Purpose::Dhw.to_string(), "dhw");
        assert_eq!(Purpose::PlugLoads.to_string(), "plug_loads");
        assert_eq!(Purpose::Unallocated.to_string(), "unallocated");
    }

    /// `total` and `unallocated` are emitted by the roll-up job. Neither may be
    /// declared on a formula — nesting is a fact on the sensor, not an override.
    #[test]
    fn reserved_purposes_are_not_declarable() {
        assert!(!Purpose::Total.declarable());
        assert!(!Purpose::Unallocated.declarable());
        assert!(Purpose::Dhw.declarable());
        assert!(Purpose::Generation.declarable());
    }

    /// Generation is an outflow — its claims are removed from `total`, not added.
    #[test]
    fn purpose_outflow() {
        assert!(Purpose::Generation.is_outflow());
        assert!(!Purpose::Cooling.is_outflow());
    }

    /// The rename is a rename: same tokens, same dimensions, new type name.
    #[test]
    fn energy_type_keeps_the_resource_wire_contract() {
        for e in EnergyType::iter() {
            assert_eq!(e.to_string().parse::<EnergyType>().unwrap(), e);
        }
        assert_eq!(EnergyType::DistrictHeating.to_string(), "district_heating");
        assert_eq!(EnergyType::Electricity.dimension(), Dimension::Energy);
        assert_eq!(EnergyType::Water.dimension(), Dimension::Volume);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model purpose_ 2>&1 | tail -20`
Expected: FAIL — `cannot find type Purpose in this scope`.

- [ ] **Step 3: Rename `Resource` → `EnergyType`**

Rename the type and every use of it. **Do not blind-`sed`** — `resource` appears in unrelated contexts (`RepositoryError`, CDK `Resources:`, `boto3.resource`). Rename in `crates/model/src/domain/values.rs` first, then let `cargo build` point at each call site:

```bash
cargo build 2>&1 | grep -E '^error' | head -40
```

Rename the DynamoDB attribute, the JSON field and the query parameter from `purpose` to `energy_type` wherever they carry the energy type (sensor codec, `json.rs`, `html/forms.rs`, `command.rs`, `query.rs`, `dispatch.rs`).

- [ ] **Step 4: Implement `Purpose`**

Insert into `crates/model/src/domain/values.rs` immediately after the `impl EnergyType { … }` block:

```rust
// ---------------------------------------------------------------------------
// Purpose
// ---------------------------------------------------------------------------

/// The **formål** — what the energy is spent on. Independent of [`EnergyType`]
/// (the energitype a sensor physically measures): electricity serves lighting,
/// cooling and ventilation alike, and space heating can arrive as district
/// heating, gas or a heat pump. The taxonomy follows Energihåndbogen 2019's
/// chapters.
///
/// The string form (`strum` serialize, always lower-case) is the wire/storage
/// contract: it is the `<purpose>` segment of the `measurements_aggregate` sort
/// key and of the `formula#<energy_type>#<purpose>` key in `hierarchy_new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString, EnumIter, strum::IntoStaticStr)]
#[strum(ascii_case_insensitive)]
pub enum Purpose {
    #[strum(serialize = "space_heating")]
    SpaceHeating,
    #[strum(serialize = "dhw")]
    Dhw,
    #[strum(serialize = "ventilation")]
    Ventilation,
    #[strum(serialize = "cooling")]
    Cooling,
    #[strum(serialize = "lighting")]
    Lighting,
    #[strum(serialize = "plug_loads")]
    PlugLoads,
    #[strum(serialize = "ev_charging")]
    EvCharging,
    #[strum(serialize = "process")]
    Process,
    #[strum(serialize = "common")]
    Common,
    /// Egenproduktion (PV export). An **outflow**: removed from `Total` at every
    /// level, so tariffs and emission factors never bill exported energy.
    #[strum(serialize = "generation")]
    Generation,
    /// The default Σ series, emitted by the roll-up job. Not declarable —
    /// overlapping readings and flow direction are recorded on the sensor
    /// (`contained_in`, `flow`), not as per-node overrides.
    #[strum(serialize = "total")]
    Total,
    /// `total − Σ(claimed)`. Emitted by the roll-up job; never declarable.
    #[strum(serialize = "unallocated")]
    Unallocated,
}

impl Purpose {
    /// The canonical lower-case wire token (zero-alloc).
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Every purpose, in declaration order.
    pub fn all() -> impl Iterator<Item = Purpose> {
        Purpose::iter()
    }

    /// Whether a node formula may declare this purpose as its output.
    pub const fn declarable(self) -> bool {
        !matches!(self, Purpose::Total | Purpose::Unallocated)
    }

    /// Whether claims of this purpose leave the site rather than being consumed
    /// on it — such sensors contribute 0 to `Total` everywhere.
    pub const fn is_outflow(self) -> bool {
        matches!(self, Purpose::Generation)
    }
}
```

- [ ] **Step 5: Run the full suite**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A crates/
git commit -m "feat(model)!: rename Resource -> EnergyType; add the Purpose (formål) axis"
```

---

### Task 2: Node-formula types, sensor containment, delete sensor formulas

**Files:**
- Create: `crates/model/src/domain/node_formula.rs`
- Delete: `crates/model/src/domain/formula.rs`
- Modify: `crates/model/src/domain/mod.rs`, `crates/model/src/domain/sensor.rs:28-29`
- Modify: `crates/model/src/repository/dynamodb/codec.rs` (sensor item: drop `formula`, add `contained_in`)
- Modify: `crates/model/src/logic/sensors.rs` (delete `walk_refs_sync`, `has_cycle`, `set_formula`, `evaluate`; drop `formula` from `attach`, add `contained_in`)
- Test: `crates/model/src/domain/node_formula.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `Purpose`, `EnergyType` (Task 1), `NodeId`, `SensorId`
- Produces:
  - `Reference` — `enum { Sensor(SensorId), Node(NodeId) }`, with `Reference::parse(&str) -> Result<Reference, String>` and `Display`
  - `Term { reference: Reference, coefficient: f64 }`
  - `NodeFormula { node: NodeId, energy_type: EnergyType, purpose: Purpose, terms: Vec<Term>, note: Option<String> }`
  - `NodeFormula::sk(&self) -> String` → `"formula#<energy_type>#<purpose>"`
  - `Flow` — `enum { In, Out }`, `Default = In`, wire tokens `"in"` / `"out"` — the direction energy flows through a **sensor** (one register), not through a device
  - `Sensor.contained_in: Option<SensorId>`, `Sensor.flow: Flow`; `Sensor.formula` no longer exists
  - `sensor::parent_path(&Sensor) -> &str` — the sensor's path minus its trailing `|S#<id>` segment
  - `sensors::attach` loses `formula: Formula`, gains `contained_in: Option<SensorId>` and `flow: Flow`

- [ ] **Step 1: Write the failing tests**

Create `crates/model/src/domain/node_formula.rs` containing only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::Level;

    #[test]
    fn reference_parses_sensors_and_nodes() {
        assert_eq!(
            Reference::parse("S#20001").unwrap(),
            Reference::Sensor(SensorId::make(20001))
        );
        assert_eq!(
            Reference::parse("HN5#10042").unwrap(),
            Reference::Node(NodeId::make(Level::Hn5, 10042))
        );
        assert!(Reference::parse("nonsense").is_err());
    }

    #[test]
    fn reference_display_round_trips() {
        for s in ["S#20001", "HN5#10042", "HN2#997"] {
            assert_eq!(Reference::parse(s).unwrap().to_string(), s);
        }
    }

    /// The sort key is the storage contract for a formula item.
    #[test]
    fn formula_sk_is_energy_type_then_purpose() {
        let f = NodeFormula {
            node: NodeId::make(Level::Hn4, 30),
            energy_type: EnergyType::DistrictHeating,
            purpose: Purpose::SpaceHeating,
            terms: vec![],
            note: None,
        };
        assert_eq!(f.sk(), "formula#district_heating#space_heating");
    }

    /// The bimåler case from Energihåndbogen: DHW metered, space heating = main − DHW.
    #[test]
    fn terms_carry_signed_coefficients() {
        let f = NodeFormula {
            node: NodeId::make(Level::Hn4, 30),
            energy_type: EnergyType::DistrictHeating,
            purpose: Purpose::SpaceHeating,
            terms: vec![
                Term { reference: Reference::Sensor(SensorId::make(1)), coefficient: 1.0 },
                Term { reference: Reference::Sensor(SensorId::make(2)), coefficient: -1.0 },
            ],
            note: Some("bimåler, jf. bygningsreglementet".to_string()),
        };
        assert_eq!(f.terms.len(), 2);
        assert_eq!(f.terms[1].coefficient, -1.0);
    }
}
```

Add to `mod tests` in `crates/model/src/domain/sensor.rs`:

```rust
    /// The node a sensor hangs off — its path minus the trailing sensor segment.
    /// Containment and claim propagation both key off this.
    #[test]
    fn parent_path_strips_the_sensor_segment() {
        let s = make_sample();
        assert_eq!(parent_path(&s), "HN0#root|HN5#10042");
    }

    /// Both installation facts default to the common case: not covered, flowing in.
    #[test]
    fn installation_facts_default_to_the_common_case() {
        let s = make_sample();
        assert_eq!(s.contained_in, None);
        assert_eq!(s.flow, Flow::In);
    }

    /// `flow` is stored as a wire token like every other enum here.
    #[test]
    fn flow_wire_tokens_round_trip() {
        assert_eq!(Flow::In.to_string(), "in");
        assert_eq!(Flow::Out.to_string(), "out");
        assert_eq!("out".parse::<Flow>().unwrap(), Flow::Out);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model node_formula 2>&1 | tail -20`
Expected: FAIL — the module is not declared in `domain/mod.rs`, then `cannot find type Reference`.

- [ ] **Step 3: Implement the types**

Prepend to `crates/model/src/domain/node_formula.rs` (above the test module):

```rust
//! Node formulas — the replacement for the deleted per-sensor `Formula`.
//!
//! A node declares an `(energy_type, purpose)` output as a weighted linear
//! combination of **its own descendants**. Terms store only what differs from
//! the default weight of 1, so attaching a sensor always moves the numbers and
//! nothing is silently dropped.

use std::fmt;

use crate::domain::ids::{NodeId, SensorId};
use crate::domain::values::{EnergyType, Purpose};

/// What a term points at: a sensor, or a child node's result for the same
/// energy type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    Sensor(SensorId),
    Node(NodeId),
}

impl Reference {
    /// Parse `"S#<n>"` as a sensor, anything else as a node id.
    pub fn parse(s: &str) -> Result<Reference, String> {
        if s.starts_with("S#") {
            SensorId::parse(s).map(Reference::Sensor)
        } else {
            NodeId::parse(s).map(Reference::Node)
        }
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reference::Sensor(id) => write!(f, "{}", id),
            Reference::Node(id) => write!(f, "{}", id),
        }
    }
}

/// One weighted term. `coefficient` covers every case the model supports:
/// include = 1, subtract = −1, apportion = 0.28, COP = 3.2,
/// brændværdi × virkningsgrad = 10.45.
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub reference: Reference,
    pub coefficient: f64,
}

/// A node's declared output series.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeFormula {
    pub node: NodeId,
    pub energy_type: EnergyType,
    pub purpose: Purpose,
    pub terms: Vec<Term>,
    pub note: Option<String>,
}

impl NodeFormula {
    /// DynamoDB sort key within the node's own partition.
    pub fn sk(&self) -> String {
        format!("formula#{}#{}", self.energy_type, self.purpose)
    }
}
```

In `crates/model/src/domain/mod.rs`, replace `pub mod formula;` with `pub mod node_formula;`.

- [ ] **Step 4: Add the two installation facts to `Sensor`**

Neither is inferable from the channel — a survey of the live `raw_data` corpus found channel names to be opaque (`a04` spans 42 330 devices; `volume` carries `kWh` while `energy` carries `J`; nothing in the corpus encodes direction). Both are entered at onboarding.

In `crates/model/src/domain/values.rs`, add:

```rust
/// Which way energy flows through a sensor. Import, production and submeter
/// channels are `In`; grid export and PV feed-in are `Out` and contribute
/// negatively to `total`, which is therefore the net energy across a node's
/// boundary. A bidirectional device exposes both as separate sensors (OBIS
/// 1.8.0 and 2.8.0), so this is a per-channel fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default,
         strum::Display, strum::EnumString, EnumIter)]
#[strum(ascii_case_insensitive)]
pub enum Flow {
    #[default]
    #[strum(serialize = "in")]
    In,
    #[strum(serialize = "out")]
    Out,
}
```

In `crates/model/src/domain/sensor.rs`, delete the `use crate::domain::formula::Formula;` import and the two `formula` struct lines, then add:

```rust
    /// The sensor whose reading already includes this one's, if any — an
    /// accumulating channel over its phase channels (usually the same device),
    /// or a main heat sensor over a DHW submeter (a different one). A covered
    /// sensor contributes 0 to `total` at any node where its container is also
    /// present, and its own weight where it is not — see `logic::formulas`.
    #[builder(default)]
    pub contained_in: Option<SensorId>,

    /// Direction of flow. `Out` sensors (grid export, PV feed-in) contribute −1.
    /// Import and export are always separate sensors, so the sign has to live
    /// somewhere; on the sensor it is said once and is correct at every level.
    #[builder(default)]
    pub flow: Flow,
```

and, next to `parent_id`:

```rust
/// The path of the node a sensor hangs off — its own path minus the trailing
/// `|S#<id>` segment. Containment and claim propagation are both keyed by this.
pub fn parent_path(s: &Sensor) -> &str {
    match s.path.rfind(PATH_SEP) {
        Some(i) => &s.path[..i],
        None => &s.path,
    }
}
```

- [ ] **Step 5: Delete the sensor-formula machinery**

1. `rm crates/model/src/domain/formula.rs`
2. In `crates/model/src/logic/sensors.rs`: delete `walk_refs_sync`, `has_cycle`, `set_formula` and `evaluate` (and their tests); remove the `formula: Formula` parameter from `attach` and the post-allocation cycle check plus its rollback; add a `contained_in: Option<SensorId>` parameter.
3. In `crates/model/src/repository/dynamodb/codec.rs`: drop the `formula` attribute from `sensor_to_item`/`sensor_of_item`, and add the optional `contained_in` attribute (stored as `S`, e.g. `"S#12"`).
4. Fix the fallout the compiler points at in `crates/services/hierarchy` — Task 7 rewrites the UI properly; here just delete the formula plumbing so the workspace builds.

Run: `cargo build 2>&1 | tail -30`
Expected: clean build.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS. Any test still referencing `Formula` should be deleted, not adapted — sensor formulas are gone.

- [ ] **Step 7: Commit**

```bash
git add -A crates/
git commit -m "feat(model)!: node-formula types + Sensor.contained_in; delete sensor formulas"
```

---

### Task 3: Flattening — `logic/formulas.rs`

**Files:**
- Create: `crates/model/src/logic/formulas.rs`
- Modify: `crates/model/src/logic/mod.rs` (add `pub mod formulas;`)
- Test: `crates/model/src/logic/formulas.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `NodeFormula`, `Term`, `Reference`, `parent_path` (Task 2), `Purpose`, `EnergyType` (Task 1)
- Produces:
  - `CompanyGraph { nodes: Vec<Node>, sensors: Vec<Sensor>, formulas: Vec<NodeFormula> }`
  - `Claim { declaring_node: String, energy_type: EnergyType, purpose: Purpose, sensor: SensorId, coefficient: f64, derived: bool }`
  - `TotalOverride { node_path: String, energy_type: EnergyType, sensor: SensorId, coefficient: f64 }`
  - `Matrix { claims: Vec<Claim>, total_overrides: Vec<TotalOverride> }`
  - `flatten(&CompanyGraph) -> Matrix`
  - `total_weight_at(&CompanyGraph, &Sensor, node_path: &str) -> f64`
  - `is_derived(&CompanyGraph, &NodeFormula) -> bool`

**Semantics being implemented (spec §3.4–§3.8):**
- A term referencing a **sensor** emits one claim.
- A term referencing a **node** expands to every sensor under that node whose `energy_type` equals the formula's, each at `coefficient × total_weight_at(sensor, that node's path)`.
- `total_weight_at(s, N)` = 0 if `s.contained_in` is a sensor present under `N`; else −1 if `s.flow == Flow::Out`; else 1. So `total` is the **net energy across the node's boundary**.
- `total_overrides` is an **exception list**: only `(node, sensor)` pairs whose weight is not 1. Overlap contributes a **0** row per ancestor-or-self path of the *container's* node; `Flow::Out` contributes a **−1** row per ancestor-or-self path of the *sensor's own* node.
- `is_derived(f)` = any referenced **sensor**'s energy type differs from the formula's output. Node references never make a formula derived.

- [ ] **Step 1: Write the failing tests**

Create `crates/model/src/logic/formulas.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::Level;
    use crate::domain::node::Node;
    use crate::domain::sensor::Sensor;
    use crate::domain::values::MeterType;

    const CO: &str = "HN0#root|HN2#997";

    fn node(level: Level, id: u32, path: &str) -> Node {
        Node::builder()
            .id(NodeId::make(level, id))
            .name(format!("n{id}"))
            .path(path.to_string())
            .build()
    }

    fn sensor(id: u32, node_path: &str, et: EnergyType, inside: Option<u32>) -> Sensor {
        Sensor::builder()
            .id(SensorId::make(id))
            .daq_id(format!("daq{id}"))
            .path(format!("{node_path}|S#{id}"))
            .energy_type(et)
            .meter_type(MeterType::Counter)
            .contained_in(inside.map(SensorId::make))
            .build()
    }

    /// A sensor energy flows OUT through — grid export, PV feed-in.
    fn out_sensor(id: u32, node_path: &str, et: EnergyType) -> Sensor {
        Sensor { flow: Flow::Out, ..sensor(id, node_path, et, None) }
    }

    fn term(r: Reference, c: f64) -> Term {
        Term { reference: r, coefficient: c }
    }

    /// The chiller from the presentation: the accumulating channel (S#1) hangs
    /// off the chiller node; its three phase channels hang off a child panel node
    /// and record that the accumulator already covers them.
    fn chiller_graph() -> CompanyGraph {
        let chill = format!("{CO}|HN5#5");
        let panel = format!("{chill}|HN6#6");
        CompanyGraph {
            nodes: vec![
                node(Level::Hn2, 997, CO),
                node(Level::Hn5, 5, &chill),
                node(Level::Hn6, 6, &panel),
            ],
            sensors: vec![
                sensor(1, &chill, EnergyType::Electricity, None),
                sensor(2, &panel, EnergyType::Electricity, Some(1)),
                sensor(3, &panel, EnergyType::Electricity, Some(1)),
            ],
            formulas: vec![NodeFormula {
                node: NodeId::make(Level::Hn5, 5),
                energy_type: EnergyType::Electricity,
                purpose: Purpose::Cooling,
                terms: vec![term(Reference::Sensor(SensorId::make(1)), 1.0)],
                note: None,
            }],
        }
    }

    fn sens(g: &CompanyGraph, id: u32) -> Sensor {
        g.sensors.iter().find(|s| s.id == SensorId::make(id)).unwrap().clone()
    }

    #[test]
    fn uncontained_sensors_weigh_one() {
        let g = chiller_graph();
        assert_eq!(total_weight_at(&g, &sens(&g, 1), &format!("{CO}|HN5#5")), 1.0);
    }

    /// Overlap is RELATIONAL: the phase channels count 0 where the accumulating
    /// channel is also present (the chiller and above), and 1 where it is not
    /// (their own panel). Both answers are correct.
    #[test]
    fn containment_zeroes_only_where_the_container_is_present() {
        let g = chiller_graph();
        let chill = format!("{CO}|HN5#5");
        let panel = format!("{chill}|HN6#6");
        for id in [2u32, 3] {
            assert_eq!(total_weight_at(&g, &sens(&g, id), &panel), 1.0,
                       "phase {id} counts at its own panel");
            assert_eq!(total_weight_at(&g, &sens(&g, id), &chill), 0.0,
                       "phase {id} is already covered by the accumulator at the chiller");
            assert_eq!(total_weight_at(&g, &sens(&g, id), CO), 0.0,
                       "…and at every ancestor above it");
        }
    }

    /// A PV site: import, production and export are three separate sensors.
    /// Direction is ABSOLUTE — an `Out` sensor is negative at every level.
    fn pv_graph(with_production: bool) -> CompanyGraph {
        let area = format!("{CO}|HN5#7");
        let mut sensors = vec![
            sensor(10, &area, EnergyType::Electricity, None), // import  50
            out_sensor(12, &area, EnergyType::Electricity),   // export  30
        ];
        if with_production {
            sensors.push(sensor(11, &area, EnergyType::Electricity, None)); // production 100
        }
        CompanyGraph {
            nodes: vec![node(Level::Hn2, 997, CO), node(Level::Hn5, 7, &area)],
            sensors,
            formulas: vec![],
        }
    }

    #[test]
    fn flow_out_is_negative_at_every_level() {
        let g = pv_graph(true);
        for path in [format!("{CO}|HN5#7").as_str(), CO] {
            assert_eq!(total_weight_at(&g, &sens(&g, 10), path), 1.0, "import");
            assert_eq!(total_weight_at(&g, &sens(&g, 11), path), 1.0, "production");
            assert_eq!(total_weight_at(&g, &sens(&g, 12), path), -1.0, "export");
        }
    }

    /// Signed Σ gives the right answer for BOTH metering setups: true consumption
    /// when production is metered (50 + 100 − 30), the net grid position when it
    /// is not (50 − 30). Zeroing the export sensor would give 150 and 50.
    #[test]
    fn signed_total_handles_both_pv_setups() {
        let readings = |g: &CompanyGraph, path: &str| -> f64 {
            let vals = [(10u32, 50.0), (11, 100.0), (12, 30.0)];
            g.sensors
                .iter()
                .map(|s| {
                    let v = vals.iter().find(|(id, _)| *id == s.id.id()).unwrap().1;
                    v * total_weight_at(g, s, path)
                })
                .sum()
        };
        assert_eq!(readings(&pv_graph(true), CO), 120.0, "fully metered");
        assert_eq!(readings(&pv_graph(false), CO), 20.0, "no production sensor");
    }

    /// The exception list carries −1 for outflow, one row per ancestor of the
    /// sensor's own node.
    #[test]
    fn outflow_rows_are_minus_one_at_every_ancestor() {
        let m = flatten(&pv_graph(true));
        let area = format!("{CO}|HN5#7");
        for p in [area.as_str(), CO] {
            let row = m.total_overrides.iter()
                .find(|o| o.node_path == p && o.sensor == SensorId::make(12));
            assert_eq!(row.map(|o| o.coefficient), Some(-1.0), "export at {p}");
        }
        assert!(m.total_overrides.iter().all(|o| o.sensor == SensorId::make(12)),
                "import and production are the default weight 1, so no rows");
    }

    /// The exception list carries only what differs from 1, one row per node
    /// where the container is present.
    #[test]
    fn total_overrides_are_an_exception_list() {
        let m = flatten(&chiller_graph());
        let chill = format!("{CO}|HN5#5");
        let panel = format!("{chill}|HN6#6");
        let at = |p: &str, s: u32| {
            m.total_overrides.iter().any(|o| o.node_path == p && o.sensor == SensorId::make(s))
        };
        assert!(at(&chill, 2) && at(&chill, 3), "zeroed at the chiller");
        assert!(at(CO, 2) && at(CO, 3), "zeroed at the company");
        assert!(!at(&panel, 2), "NOT zeroed at their own panel");
        assert!(m.total_overrides.iter().all(|o| o.coefficient == 0.0));
        assert!(m.total_overrides.iter().all(|o| o.sensor != SensorId::make(1)));
    }

    #[test]
    fn flatten_emits_one_claim_per_sensor_term() {
        let m = flatten(&chiller_graph());
        assert_eq!(m.claims.len(), 1);
        let c = &m.claims[0];
        assert_eq!(c.sensor, SensorId::make(1));
        assert_eq!(c.coefficient, 1.0);
        assert_eq!(c.purpose, Purpose::Cooling);
        assert_eq!(c.declaring_node, format!("{CO}|HN5#5"));
        assert!(!c.derived);
        assert!(c.allocates, "a physical claim reduces unallocated");
    }

    /// The roll-up job filters `unallocated` on this boolean, so the rule for
    /// what counts as an allocation lives here and only here.
    #[test]
    fn derived_and_outflow_claims_do_not_allocate() {
        let mut g = chiller_graph();
        g.formulas.push(NodeFormula {
            energy_type: EnergyType::DistrictCooling, // differs from the sensor's
            purpose: Purpose::Cooling,
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 3.2)],
            ..g.formulas[0].clone()
        });
        g.formulas.push(NodeFormula {
            purpose: Purpose::Generation,
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 1.0)],
            ..g.formulas[0].clone()
        });
        let m = flatten(&g);
        let by = |p: Purpose, e: EnergyType| {
            m.claims.iter().find(|c| c.purpose == p && c.energy_type == e).unwrap()
        };
        assert!(by(Purpose::Cooling, EnergyType::Electricity).allocates);
        assert!(!by(Purpose::Cooling, EnergyType::DistrictCooling).allocates, "derived");
        assert!(!by(Purpose::Generation, EnergyType::Electricity).allocates, "outflow");
    }

    /// A node reference expands to that node's sensors of the same energy type,
    /// scaled by the term coefficient AND each sensor's weight at that node.
    #[test]
    fn node_reference_expands_to_weighted_descendants() {
        let mut g = chiller_graph();
        g.formulas.push(NodeFormula {
            node: NodeId::make(Level::Hn2, 997),
            energy_type: EnergyType::Electricity,
            purpose: Purpose::Process,
            terms: vec![term(Reference::Node(NodeId::make(Level::Hn5, 5)), 0.5)],
            note: None,
        });
        let m = flatten(&g);
        let process: Vec<_> = m.claims.iter().filter(|c| c.purpose == Purpose::Process).collect();
        // Only the accumulator survives: the phases weigh 0 at the chiller node.
        assert_eq!(process.len(), 1);
        assert_eq!(process[0].sensor, SensorId::make(1));
        assert_eq!(process[0].coefficient, 0.5);
    }

    /// Output energy type differing from the referenced sensors' marks the claim
    /// derived — gas m³ × brændværdi × virkningsgrad is delivered heat, not
    /// metered consumption, so it must never fold into a total.
    #[test]
    fn cross_type_formula_is_derived() {
        let b = format!("{CO}|HN4#8");
        let g = CompanyGraph {
            nodes: vec![node(Level::Hn2, 997, CO), node(Level::Hn4, 8, &b)],
            sensors: vec![sensor(20, &b, EnergyType::Gas, None)],
            formulas: vec![NodeFormula {
                node: NodeId::make(Level::Hn4, 8),
                energy_type: EnergyType::Heat,
                purpose: Purpose::SpaceHeating,
                terms: vec![term(Reference::Sensor(SensorId::make(20)), 10.45)],
                note: Some("brændværdi 11,0 kWh/m³ × virkningsgrad 0,95".to_string()),
            }],
        };
        assert!(is_derived(&g, &g.formulas[0]));
        assert!(flatten(&g).claims.iter().all(|c| c.derived));
    }

    #[test]
    fn same_type_formula_is_not_derived() {
        let g = chiller_graph();
        assert!(!is_derived(&g, &g.formulas[0]));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model formulas:: 2>&1 | tail -20`
Expected: FAIL — `cannot find type CompanyGraph`.

- [ ] **Step 3: Implement the flattener**

Prepend to `crates/model/src/logic/formulas.rs`:

```rust
//! Flattening node formulas into a weight matrix.
//!
//! Pure functions over an in-memory company graph — no effects, no repository
//! access. The hierarchy service materialises the output into `hierarchy_new`
//! (see `repository::dynamodb::weight`), and the Glue roll-up reads those flat
//! rows, so these rules exist in exactly one place.

use crate::domain::ids::{NodeId, SensorId};
use crate::domain::node::{Node, PATH_SEP};
use crate::domain::node_formula::{NodeFormula, Reference, Term};
use crate::domain::sensor::{parent_path, Sensor};
use crate::domain::values::{EnergyType, Purpose};

/// Everything under one company (HN2) needed to evaluate its formulas.
#[derive(Clone, Debug, Default)]
pub struct CompanyGraph {
    pub nodes: Vec<Node>,
    pub sensors: Vec<Sensor>,
    pub formulas: Vec<NodeFormula>,
}

/// One declared `(node, energy_type, purpose, sensor)` weight. The roll-up job
/// multiplies each sensor reading by `coefficient` and groups by the declaring
/// node's ancestor paths.
#[derive(Clone, Debug, PartialEq)]
pub struct Claim {
    pub declaring_node: String,
    pub energy_type: EnergyType,
    pub purpose: Purpose,
    pub sensor: SensorId,
    pub coefficient: f64,
    /// Output energy type differs from the referenced sensors' (§3.7).
    pub derived: bool,
    /// Whether this claim reduces `unallocated` — false for derived rows
    /// (delivered energy, not metered consumption) and for outflow purposes
    /// (exported energy is not a slice of consumption). Computed here so the
    /// roll-up job filters on a boolean instead of re-deriving the rule.
    pub allocates: bool,
}

/// A `(node, sensor)` pair whose weight in `total` is not the default 1.
#[derive(Clone, Debug, PartialEq)]
pub struct TotalOverride {
    pub node_path: String,
    pub energy_type: EnergyType,
    pub sensor: SensorId,
    pub coefficient: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Matrix {
    pub claims: Vec<Claim>,
    pub total_overrides: Vec<TotalOverride>,
}

impl CompanyGraph {
    fn node_path(&self, id: &NodeId) -> Option<&str> {
        self.nodes.iter().find(|n| &n.id == id).map(|n| n.path.as_str())
    }

    fn sensor(&self, id: SensorId) -> Option<&Sensor> {
        self.sensors.iter().find(|s| s.id == id)
    }
}

/// True when `descendant` is at or below `ancestor`.
fn is_at_or_under(descendant: &str, ancestor: &str) -> bool {
    descendant == ancestor || descendant.starts_with(&format!("{ancestor}{PATH_SEP}"))
}

/// `"A|B|C"` → `["A", "A|B", "A|B|C"]`.
fn ancestors_inclusive(path: &str) -> Vec<String> {
    let segs: Vec<&str> = path.split(PATH_SEP).collect();
    (1..=segs.len()).map(|i| segs[..i].join(PATH_SEP)).collect()
}

/// A sensor's weight in the `total` series **evaluated at `node_path`**
/// (spec §3.8): 0 where the sensor that already covers it is also present
/// (relational), −1 where energy flows out (absolute), else 1. `total` is
/// therefore the net energy across the node's boundary.
pub fn total_weight_at(g: &CompanyGraph, s: &Sensor, node_path: &str) -> f64 {
    let contained = s
        .contained_in
        .and_then(|c| g.sensor(c))
        .is_some_and(|container| is_at_or_under(parent_path(container), node_path));
    if contained {
        return 0.0;
    }
    match s.flow {
        Flow::Out => -1.0,
        Flow::In => 1.0,
    }
}

/// A formula is derived when its declared output energy type differs from the
/// energy type of any sensor it references directly. Node references resolve to
/// the formula's own energy type, so they never make it derived.
pub fn is_derived(g: &CompanyGraph, f: &NodeFormula) -> bool {
    f.terms.iter().any(|t| match &t.reference {
        Reference::Sensor(id) => g.sensor(*id).is_some_and(|s| s.energy_type != f.energy_type),
        Reference::Node(_) => false,
    })
}

/// Expand one term into `(sensor, coefficient)` pairs.
fn expand(g: &CompanyGraph, f: &NodeFormula, t: &Term) -> Vec<(SensorId, f64)> {
    match &t.reference {
        Reference::Sensor(id) => vec![(*id, t.coefficient)],
        Reference::Node(id) => match g.node_path(id) {
            None => vec![],
            Some(path) => g
                .sensors
                .iter()
                .filter(|s| s.energy_type == f.energy_type)
                .filter(|s| is_at_or_under(&s.path, path))
                .map(|s| (s.id, t.coefficient * total_weight_at(g, s, path)))
                .filter(|(_, c)| *c != 0.0)
                .collect(),
        },
    }
}

/// The exception list of non-default `total` weights.
///
/// Overlap yields a **0** row per ancestor-or-self path of the **container's**
/// node — exactly the nodes where both sensors are present, and below which the
/// covered sensor still counts normally. `Flow::Out` yields a **−1** row per
/// ancestor-or-self path of the **sensor's own** node, since exported energy
/// leaves the site at every level.
fn total_overrides(g: &CompanyGraph) -> Vec<TotalOverride> {
    let mut out = Vec::new();
    for s in &g.sensors {
        let mut push = |node_path: String, coefficient: f64| {
            out.push(TotalOverride {
                node_path,
                energy_type: s.energy_type,
                sensor: s.id,
                coefficient,
            })
        };
        if let Some(container) = s.contained_in.and_then(|c| g.sensor(c)) {
            for p in ancestors_inclusive(parent_path(container)) {
                push(p, 0.0);
            }
        }
        if s.flow == Flow::Out {
            // Nodes at/above the container already carry a 0 row, which wins —
            // a covered sensor is not double counted, whichever way it flows.
            let zeroed: Vec<String> = s
                .contained_in
                .and_then(|c| g.sensor(c))
                .map(|c| ancestors_inclusive(parent_path(c)))
                .unwrap_or_default();
            for p in ancestors_inclusive(parent_path(s)) {
                if !zeroed.contains(&p) {
                    push(p, -1.0);
                }
            }
        }
    }
    out
}

/// Flatten a company into the weight matrix the roll-up job consumes.
pub fn flatten(g: &CompanyGraph) -> Matrix {
    let mut claims = Vec::new();
    for f in &g.formulas {
        let Some(declaring_node) = g.node_path(&f.node) else {
            continue;
        };
        let derived = is_derived(g, f);
        let allocates = !derived && !f.purpose.is_outflow();
        for t in &f.terms {
            for (sensor, coefficient) in expand(g, f, t) {
                claims.push(Claim {
                    declaring_node: declaring_node.to_string(),
                    energy_type: f.energy_type,
                    purpose: f.purpose,
                    sensor,
                    coefficient,
                    derived,
                    allocates,
                });
            }
        }
    }
    Matrix { claims, total_overrides: total_overrides(g) }
}
```

Add `pub mod formulas;` to `crates/model/src/logic/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p model formulas:: 2>&1 | tail -20`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/model/src/logic/formulas.rs crates/model/src/logic/mod.rs
git commit -m "feat(model): flatten node formulas into a weight matrix"
```

---

### Task 4: Formula and containment validation

**Files:**
- Modify: `crates/model/src/logic/formulas.rs` (append `validate` + `validate_containment` and tests)

**Interfaces:**
- Consumes: `CompanyGraph`, `NodeFormula` (Task 3)
- Produces:
  - `validate(&CompanyGraph, &NodeFormula) -> Result<(), String>`
  - `validate_containment(&CompanyGraph, &Sensor) -> Result<(), String>`

- [ ] **Step 1: Write the failing tests**

Append inside the existing `mod tests` in `crates/model/src/logic/formulas.rs`:

```rust
    fn ok_formula() -> NodeFormula {
        NodeFormula {
            node: NodeId::make(Level::Hn5, 5),
            energy_type: EnergyType::Electricity,
            purpose: Purpose::Cooling,
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 1.0)],
            note: None,
        }
    }

    #[test]
    fn validate_accepts_a_well_formed_formula() {
        assert!(validate(&chiller_graph(), &ok_formula()).is_ok());
    }

    #[test]
    fn validate_rejects_reserved_purposes() {
        for p in [Purpose::Total, Purpose::Unallocated] {
            let f = NodeFormula { purpose: p, ..ok_formula() };
            let e = validate(&chiller_graph(), &f).unwrap_err();
            assert!(e.contains("roll-up job"), "got: {e}");
        }
    }

    #[test]
    fn validate_rejects_zero_and_non_finite_coefficients() {
        for (c, want) in [(0.0, "non-zero"), (f64::NAN, "finite")] {
            let f = NodeFormula {
                terms: vec![term(Reference::Sensor(SensorId::make(1)), c)],
                ..ok_formula()
            };
            assert!(validate(&chiller_graph(), &f).unwrap_err().contains(want));
        }
    }

    /// The subtree rule: a formula may only reference its own descendants. This
    /// is what makes the reference graph acyclic and reparenting safe.
    #[test]
    fn validate_rejects_reference_outside_the_subtree() {
        let mut g = chiller_graph();
        let other = format!("{CO}|HN5#99");
        g.nodes.push(node(Level::Hn5, 99, &other));
        g.sensors.push(sensor(77, &other, EnergyType::Electricity, None));
        let f = NodeFormula {
            terms: vec![term(Reference::Sensor(SensorId::make(77)), 1.0)],
            ..ok_formula()
        };
        assert!(validate(&g, &f).unwrap_err().contains("descendant"));
    }

    /// One sensor may feed MANY purposes — a heat pump's electricity channel
    /// splitting between space heating and DHW, with delivered heat alongside.
    /// All declared on the same node.
    #[test]
    fn validate_accepts_one_sensor_across_several_purposes() {
        let mut g = chiller_graph();
        g.formulas.push(NodeFormula {
            purpose: Purpose::SpaceHeating,
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 0.7)],
            ..ok_formula()
        });
        let dhw = NodeFormula {
            purpose: Purpose::Dhw,
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 0.3)],
            ..ok_formula()
        };
        assert!(validate(&g, &dhw).is_ok(), "0.7 + 0.3 = 1.0 on one node");
    }

    /// A derived claim is a different energy type, so it sits outside the sum.
    #[test]
    fn validate_ignores_derived_claims_in_the_coefficient_sum() {
        let g = chiller_graph();
        let scop = NodeFormula {
            energy_type: EnergyType::Heat,
            purpose: Purpose::SpaceHeating,
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 3.5)],
            ..ok_formula()
        };
        assert!(validate(&g, &scop).is_ok());
    }

    /// Claims naming one sensor must all live on ONE node. Split across a node and
    /// its ancestor the arithmetic is consistent, but the descendant reports the
    /// ancestor's share as `unallocated` — claims only travel up.
    #[test]
    fn validate_rejects_claims_split_across_nodes() {
        let g = chiller_graph(); // HN5#5 already claims S#1 for cooling
        let elsewhere = NodeFormula {
            node: NodeId::make(Level::Hn2, 997),
            purpose: Purpose::Lighting,
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 0.2)],
            ..ok_formula()
        };
        let e = validate(&g, &elsewhere).unwrap_err();
        assert!(e.contains("already claimed"), "got: {e}");
    }

    /// You cannot allocate more of a sensor than it measured.
    #[test]
    fn validate_rejects_coefficients_summing_past_one() {
        let mut g = chiller_graph();
        g.formulas.push(NodeFormula {
            purpose: Purpose::SpaceHeating,
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 0.8)],
            ..ok_formula()
        });
        let too_much = NodeFormula {
            purpose: Purpose::Dhw,
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 0.5)],
            ..ok_formula()
        };
        let e = validate(&g, &too_much).unwrap_err();
        assert!(e.contains("sum"), "got: {e}");
    }

    /// The bimåler pattern nets to 0 for the submeter's sensor and 1 for the
    /// main, so it must pass: −1 in space heating, +1 in DHW.
    #[test]
    fn validate_accepts_the_bimaaler_pattern() {
        let b = format!("{CO}|HN4#9");
        let mut g = CompanyGraph {
            nodes: vec![node(Level::Hn2, 997, CO), node(Level::Hn4, 9, &b)],
            sensors: vec![
                sensor(30, &b, EnergyType::DistrictHeating, None),     // main
                sensor(31, &b, EnergyType::DistrictHeating, Some(30)), // submeter
            ],
            formulas: vec![],
        };
        let base = NodeFormula {
            node: NodeId::make(Level::Hn4, 9),
            energy_type: EnergyType::DistrictHeating,
            purpose: Purpose::Dhw,
            terms: vec![term(Reference::Sensor(SensorId::make(31)), 1.0)],
            note: None,
        };
        assert!(validate(&g, &base).is_ok());
        g.formulas.push(base);
        let space = NodeFormula {
            purpose: Purpose::SpaceHeating,
            terms: vec![
                term(Reference::Sensor(SensorId::make(30)), 1.0),
                term(Reference::Sensor(SensorId::make(31)), -1.0),
            ],
            ..g.formulas[0].clone()
        };
        assert!(validate(&g, &space).is_ok(), "S#31 nets to 0, S#30 to 1");
    }

    /// Re-declaring the SAME (node, energy_type, purpose) is an upsert.
    #[test]
    fn validate_allows_upserting_the_same_formula() {
        let g = chiller_graph();
        assert!(validate(&g, &g.formulas[0].clone()).is_ok());
    }

    // ---- containment --------------------------------------------------------

    #[test]
    fn containment_accepts_a_container_on_an_ancestor_node() {
        let g = chiller_graph();
        assert!(validate_containment(&g, &sens(&g, 2)).is_ok());
    }

    #[test]
    fn containment_rejects_a_container_below_the_sensor() {
        let mut g = chiller_graph();
        // Flip it: make the accumulating channel claim to be covered by a phase
        // channel, which lives on a DEEPER node. The container must be at or above.
        let mut acc = sens(&g, 1);
        acc.contained_in = Some(SensorId::make(2));
        g.sensors[0] = acc.clone();
        assert!(validate_containment(&g, &acc).unwrap_err().contains("ancestor"));
    }

    #[test]
    fn containment_rejects_a_different_energy_type() {
        let mut g = chiller_graph();
        let chill = format!("{CO}|HN5#5");
        g.sensors.push(sensor(40, &chill, EnergyType::Water, None));
        let mut s = sens(&g, 2);
        s.contained_in = Some(SensorId::make(40));
        assert!(validate_containment(&g, &s).unwrap_err().contains("energy type"));
    }

    #[test]
    fn containment_rejects_cycles() {
        let mut g = chiller_graph();
        let chill = format!("{CO}|HN5#5");
        let mut a = sensor(50, &chill, EnergyType::Electricity, Some(51));
        let b = sensor(51, &chill, EnergyType::Electricity, Some(50));
        g.sensors.push(a.clone());
        g.sensors.push(b);
        a.contained_in = Some(SensorId::make(51));
        assert!(validate_containment(&g, &a).unwrap_err().contains("cycle"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model validate_ 2>&1 | tail -20`
Expected: FAIL — `cannot find function validate`.

- [ ] **Step 3: Implement validation**

Append to the non-test part of `crates/model/src/logic/formulas.rs`:

```rust
/// Validate a formula against its company graph. Returns a human-readable
/// message suitable for a 400/409 body.
pub fn validate(g: &CompanyGraph, f: &NodeFormula) -> Result<(), String> {
    if !f.purpose.declarable() {
        return Err(format!(
            "purpose {} is emitted by the roll-up job and cannot be declared",
            f.purpose
        ));
    }

    let Some(node_path) = g.node_path(&f.node) else {
        return Err(format!("node {} not found in this company", f.node));
    };

    for t in &f.terms {
        if !t.coefficient.is_finite() {
            return Err(format!("coefficient for {} must be finite", t.reference));
        }
        if t.coefficient == 0.0 {
            return Err(format!(
                "coefficient for {} must be non-zero — to exclude a covered sensor, \
                 record it as contained_in its container instead",
                t.reference
            ));
        }
        let ref_path = match &t.reference {
            Reference::Sensor(id) => g.sensor(*id).map(|s| s.path.clone()),
            Reference::Node(id) => g.node_path(id).map(str::to_string),
        };
        match ref_path {
            None => return Err(format!("{} not found in this company", t.reference)),
            Some(p) if !is_at_or_under(&p, node_path) || p == node_path => {
                return Err(format!(
                    "{} is not a descendant of {} — a formula may only reference its own subtree",
                    t.reference, f.node
                ))
            }
            Some(_) => {}
        }
    }

    // Every claim naming a sensor must be declared on ONE node. Splitting them
    // across a node and its ancestor is arithmetically consistent but reports
    // the ancestor's share as `unallocated` at the descendant, since claims only
    // travel up. Re-declaring the same (node, energy_type, purpose) is an upsert.
    for t in &f.terms {
        let Reference::Sensor(id) = &t.reference else {
            continue;
        };
        if let Some(other) = g.formulas.iter().find(|o| {
            o.node != f.node && o.terms.iter().any(|ot| ot.reference == t.reference)
        }) {
            return Err(format!(
                "sensor {} is already claimed by node {} — all of a sensor's claims must \
                 be declared on one node",
                id, other.node
            ));
        }
    }

    // A sensor's physical coefficients for one energy type sum to at most 1 —
    // you cannot allocate more of a sensor than it measured. Derived claims are a
    // different energy type and sit outside the sum.
    for t in &f.terms {
        let Reference::Sensor(id) = &t.reference else {
            continue;
        };
        let others: f64 = g
            .formulas
            .iter()
            .filter(|o| {
                o.energy_type == f.energy_type
                    && !(o.node == f.node && o.purpose == f.purpose) // this is an upsert
            })
            .flat_map(|o| &o.terms)
            .filter(|ot| ot.reference == t.reference)
            .map(|ot| ot.coefficient)
            .sum();
        let total = others + t.coefficient;
        if total > 1.0 + f64::EPSILON {
            return Err(format!(
                "sensor {} would be allocated {:.2}× its {} reading — coefficients for \
                 one energy type must sum to at most 1",
                id, total, f.energy_type
            ));
        }
    }

    Ok(())
}

/// Validate a sensor's `contained_in`: the covering sensor must measure the same
/// energy type, hang off the sensor's own node or an ancestor of it, and the
/// chain must not cycle. `flow` needs no validation — either direction is legal
/// on any sensor.
pub fn validate_containment(g: &CompanyGraph, s: &Sensor) -> Result<(), String> {
    let Some(container_id) = s.contained_in else {
        return Ok(());
    };
    if container_id == s.id {
        return Err("a sensor cannot cover itself".to_string());
    }
    let Some(container) = g.sensor(container_id) else {
        return Err(format!("sensor {container_id} not found in this company"));
    };
    if container.energy_type != s.energy_type {
        return Err(format!(
            "sensor {container_id} measures {} — a sensor can only be covered by one of \
             the same energy type ({})",
            container.energy_type, s.energy_type
        ));
    }
    if !is_at_or_under(parent_path(s), parent_path(container)) {
        return Err(format!(
            "sensor {container_id} must be attached to {}'s own node or an ancestor of it",
            s.id
        ));
    }
    // Walk the chain; a revisit is a cycle.
    let mut seen = vec![s.id];
    let mut cur = Some(container_id);
    while let Some(id) = cur {
        if seen.contains(&id) {
            return Err(format!("containment cycle through sensor {id}"));
        }
        seen.push(id);
        cur = g.sensor(id).and_then(|c| c.contained_in);
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p model 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/model/src/logic/formulas.rs
git commit -m "feat(model): validate formulas (subtree, one-claim) and containment"
```

---

### Task 5: Persist formulas and the materialised matrix

**Files:**
- Create: `crates/model/src/repository/dynamodb/node_formula.rs`, `crates/model/src/repository/dynamodb/weight.rs`
- Modify: `crates/model/src/repository/dynamodb/mod.rs`, `.../codec.rs`, `crates/model/src/repository/memory.rs`
- Test: `crates/model/src/repository/dynamodb/codec.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `NodeFormula` (Task 2), `Matrix`/`Claim`/`TotalOverride` (Task 3)
- Produces:
  - `codec::node_formula_to_item(&NodeFormula, node_path, company_path) -> Item` / `node_formula_of_item(&Item) -> Result<NodeFormula, CodecError>`
  - `codec::claim_to_item(&Claim, company_path) -> Item` / `codec::total_override_to_item(&TotalOverride, company_path) -> Item`
  - `codec::formula_gsi1pk(company_path) -> String` → `"F#HN2#<id>"`; `codec::weight_gsi1pk(company_path) -> String` → `"W#HN2#<id>"`
  - `node_formula::{put_node_formula, delete_node_formula, list_node_formulas, list_company_formulas}`
  - `weight::replace_company_matrix(client, table, company_path, &Matrix) -> Result<(), RepositoryError>`

**Item shapes (spec §4):**

| | Formula | Claim | Total override |
|---|---|---|---|
| `pk` | `<NodeId>` | `HN2#<id>` | `HN2#<id>` |
| `sk` | `formula#<energy_type>#<purpose>` | `weight#claim#<declaring_node>#<energy_type>#<purpose>#<sensor>` | `weight#total#<node_path>#<energy_type>#<sensor>` |
| `gsi1pk` | `F#HN2#<id>` | `W#HN2#<id>` | `W#HN2#<id>` |
| `gsi1sk` | `<node_path>#<energy_type>#<purpose>` | — | — |

- [ ] **Step 1: Write the failing codec tests**

Add to `mod tests` in `crates/model/src/repository/dynamodb/codec.rs`:

```rust
    #[test]
    fn node_formula_item_round_trips() {
        use crate::domain::node_formula::{NodeFormula, Reference, Term};
        use crate::domain::values::Purpose;

        let f = NodeFormula {
            node: NodeId::make(Level::Hn4, 30),
            energy_type: EnergyType::DistrictHeating,
            purpose: Purpose::SpaceHeating,
            terms: vec![
                Term { reference: Reference::Sensor(SensorId::make(1)), coefficient: 1.0 },
                Term { reference: Reference::Sensor(SensorId::make(2)), coefficient: -1.0 },
            ],
            note: Some("bimåler".to_string()),
        };
        let node_path = "HN0#root|HN1#1|HN2#997|HN3#3|HN4#30";
        let item = node_formula_to_item(&f, node_path, "HN0#root|HN1#1|HN2#997");

        let s = |k: &str| item.get(k).and_then(|v| v.as_s().ok()).map(String::as_str);
        assert_eq!(s("sk"), Some("formula#district_heating#space_heating"));
        assert_eq!(s("gsi1pk"), Some("F#HN2#997"));
        assert_eq!(
            s("gsi1sk"),
            Some("HN0#root|HN1#1|HN2#997|HN3#3|HN4#30#district_heating#space_heating")
        );
        assert_eq!(node_formula_of_item(&item).unwrap(), f);
    }

    #[test]
    fn weight_items_key_by_company_and_kind() {
        use crate::domain::values::Purpose;
        use crate::logic::formulas::{Claim, TotalOverride};

        let company = "HN0#root|HN1#1|HN2#997";
        let claim = Claim {
            declaring_node: "HN0#root|HN1#1|HN2#997|HN4#30".to_string(),
            energy_type: EnergyType::DistrictHeating,
            purpose: Purpose::Dhw,
            sensor: SensorId::make(21),
            coefficient: 1.0,
            derived: false,
            allocates: true,
        };
        let ci = claim_to_item(&claim, company);
        let s = |i: &Item, k: &str| i.get(k).and_then(|v| v.as_s().ok()).map(String::as_str);
        assert_eq!(s(&ci, "pk"), Some("HN2#997"));
        assert_eq!(s(&ci, "gsi1pk"), Some("W#HN2#997"));
        assert_eq!(s(&ci, "kind"), Some("claim"));
        assert_eq!(
            s(&ci, "sk"),
            Some("weight#claim#HN0#root|HN1#1|HN2#997|HN4#30#district_heating#dhw#S#21")
        );

        let ov = TotalOverride {
            node_path: "HN0#root|HN1#1|HN2#997|HN4#30".to_string(),
            energy_type: EnergyType::DistrictHeating,
            sensor: SensorId::make(21),
            coefficient: 0.0,
        };
        let oi = total_override_to_item(&ov, company);
        assert_eq!(s(&oi, "kind"), Some("total"));
        assert_eq!(
            s(&oi, "sk"),
            Some("weight#total#HN0#root|HN1#1|HN2#997|HN4#30#district_heating#S#21")
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model node_formula_item 2>&1 | tail -20`
Expected: FAIL — `cannot find function node_formula_to_item`.

- [ ] **Step 3: Implement the codec**

Append to `crates/model/src/repository/dynamodb/codec.rs`:

```rust
// ---------------------------------------------------------------------------
// Node formulas + materialised weight matrix
// ---------------------------------------------------------------------------

use crate::domain::node_formula::{NodeFormula, Reference, Term};
use crate::domain::values::Purpose;
use crate::logic::formulas::{Claim, TotalOverride};

fn company_segment(company_path: &str) -> &str {
    company_path
        .split(crate::domain::node::PATH_SEP)
        .find(|s| s.starts_with("HN2#"))
        .unwrap_or("HN2#0")
}

/// `gsi1pk = "F#HN2#<id>"` — one partition per company's formulas.
pub(crate) fn formula_gsi1pk(company_path: &str) -> String {
    format!("F#{}", company_segment(company_path))
}

/// `gsi1pk = "W#HN2#<id>"` — one partition per company's weight matrix, so the
/// roll-up job loads it in a single query.
pub(crate) fn weight_gsi1pk(company_path: &str) -> String {
    format!("W#{}", company_segment(company_path))
}

pub fn node_formula_to_item(f: &NodeFormula, node_path: &str, company_path: &str) -> Item {
    let terms: Vec<AttributeValue> = f
        .terms
        .iter()
        .map(|t| {
            let mut m = std::collections::HashMap::new();
            m.insert("ref".to_string(), s(t.reference.to_string()));
            m.insert("coefficient".to_string(), AttributeValue::N(t.coefficient.to_string()));
            AttributeValue::M(m)
        })
        .collect();

    let mut item: Item = std::collections::HashMap::new();
    item.insert("pk".to_string(), s(f.node.to_string()));
    item.insert("sk".to_string(), s(f.sk()));
    item.insert("gsi1pk".to_string(), s(formula_gsi1pk(company_path)));
    item.insert("gsi1sk".to_string(), s(format!("{node_path}#{}#{}", f.energy_type, f.purpose)));
    item.insert("energy_type".to_string(), s(f.energy_type.to_string()));
    item.insert("purpose".to_string(), s(f.purpose.to_string()));
    item.insert("terms".to_string(), AttributeValue::L(terms));
    if let Some(n) = &f.note {
        item.insert("note".to_string(), s(n.clone()));
    }
    item.insert("updated".to_string(), s(chrono::Utc::now().to_rfc3339()));
    item
}

pub fn node_formula_of_item(item: &Item) -> Result<NodeFormula, CodecError> {
    let get = |k: &str| {
        item.get(k)
            .and_then(|v| v.as_s().ok())
            .cloned()
            .ok_or_else(|| CodecError::from(format!("formula item missing {k}")))
    };
    let node = NodeId::parse(&get("pk")?).map_err(CodecError::from)?;
    let energy_type: EnergyType = get("energy_type")?
        .parse()
        .map_err(|_| CodecError::from("bad energy_type on formula item"))?;
    let purpose: Purpose = get("purpose")?
        .parse()
        .map_err(|_| CodecError::from("bad purpose on formula item"))?;

    let terms = item
        .get("terms")
        .and_then(|v| v.as_l().ok())
        .map(|l| {
            l.iter()
                .filter_map(|v| v.as_m().ok())
                .map(|m| {
                    let r = m.get("ref").and_then(|v| v.as_s().ok()).cloned().unwrap_or_default();
                    let c = m
                        .get("coefficient")
                        .and_then(|v| v.as_n().ok())
                        .and_then(|n| n.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    Reference::parse(&r).map(|reference| Term { reference, coefficient: c })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
        .map_err(CodecError::from)?
        .unwrap_or_default();

    Ok(NodeFormula {
        node,
        energy_type,
        purpose,
        terms,
        note: item.get("note").and_then(|v| v.as_s().ok()).cloned(),
    })
}

fn weight_base(company_path: &str, sensor: SensorId, coefficient: f64) -> Item {
    let mut item: Item = std::collections::HashMap::new();
    item.insert("pk".to_string(), s(company_segment(company_path).to_string()));
    item.insert("gsi1pk".to_string(), s(weight_gsi1pk(company_path)));
    item.insert("sensor_id".to_string(), AttributeValue::N(sensor.id().to_string()));
    item.insert("coefficient".to_string(), AttributeValue::N(coefficient.to_string()));
    item
}

pub fn claim_to_item(c: &Claim, company_path: &str) -> Item {
    let mut item = weight_base(company_path, c.sensor, c.coefficient);
    item.insert("kind".to_string(), s("claim".to_string()));
    item.insert(
        "sk".to_string(),
        s(format!(
            "weight#claim#{}#{}#{}#{}",
            c.declaring_node, c.energy_type, c.purpose, c.sensor
        )),
    );
    item.insert("declaring_node".to_string(), s(c.declaring_node.clone()));
    item.insert("energy_type".to_string(), s(c.energy_type.to_string()));
    item.insert("purpose".to_string(), s(c.purpose.to_string()));
    item.insert("derived".to_string(), AttributeValue::Bool(c.derived));
    item.insert("allocates".to_string(), AttributeValue::Bool(c.allocates));
    item
}

pub fn total_override_to_item(o: &TotalOverride, company_path: &str) -> Item {
    let mut item = weight_base(company_path, o.sensor, o.coefficient);
    item.insert("kind".to_string(), s("total".to_string()));
    item.insert(
        "sk".to_string(),
        s(format!("weight#total#{}#{}#{}", o.node_path, o.energy_type, o.sensor)),
    );
    item.insert("node_path".to_string(), s(o.node_path.clone()));
    item.insert("energy_type".to_string(), s(o.energy_type.to_string()));
    item
}
```

Reuse the file's existing `s(..)`, `Item` and `CodecError` helpers rather than redeclaring them.

- [ ] **Step 4: Run codec tests to verify they pass**

Run: `cargo test -p model _item 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Implement the adapters**

Create `crates/model/src/repository/dynamodb/node_formula.rs` with `put_node_formula`, `delete_node_formula`, `list_node_formulas` (query `pk = <NodeId>` + `begins_with(sk, "formula#")`) and `list_company_formulas` (query `gsi1` on `gsi1pk = codec::formula_gsi1pk(..)`), following the query/paginator style already used in `sensor.rs`.

Create `crates/model/src/repository/dynamodb/weight.rs`:

```rust
//! The materialised weight matrix (spec §4.2). Replaced wholesale per company —
//! it is derived data, so a delete-then-write is simpler and safer than a diff.

use aws_sdk_dynamodb::types::{AttributeValue, WriteRequest, DeleteRequest, PutRequest};
use aws_sdk_dynamodb::Client;

use crate::errors::RepositoryError;
use crate::logic::formulas::Matrix;
use crate::repository::dynamodb::codec;

const WEIGHT_SK_PREFIX: &str = "weight#";

/// Delete every weight row for the company, then write the new matrix.
pub async fn replace_company_matrix(
    client: &Client,
    table: &str,
    company_path: &str,
    m: &Matrix,
) -> Result<(), RepositoryError> {
    let existing = list_weight_keys(client, table, company_path).await?;
    let deletes = existing.into_iter().map(|(pk, sk)| {
        WriteRequest::builder()
            .delete_request(
                DeleteRequest::builder()
                    .key("pk", AttributeValue::S(pk))
                    .key("sk", AttributeValue::S(sk))
                    .build()
                    .expect("delete key"),
            )
            .build()
    });
    let puts = m
        .claims
        .iter()
        .map(|c| codec::claim_to_item(c, company_path))
        .chain(m.total_overrides.iter().map(|o| codec::total_override_to_item(o, company_path)))
        .map(|item| {
            WriteRequest::builder()
                .put_request(PutRequest::builder().set_item(Some(item)).build().expect("put item"))
                .build()
        });

    for chunk in deletes.chain(puts).collect::<Vec<_>>().chunks(25) {
        client
            .batch_write_item()
            .request_items(table, chunk.to_vec())
            .send()
            .await
            .map_err(|e| RepositoryError::Aws(format!("replace_company_matrix: {e:?}")))?;
    }
    Ok(())
}

async fn list_weight_keys(
    client: &Client,
    table: &str,
    company_path: &str,
) -> Result<Vec<(String, String)>, RepositoryError> {
    let rows = client
        .query()
        .table_name(table)
        .index_name("gsi1")
        .key_condition_expression("#pk = :pk")
        .expression_attribute_names("#pk", "gsi1pk")
        .expression_attribute_values(":pk", AttributeValue::S(codec::weight_gsi1pk(company_path)))
        .into_paginator()
        .items()
        .send()
        .collect::<Result<Vec<_>, _>>()
        .await
        .map_err(|e| RepositoryError::Aws(format!("list_weight_keys: {e:?}")))?;

    Ok(rows
        .iter()
        .filter_map(|i| {
            let pk = i.get("pk")?.as_s().ok()?.clone();
            let sk = i.get("sk")?.as_s().ok()?.clone();
            sk.starts_with(WEIGHT_SK_PREFIX).then_some((pk, sk))
        })
        .collect())
}
```

Add both modules to `crates/model/src/repository/dynamodb/mod.rs` and mirror the functions in `crates/model/src/repository/memory.rs`.

- [ ] **Step 6: Verify the workspace builds and tests pass**

Run: `cargo test -p model 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/model/src/repository/
git commit -m "feat(model): persist node formulas and the materialised weight matrix"
```

---

### Task 6: Formula commands and matrix recompute

**Files:**
- Modify: `crates/services/hierarchy/src/command.rs:135` (add three variants), `crates/services/hierarchy/src/dispatch.rs` (arms + handlers + recompute), `crates/services/hierarchy/src/repo_fns.rs` (closure factories), `crates/services/hierarchy/src/json.rs` (`formula_to_json`)
- Test: `crates/services/hierarchy/src/command.rs` (`mod tests`), `crates/services/hierarchy/src/dispatch.rs` (`mod tests`)

**Interfaces:**
- Consumes: `validate`, `flatten` (Tasks 3-4), `put_node_formula` / `delete_node_formula` / `list_company_formulas` / `replace_company_matrix` (Task 5)
- Produces:
  - `Command::SetNodeFormula { node_id, energy_type, purpose, terms: Value, note: Option<String> }`
  - `Command::DeleteNodeFormula { node_id, energy_type, purpose }`
  - `Command::RebuildCompanyMatrix { company }`
  - `dispatch::handle_set_node_formula`, `handle_delete_node_formula`, `handle_rebuild_company_matrix`
  - `dispatch::recompute_matrix(company_path, …) -> Result<(), RepositoryError>` — the shared helper every triggering command calls

**Wire format** — `terms` is accepted as a JSON array or a JSON-encoded string (the HTML form builds it client-side to avoid dynamic field names):

```
action=set_node_formula
node_id=HN4#30
energy_type=district_heating
purpose=space_heating
terms=[{"ref":"S#1","coefficient":1},{"ref":"S#2","coefficient":-1}]
note=bimåler, jf. bygningsreglementet
```

- [ ] **Step 1: Write the failing parse tests**

Add to `mod tests` in `crates/services/hierarchy/src/command.rs`:

```rust
    #[test]
    fn parse_set_node_formula_json() {
        let body = serde_json::json!({
            "action": "set_node_formula",
            "node_id": "HN4#30",
            "energy_type": "district_heating",
            "purpose": "space_heating",
            "terms": [{"ref": "S#1", "coefficient": 1}, {"ref": "S#2", "coefficient": -1}],
            "note": "bimåler"
        })
        .to_string();
        let cmd = parse_command(&body, Some("application/json")).unwrap();
        assert!(matches!(&cmd, Command::SetNodeFormula { node_id, purpose, .. }
                         if node_id == "HN4#30" && purpose == "space_heating"));
    }

    /// The form path posts `terms` as a JSON-encoded string.
    #[test]
    fn parse_set_node_formula_form() {
        let form = "action=set_node_formula\
                    &node_id=HN5%235\
                    &energy_type=electricity\
                    &purpose=cooling\
                    &terms=%5B%7B%22ref%22%3A%22S%231%22%2C%22coefficient%22%3A1%7D%5D";
        let cmd = parse_command(form, Some("application/x-www-form-urlencoded")).unwrap();
        assert!(matches!(&cmd, Command::SetNodeFormula { terms, .. }
                         if terms.is_string() || terms.is_array()));
    }

    #[test]
    fn parse_delete_node_formula() {
        let form = "action=delete_node_formula&node_id=HN4%2330\
                    &energy_type=district_heating&purpose=dhw";
        let cmd = parse_command(form, Some("application/x-www-form-urlencoded")).unwrap();
        assert!(matches!(&cmd, Command::DeleteNodeFormula { purpose, .. } if purpose == "dhw"));
    }

    #[test]
    fn parse_rebuild_company_matrix() {
        let form = "action=rebuild_company_matrix&company=HN2%23997";
        let cmd = parse_command(form, Some("application/x-www-form-urlencoded")).unwrap();
        assert!(matches!(&cmd, Command::RebuildCompanyMatrix { company } if company == "HN2#997"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p hierarchy node_formula 2>&1 | tail -20`
Expected: FAIL — `no variant named SetNodeFormula`.

- [ ] **Step 3: Add the command variants**

Insert into `enum Command` in `crates/services/hierarchy/src/command.rs`, before the closing brace:

```rust
    /// `set_node_formula` — upsert a node's `(energy_type, purpose)` formula.
    /// `terms` is a JSON array of `{ref, coefficient}`, or a JSON-encoded string
    /// of the same (the HTML form builds it client-side).
    SetNodeFormula {
        node_id: String,
        energy_type: String,
        purpose: String,
        terms: Value,
        #[serde(default)]
        note: Option<String>,
    },

    /// `delete_node_formula` — remove a node's `(energy_type, purpose)` formula.
    DeleteNodeFormula {
        node_id: String,
        energy_type: String,
        purpose: String,
    },

    /// `rebuild_company_matrix` — recompute a company's materialised weight
    /// matrix from scratch. Operator escape hatch for a skipped recompute.
    RebuildCompanyMatrix {
        company: String,
    },
```

- [ ] **Step 4: Run parse tests to verify they pass**

Run: `cargo test -p hierarchy node_formula 2>&1 | tail -20`
Expected: PASS (4 tests).

- [ ] **Step 5: Write the failing handler tests**

Add to `mod tests` in `crates/services/hierarchy/src/dispatch.rs`, following the in-memory-repo style already used there. Add `memory_store_with_company()` alongside the existing helpers if it doesn't exist — it must build a company containing `HN5#5` with `S#1` under it and `S#77` under a sibling node:

```rust
    #[tokio::test]
    async fn set_node_formula_stores_the_formula() {
        let store = memory_store_with_company();
        let out = set_formula(&store, "electricity", "cooling",
                              serde_json::json!([{"ref": "S#1", "coefficient": 1}])).await;
        assert_eq!(out["ok"], serde_json::json!(true));
        assert_eq!(out["formula"]["purpose"], serde_json::json!("cooling"));
    }

    /// Writing a formula rebuilds the company's materialised matrix — that is
    /// what the Glue roll-up reads, so it must never lag the formulas.
    #[tokio::test]
    async fn set_node_formula_rebuilds_the_matrix() {
        let store = memory_store_with_company();
        assert!(store.weight_rows().is_empty());
        set_formula(&store, "electricity", "cooling",
                    serde_json::json!([{"ref": "S#1", "coefficient": 1}])).await;
        let rows = store.weight_rows();
        assert!(rows.iter().any(|r| r.contains("weight#claim") && r.contains("cooling")),
                "claim row written, got: {rows:?}");
    }

    /// The subtree rule is enforced at the command boundary, not just in the UI.
    #[tokio::test]
    async fn set_node_formula_rejects_out_of_subtree_reference() {
        let store = memory_store_with_company();
        let out = set_formula(&store, "electricity", "cooling",
                              serde_json::json!([{"ref": "S#77", "coefficient": 1}])).await;
        assert_eq!(out["error"]["code"], serde_json::json!("Validation"));
    }

    #[tokio::test]
    async fn set_node_formula_rejects_reserved_purpose() {
        let store = memory_store_with_company();
        let out = set_formula(&store, "electricity", "total",
                              serde_json::json!([{"ref": "S#1", "coefficient": 1}])).await;
        assert_eq!(out["error"]["code"], serde_json::json!("Validation"));
    }

    #[tokio::test]
    async fn set_node_formula_rejects_malformed_terms() {
        let store = memory_store_with_company();
        let out = set_formula(&store, "electricity", "cooling",
                              serde_json::json!("not-json")).await;
        assert_eq!(out["error"]["code"], serde_json::json!("Bad_request"));
    }

    /// Attaching a sensor changes the matrix (a new descendant at weight 1) and
    /// must therefore trigger a rebuild too.
    #[tokio::test]
    async fn attach_sensor_rebuilds_the_matrix() {
        let store = memory_store_with_company();
        set_formula(&store, "electricity", "cooling",
                    serde_json::json!([{"ref": "S#1", "coefficient": 1}])).await;
        let before = store.weight_rows().len();
        attach_contained_sensor(&store, 2, Some("S#1")).await; // S#2 inside S#1
        let after = store.weight_rows();
        assert!(after.len() > before, "containment added total-override rows");
        assert!(after.iter().any(|r| r.contains("weight#total")));
    }
```

Add the `set_formula` and `attach_contained_sensor` test helpers that call the handlers with the store's closures.

- [ ] **Step 6: Run handler tests to verify they fail**

Run: `cargo test -p hierarchy set_node_formula_ 2>&1 | tail -20`
Expected: FAIL — `cannot find function handle_set_node_formula`.

- [ ] **Step 7: Implement the handlers and recompute**

Add to `crates/services/hierarchy/src/dispatch.rs`:

```rust
/// Parse the `terms` payload: a JSON array, or a JSON-encoded string of one.
fn parse_terms(v: &Value) -> Result<Vec<Term>, String> {
    let arr = match v {
        Value::Array(a) => a.clone(),
        Value::String(s) => serde_json::from_str::<Vec<Value>>(s)
            .map_err(|e| format!("terms is not a JSON array: {e}"))?,
        _ => return Err("terms must be a JSON array".to_string()),
    };
    arr.iter()
        .map(|t| {
            let r = t.get("ref").and_then(Value::as_str).ok_or("term missing \"ref\"")?;
            let c = t
                .get("coefficient")
                .and_then(Value::as_f64)
                .ok_or("term missing numeric \"coefficient\"")?;
            Ok(Term { reference: Reference::parse(r)?, coefficient: c })
        })
        .collect()
}

/// Load a company's graph, flatten it, and replace its materialised matrix.
/// Called by every command that can change the matrix: set/delete formula,
/// attach/replace/delete sensor, add/delete node.
pub async fn recompute_matrix<FLF, FLFFut, FLS, FLSFut, FLN, FLNFut, FRM, FRMFut>(
    company_path: String,
    list_company_formulas: FLF,
    list_company_sensors: FLS,
    list_company_nodes: FLN,
    replace_matrix: FRM,
) -> Result<(), RepositoryError>
where
    FLF: FnOnce(String) -> FLFFut,
    FLFFut: Future<Output = Result<Vec<NodeFormula>, RepositoryError>>,
    FLS: FnOnce(String) -> FLSFut,
    FLSFut: Future<Output = Result<Vec<Sensor>, RepositoryError>>,
    FLN: FnOnce(String) -> FLNFut,
    FLNFut: Future<Output = Result<Vec<Node>, RepositoryError>>,
    FRM: FnOnce(String, Matrix) -> FRMFut,
    FRMFut: Future<Output = Result<(), RepositoryError>>,
{
    let (formulas, sensors, nodes) = futures::try_join!(
        list_company_formulas(company_path.clone()),
        list_company_sensors(company_path.clone()),
        list_company_nodes(company_path.clone()),
    )?;
    let matrix = formulas_logic::flatten(&CompanyGraph { nodes, sensors, formulas });
    replace_matrix(company_path, matrix).await
}
```

`handle_set_node_formula` parses and validates `(node_id, energy_type, purpose, terms)`, resolves the company path from the node, loads the company graph, calls `formulas_logic::validate`, writes the formula item, then calls `recompute_matrix`. `handle_delete_node_formula` deletes the item then recomputes. `handle_rebuild_company_matrix` resolves the company node's path and calls `recompute_matrix` alone.

Add `formula_to_json` to `crates/services/hierarchy/src/json.rs` emitting `{node, energy_type, purpose, terms:[{ref, coefficient}], note}`.

Add the three `run` arms, and call `recompute_matrix` at the end of the existing `AttachSensor`, `ReplaceSensorDevice`, `DeleteSensor`, `AddNode` and `DeleteNode` arms (resolving the company path from the affected node). `UpdateNode` touches only metadata and must **not** recompute.

Add the closure factories to `crates/services/hierarchy/src/repo_fns.rs`, following the existing pattern exactly:

```rust
pub fn put_node_formula_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(NodeFormula, String, String) -> RepoFut<()> + Clone {
    move |f, node_path, company_path| {
        let t = table.clone();
        Box::pin(async move {
            ddb_formula::put_node_formula(ddb, &t, &f, &node_path, &company_path).await
        })
    }
}

pub fn replace_matrix_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(String, Matrix) -> RepoFut<()> + Clone {
    move |company_path, m| {
        let t = table.clone();
        Box::pin(async move {
            ddb_weight::replace_company_matrix(ddb, &t, &company_path, &m).await
        })
    }
}
```

plus `delete_node_formula_fn`, `list_company_formulas_fn`, `list_node_formulas_fn`, `list_company_sensors_fn` (wrapping `ddb_sensor::list_sensors_under_path`) and `list_company_nodes_fn` (wrapping `ddb_node::list_by_gsi1_prefix` fanned out over `HN2`..`HN9`).

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p hierarchy 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 9: Confirm the write gate**

Formula edits must sit behind the `writes` edge, the same gate as `attach_sensor`. Check how `run` gates `Command::AttachSensor` (`crates/services/hierarchy/src/dispatch.rs:821` onward) and apply the identical guard to all three new arms.

Run: `cargo test -p hierarchy 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/services/hierarchy/src/
git commit -m "feat(hierarchy): formula commands + materialised matrix recompute"
```

---

### Task 7: Formler tab, containment picker, remove the sensor-formula UI

**Files:**
- Modify: `crates/services/hierarchy/src/query.rs` (add `node_formulas` next to `"sensors"` at ~line 930)
- Modify: `crates/services/hierarchy/src/html/node.rs` (delete lines ~308-500, the formula dialog; add the Formler tab)
- Modify: `crates/services/hierarchy/src/html/forms.rs` (drop the formula row; rename the `purpose` select to `energy_type`; add the `contained_in` select)
- Test: `crates/services/hierarchy/tests/node_forms_html.rs`

**Interfaces:**
- Consumes: `list_node_formulas_fn`, `list_company_sensors_fn`, `list_company_nodes_fn` (Task 6)
- Produces: `GET /hierarchy/query/node_formulas?node=<NodeId>` → HTML fragment; `html::node::render_node_formulas(&Node, &[NodeFormula], &[Sensor], &[Node]) -> Markup`

- [ ] **Step 1: Write the failing HTML tests**

Add to `crates/services/hierarchy/tests/node_forms_html.rs`:

```rust
/// The Formler tab renders one card per formula, headed by (energy_type, purpose).
#[test]
fn node_formulas_render_one_card_per_formula() {
    let html = render_node_formulas_fixture();
    assert!(html.contains("district_heating"), "energy type in the heading");
    assert!(html.contains("space_heating"), "purpose in the heading");
    assert!(html.contains("bimåler"), "the note is shown");
    assert!(html.contains("-1"), "the subtraction coefficient is shown");
}

/// The reference picker offers only descendants of the node — the subtree rule
/// is enforced in the UI as well as in the command.
#[test]
fn node_formulas_reference_picker_lists_only_descendants() {
    let html = render_node_formulas_fixture();
    assert!(html.contains("S#1"), "descendant sensor is offered");
    assert!(!html.contains("S#77"), "sensor outside the subtree must not be offered");
}

/// Reserved purposes are never offered — nesting is recorded on the sensor.
#[test]
fn node_formulas_do_not_offer_reserved_purposes() {
    let html = render_node_formulas_fixture();
    assert!(!html.contains("value=\"total\""));
    assert!(!html.contains("value=\"unallocated\""));
}

/// The add-sensor form asks for both installation facts. Neither is inferable
/// from the device, and a forgotten one silently inflates every total above it,
/// so both are on the main form rather than behind an advanced section.
#[test]
fn add_sensor_form_asks_for_both_installation_facts() {
    let html = render_add_sensor_form_fixture();
    assert!(html.contains("name=\"contained_in\""));
    assert!(html.contains("Indgår allerede i"));
    assert!(html.contains("name=\"flow\""));
    assert!(html.contains("Retning"));
    assert!(html.contains("value=\"out\""));
}

/// The old per-sensor formula dialog is gone for good.
#[test]
fn add_sensor_form_has_no_formula_controls() {
    let html = render_add_sensor_form_fixture();
    assert!(!html.contains("formula-dialog"));
    assert!(!html.contains("data.formula.kind"));
    assert!(html.contains("name=\"energy_type\""), "purpose select renamed");
}
```

Add `render_node_formulas_fixture`, building two `NodeFormula`s on `HN4#30` (`district_heating/dhw`, and `district_heating/space_heating` with the −1 term and the note `"bimåler"`), a descendant sensor `S#1` and a non-descendant `S#77`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p hierarchy --test node_forms_html 2>&1 | tail -20`
Expected: FAIL — `cannot find function render_node_formulas`.

- [ ] **Step 3: Delete the sensor-formula UI**

In `crates/services/hierarchy/src/html/node.rs`, delete the whole formula block (the `<dialog id="formula-dialog">`, the kind select, the expression input, the alias→sensor ref rows, the three hidden `data.formula.*` inputs, the "Edit formula…" button, the summary span, and their inline `<script>`). In `crates/services/hierarchy/src/html/forms.rs`, remove the Formula row and rename the `purpose` select to `energy_type`.

Run: `cargo test -p hierarchy add_sensor_form_has_no_formula_controls 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 4: Add the two installation-fact controls**

In the add-sensor form in `crates/services/hierarchy/src/html/forms.rs`:

```rust
    label {
        "Indgår allerede i"
        select name="contained_in" {
            option value="" { "— indgår ikke i en anden måling —" }
            @for s in candidate_containers {
                option value=(s.id) { (s.daq_id) " (" (s.energy_type) ")" }
            }
        }
        span class="hint" {
            "Vælg den måling der allerede dækker denne — fx en akkumuleret kanal \
             over sine fasekanaler — så forbruget ikke tælles dobbelt."
        }
    }

    label {
        "Retning"
        select name="flow" {
            option value="in" selected { "Ind — forbrug, produktion, bimåler" }
            option value="out" { "Ud — eksport til nettet, solcelle-feed-in" }
        }
        span class="hint" {
            "Målinger med retning \"ud\" trækkes fra i totalen."
        }
    }
```

`candidate_containers` is the company's active sensors filtered to the same `energy_type` and attached to this node or an ancestor of it — the same constraint `validate_containment` enforces. Neither control may be hidden behind an advanced section: a survey of the live `raw_data` corpus found nothing in the stream that encodes either fact, so the form is the only place they can be captured.

- [ ] **Step 5: Implement the Formler tab**

Add to `crates/services/hierarchy/src/html/node.rs`:

```rust
/// The node panel's **Formler** tab: one card per declared formula, plus an
/// empty card for adding one. `descendant_*` is what the reference picker
/// offers — the subtree rule made visible.
pub fn render_node_formulas(
    node: &Node,
    formulas: &[NodeFormula],
    descendant_sensors: &[Sensor],
    descendant_nodes: &[Node],
) -> Markup {
    html! {
        div class="formulas" {
            @for f in formulas {
                section class="formula-card"
                        data-energy-type=(f.energy_type) data-purpose=(f.purpose) {
                    header {
                        span class="et" { (f.energy_type) }
                        span class="pur" { (f.purpose) }
                        @if let Some(n) = &f.note { span class="note" { (n) } }
                    }
                    form hx-post="/hierarchy/command" hx-swap="none" {
                        input type="hidden" name="action" value="set_node_formula";
                        input type="hidden" name="node_id" value=(node.id);
                        input type="hidden" name="energy_type" value=(f.energy_type);
                        input type="hidden" name="purpose" value=(f.purpose);
                        input type="hidden" name="terms" value=(terms_json(&f.terms));
                        @for t in &f.terms {
                            div class="term-row" {
                                (reference_select(&t.reference, descendant_sensors, descendant_nodes))
                                input type="number" step="any" class="coef" value=(t.coefficient);
                            }
                        }
                        button type="button" class="add-term" { "+ Term" }
                        button type="submit" { "Gem" }
                    }
                    form hx-post="/hierarchy/command" hx-swap="none" class="danger" {
                        input type="hidden" name="action" value="delete_node_formula";
                        input type="hidden" name="node_id" value=(node.id);
                        input type="hidden" name="energy_type" value=(f.energy_type);
                        input type="hidden" name="purpose" value=(f.purpose);
                        button type="submit" { "Slet" }
                    }
                }
            }
            (new_formula_card(node, descendant_sensors, descendant_nodes))
        }
    }
}

/// `<select>` of every reference the node may legally use: its descendant nodes
/// and descendant sensors, nothing else.
fn reference_select(selected: &Reference, sensors: &[Sensor], nodes: &[Node]) -> Markup {
    html! {
        select class="ref" {
            @for n in nodes {
                option value=(n.id) selected[*selected == Reference::Node(n.id.clone())] {
                    (n.name) " (" (n.id) ")"
                }
            }
            @for s in sensors {
                option value=(s.id) selected[*selected == Reference::Sensor(s.id)] {
                    (s.daq_id) " (" (s.energy_type) ")"
                }
            }
        }
    }
}
```

Add `terms_json` (serialising `&[Term]` to `[{"ref":…,"coefficient":…}]`) and `new_formula_card` — the same card with an empty term list plus selects populated from `EnergyType::all()` and `Purpose::all().filter(|p| p.declarable())`. Add a small inline script keeping the hidden `terms` input in sync with the rows on submit, the same technique the deleted dialog used.

Wire the query action in `crates/services/hierarchy/src/query.rs` next to `"sensors"`:

```rust
        "node_formulas" => {
            handle_node_formulas(
                qs.get("node").cloned().unwrap_or_default(),
                get_node_fn(ddb, table.clone()),
                list_node_formulas_fn(ddb, table.clone()),
                list_company_sensors_fn(ddb, table.clone()),
                list_company_nodes_fn(ddb, table.clone()),
            )
            .await
        }
```

`handle_node_formulas` loads the node, filters the company's sensors and nodes to strict descendants of `node.path`, and renders.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p hierarchy 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Add the tab to the node panel**

In the node panel's tab strip in `crates/services/hierarchy/src/html/node.rs`, add a **Formler** tab whose content loads via `hx-get="/hierarchy/query/node_formulas?node=<id>"`, matching how the existing Data/sensor tabs load.

Run: `cargo test -p hierarchy 2>&1 | tail -5 && cargo clippy --all-targets 2>&1 | grep -c warning`
Expected: tests PASS, `0` warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/services/hierarchy/
git commit -m "feat(hierarchy): Formler tab + containment picker; drop the sensor formula dialog"
```

---

### Task 8: Deploy Phase 1 and seed the matrices

**Files:** none (deploy only)

- [ ] **Step 1: Build and diff**

```bash
cargo lambda build --release --arm64 -p hierarchy
cd infra/hierarchy && unset GOROOT && \
export AWS_PROFILE=stel-sb && \
eval "$(aws configure export-credentials --profile stel-sb --format env)" && \
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1 && \
cdk diff OcamlHierarchyStack
```

Expected: only Lambda `Code` `[~]` updates. **Stop and report** if the DynamoDB table shows any change.

- [ ] **Step 2: Deploy**

```bash
cd infra/hierarchy && unset GOROOT && \
export AWS_PROFILE=stel-sb && \
eval "$(aws configure export-credentials --profile stel-sb --format env)" && \
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1 && \
cdk deploy OcamlHierarchyStack --require-approval never
aws lambda get-function-configuration --profile stel-sb \
  --function-name rust-lambda-hierarchy --query '{State:State,Last:LastUpdateStatus}'
```

Expected: `State=Active`, `LastUpdateStatus=Successful`.

- [ ] **Step 3: Seed every company's matrix**

Existing companies have no weight rows yet. For each HN2 node, fire the rebuild:

```bash
API=https://d24beiqs2cj89y.cloudfront.net
for c in $(curl -s "$API/hierarchy/query/nodes?level=hn2" | grep -o 'HN2#[0-9]*' | sort -u); do
  curl -s -X POST "$API/command" \
    -H 'Content-Type: application/x-www-form-urlencoded' \
    --data-urlencode "action=rebuild_company_matrix" --data-urlencode "company=$c"
  echo " <- $c"
done
```

Expected: `{"ok":true}` per company. Adjust the node-listing query to whatever `query.rs` actually exposes.

- [ ] **Step 4: Verify the rows landed**

```bash
aws dynamodb query --profile stel-sb --table-name hierarchy_new --index-name gsi1 \
  --key-condition-expression 'gsi1pk = :p' \
  --expression-attribute-values '{":p":{"S":"W#HN2#997"}}' \
  --query 'Items[].{kind:kind.S,sk:sk.S,coef:coefficient.N}' --max-items 10
```

Expected: weight rows for the company (empty is correct if it has no formulas and no covered sensors yet).

- [ ] **Step 5: Commit**

```bash
git commit --allow-empty -m "chore(hierarchy): deploy node-formula authoring (phase 1)"
```

---

# Phase 2 — Pipeline

Renames the `purpose` column to `energy_type` end-to-end and teaches the roll-up job to consume the matrix. **Tasks 9–11 must land together** — there is no fallback in the contract.

---

### Task 9: Bridge rename + cross-account reader role

**Files:**
- Modify: `infra/hierarchy/app.go:285-360` (inlined Python bridge `_item` builder), plus a new `HierarchyReaderRole`

**Interfaces:**
- Produces: `meter-identity` items with `energy_type` instead of `purpose` and **no** `formula`; an IAM role `arn:aws:iam::339712745226:role/HierarchyReaderRole` consumed by Task 11.

- [ ] **Step 1: Edit the bridge item builder**

In the inlined Python in `infra/hierarchy/app.go`, emit `"energy_type": {"S": img["energy_type"]["S"]}` instead of `"purpose"`, and **delete** the two lines copying `formula`:

```python
        if "formula" in img:
            out["formula"] = {"S": json.dumps(_d.deserialize(img["formula"]), default=str)}
```

- [ ] **Step 2: Add the reader role**

Find the Glue job's role name first:

```bash
grep -n 'NewRole\|RoleName' infra/daq/data_pipeline/measurements_aggregate_stack.go
```

Then add to `infra/hierarchy/app.go`, using that exact role ARN:

```go
	// The DAQ account's Glue roll-up job assumes this to read the materialised
	// weight matrix (spec §4.2, §6). Read-only, scoped to hierarchy_new + gsi1.
	readerRole := awsiam.NewRole(stack, jsii.String("HierarchyReaderRole"), &awsiam.RoleProps{
		RoleName: jsii.String("HierarchyReaderRole"),
		AssumedBy: awsiam.NewArnPrincipal(
			jsii.String("arn:aws:iam::891377204778:role/<GlueRoleName>")),
	})
	readerRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Actions: jsii.Strings("dynamodb:Query"),
		Resources: jsii.Strings(
			*hierarchyTable.TableArn(),
			*hierarchyTable.TableArn()+"/index/gsi1",
		),
	}))
```

- [ ] **Step 3: Diff and deploy**

```bash
cd infra/hierarchy && unset GOROOT && \
export AWS_PROFILE=stel-sb && \
eval "$(aws configure export-credentials --profile stel-sb --format env)" && \
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1 && \
cdk diff OcamlHierarchyStack
```

Expected: a new IAM role plus the bridge Lambda's inline code `[~]`. **Stop and report** if the DynamoDB table shows any change. Then `cdk deploy OcamlHierarchyStack --require-approval never`.

- [ ] **Step 4: Verify the bridge writes the new shape**

Re-save a sensor through the UI, then:

```bash
aws dynamodb scan --profile daq_dev --table-name meter-identity --limit 3 \
  --query 'Items[].{daq:sk.S,et:energy_type.S,purpose:purpose.S,formula:formula.S}'
```

Expected: `energy_type` populated; `purpose` and `formula` absent on freshly written rows.

- [ ] **Step 5: Commit**

```bash
git add infra/hierarchy/app.go
git commit -m "feat(bridge): purpose -> energy_type, drop formula; add HierarchyReaderRole"
```

---

### Task 10: Flink + Iceberg column rename

**Files:**
- Modify: `.../enrichment/MeterMapping.scala:25,72`, `DdbBootstrapLoader.scala:27,39`, `DdbStreamDeserializer.scala:35,45`, `MeterEnrichmentFunction.scala:109`, `flink/Main.scala:304,336,345`
- Modify: `infra/daq/data_pipeline/s3tables_stack.go:73`
- Modify: the four Scala spec files under `src/test/scala/` referencing `purpose`

- [ ] **Step 1: Rename in Scala and its tests**

Rename `purpose` → `energyType` in `MeterMapping`, both deserialisers, the enrichment function, and `Main.scala`'s table schema and column list (the Iceberg column is `energy_type`). Update the four spec files.

Run: `cd infra/daq/data_pipeline/flink_app_scala && sbt test 2>&1 | tail -20`
Expected: all specs PASS.

- [ ] **Step 2: Rename the Iceberg column**

In `infra/daq/data_pipeline/s3tables_stack.go:73`, change `field("purpose", "string", false)` to `field("energy_type", "string", false)` — in **both** the `raw_data` and `logical_meter_data` definitions.

- [ ] **Step 3: Two-step delete/recreate (destructive — confirm first)**

`AWS::S3Tables::Table` cannot be replaced in place: create-before-delete fails with `409 "table with an identical name already exists"`. **This clears both tables' data.** Kinesis retention is 24 h, so ~1 day is replayable.

Stop the Flink app first so it isn't writing to a table being dropped:

```bash
aws kinesisanalyticsv2 stop-application --profile daq_dev \
  --application-name flink-iceberg-processor --force
```

Step (1) — comment out the two table resources in `s3tables_stack.go` and deploy so CFN deletes them:

```bash
cd infra/daq/data_pipeline && unset GOROOT && \
export AWS_PROFILE=daq_dev && \
eval "$(aws configure export-credentials --profile daq_dev --format env)" && \
npx cdk diff S3TablesStack -c TableBucketName=measurements && \
npx cdk deploy S3TablesStack --require-approval never -c TableBucketName=measurements
```

Step (2) — restore both resources with the `energy_type` column and deploy again. Confirm the diff creates exactly the two tables.

- [ ] **Step 4: Redeploy Flink and restart**

```bash
cd infra/daq/data_pipeline/flink_app_scala && sbt clean assembly
cd .. && unset GOROOT && \
export AWS_PROFILE=daq_dev && \
eval "$(aws configure export-credentials --profile daq_dev --format env)" && \
npx cdk deploy DaqPipelineStack --require-approval never \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
aws kinesisanalyticsv2 start-application --profile daq_dev \
  --application-name flink-iceberg-processor \
  --run-configuration '{"ApplicationRestoreConfiguration":{"ApplicationRestoreType":"RESTORE_FROM_LATEST_SNAPSHOT"}}'
```

The operator `uid` and keyed-state descriptors are unchanged, so the snapshot restores; only the sink schema moved.

- [ ] **Step 5: Verify rows land with the new column**

```bash
aws athena start-query-execution --profile daq_dev --work-group daq-workgroup \
  --query-string "SELECT energy_type, count(*) FROM all.raw_data GROUP BY energy_type LIMIT 10"
```

Expected: rows grouped by populated `energy_type` values.

- [ ] **Step 6: Commit**

```bash
git add infra/daq/data_pipeline/
git commit -m "feat(pipeline)!: rename purpose -> energy_type in Flink and the Iceberg tables"
```

---

### Task 11: Weighted roll-up

**Files:**
- Create: `infra/daq/data_pipeline/glue/hierarchy_matrix.py`
- Modify: `infra/daq/data_pipeline/glue/measurements_aggregate.py` (`build_sk`, `build_gsi1pk`, `build_rollups`, `read_counters`, `main`)
- Modify: `infra/daq/data_pipeline/glue/tests/test_rollups.py`
- Modify: `infra/daq/data_pipeline/measurements_aggregate_stack.go` (extra-py-files, job arg, `sts:AssumeRole`)

**Interfaces:**
- Consumes: the materialised matrix (Task 5), `HierarchyReaderRole` (Task 9)
- Produces:
  - `hierarchy_matrix.reader_table(role_arn, region) -> boto3 Table`
  - `hierarchy_matrix.load_matrix(table, company_id) -> {"claims": [...], "total_overrides": [...]}`
  - `build_sk(node_path, energy_type, purpose, gran, bucket) -> str`
  - `build_gsi1pk(hn2, dimension, purpose) -> str`
  - `build_rollups(df, matrix, run_at_iso) -> DataFrame`

**The matrix module holds no domain rules.** It is a GSI query plus a dict transform — the flattening lives in `crates/model` and is materialised by the hierarchy service.

- [ ] **Step 1: Write the failing tests**

Rewrite `infra/daq/data_pipeline/glue/tests/test_rollups.py`'s `_input` so its column is `energy_type` with the lower-case token `"electricity"`, keep the idempotency test (updating its sort keys), and add:

```python
def _matrix():
    """10009 claimed as lighting; 10010 sits inside 10009, so it is zeroed at
    HN2#2 and HN3#9 but still counts at its own leaf."""
    return {
        "claims": [
            {"declaring_node": "HN2#2|HN3#9|HN4#456", "energy_type": "electricity",
             "purpose": "lighting", "sensor_id": 10009, "coefficient": 1.0,
             "derived": False, "allocates": True},
        ],
        "total_overrides": [
            {"node_path": "HN2#2", "energy_type": "electricity",
             "sensor_id": 10010, "coefficient": 0.0},
            {"node_path": "HN2#2|HN3#9", "energy_type": "electricity",
             "sensor_id": 10010, "coefficient": 0.0},
        ],
    }


def test_sort_key_carries_the_purpose_segment(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    assert "HN2#2#electricity#total#h#2026-06-07T08" in out
    assert "HN2#2|HN3#9|HN4#456#electricity#lighting#h#2026-06-07T08" in out


def test_total_honours_the_override_exception_list(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    # 10009 contributes 4+6 = 10; 10010's 5 is zeroed at HN2#2 (covered sensor).
    assert out["HN2#2#electricity#total#h#2026-06-07T08"]["sum"] == 10.0


def test_unlisted_pairs_default_to_weight_one(spark):
    """The override list is an EXCEPTION list — a left-join miss means weight 1."""
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    leaf = out["HN2#2|HN3#9|L#10010#electricity#total#h#2026-06-07T08"]
    assert leaf["sum"] == 5.0


def test_claims_roll_up_to_every_ancestor(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    for path in ["HN2#2", "HN2#2|HN3#9", "HN2#2|HN3#9|HN4#456"]:
        assert out["%s#electricity#lighting#h#2026-06-07T08" % path]["sum"] == 10.0


def test_unallocated_is_total_minus_claims(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2#electricity#unallocated#h#2026-06-07T08"]["sum"] == 0.0


def test_derived_claims_are_excluded_from_unallocated(spark):
    matrix = _matrix()
    matrix["claims"].append({
        "declaring_node": "HN2#2|HN3#9|HN4#456", "energy_type": "district_cooling",
        "purpose": "cooling", "sensor_id": 10009, "coefficient": 3.2,
        "derived": True, "allocates": False})
    out = _by_sk(m.build_rollups(_input(spark), matrix, run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2#district_cooling#cooling#h#2026-06-07T08"]["sum"] == 32.0
    assert out["HN2#2#district_cooling#unallocated#h#2026-06-07T08"]["sum"] == 0.0


def test_gsi1pk_carries_the_purpose(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    row = out["HN2#2#electricity#total#h#2026-06-07T08"]
    assert row["gsi1pk"] == "HN2#2#energy#total"
    assert row["gsi1sk"] == "HN2#2#h#2026-06-07T08"


def test_min_max_and_last_value_are_gone(spark):
    df = m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z")
    for dropped in ("min", "max", "last_value", "last_ts"):
        assert dropped not in df.columns
    assert "count" in df.columns
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd infra/daq/data_pipeline/glue && python -m pytest tests/test_rollups.py -q 2>&1 | tail -15`
Expected: FAIL — `build_rollups() takes 2 positional arguments but 3 were given`.

- [ ] **Step 3: Write the matrix reader**

Create `infra/daq/data_pipeline/glue/hierarchy_matrix.py`:

```python
"""Read a company's materialised weight matrix from hierarchy_new (cross-account).

Deliberately holds NO domain rules: the flattening (containment, outflow, the
subtree rule, node-reference expansion, derived detection) lives in
crates/model/src/logic/formulas.rs and is materialised by the hierarchy service.
This module is a GSI query and a dict transform.
"""


def reader_table(role_arn, region, table_name="hierarchy_new"):
    """hierarchy_new in the hierarchy account, via HierarchyReaderRole."""
    import boto3
    c = boto3.client("sts").assume_role(
        RoleArn=role_arn, RoleSessionName="measurements-aggregate")["Credentials"]
    return boto3.resource(
        "dynamodb", region_name=region,
        aws_access_key_id=c["AccessKeyId"],
        aws_secret_access_key=c["SecretAccessKey"],
        aws_session_token=c["SessionToken"]).Table(table_name)


def _query_gsi(table, gsi1pk):
    from boto3.dynamodb.conditions import Key
    items, kwargs = [], {"IndexName": "gsi1",
                         "KeyConditionExpression": Key("gsi1pk").eq(gsi1pk)}
    while True:
        page = table.query(**kwargs)
        items.extend(page.get("Items", []))
        if "LastEvaluatedKey" not in page:
            return items
        kwargs["ExclusiveStartKey"] = page["LastEvaluatedKey"]


def load_matrix(table, company_id):
    """One query per company. Returns claims + the total-weight exception list."""
    rows = _query_gsi(table, "W#HN2#%d" % int(company_id))
    claims, overrides = [], []
    for r in rows:
        if r.get("kind") == "claim":
            claims.append({
                "declaring_node": r["declaring_node"],
                "energy_type": r["energy_type"],
                "purpose": r["purpose"],
                "sensor_id": int(r["sensor_id"]),
                "coefficient": float(r["coefficient"]),
                "derived": bool(r.get("derived", False)),
                "allocates": bool(r.get("allocates", True)),
            })
        elif r.get("kind") == "total":
            overrides.append({
                "node_path": r["node_path"],
                "energy_type": r["energy_type"],
                "sensor_id": int(r["sensor_id"]),
                "coefficient": float(r["coefficient"]),
            })
    return {"claims": claims, "total_overrides": overrides}


def merge(a, b):
    """Combine two companies' matrices."""
    return {"claims": a["claims"] + b["claims"],
            "total_overrides": a["total_overrides"] + b["total_overrides"]}


EMPTY = {"claims": [], "total_overrides": []}
```

- [ ] **Step 4: Rewrite the key builders and `build_rollups`**

In `infra/daq/data_pipeline/glue/measurements_aggregate.py`:

```python
def build_sk(node_path: str, energy_type: str, purpose: str, gran: str, bucket: str) -> str:
    """sk = '<node_path>#<energy_type>#<purpose>#<gran>#<bucket>'. The bucket stays
    LAST so a fixed (energy_type, purpose) is a pure BETWEEN key-range. The '#'
    after node_path keeps a node's own rows sorting before its descendants'
    ('|' > '#')."""
    return "%s#%s#%s#%s#%s" % (node_path, energy_type, purpose, gran, bucket)


def build_gsi1pk(hn2: int, dimension: str, purpose: str) -> str:
    """The dimension partition is per-purpose, so a cross-type 'all energy' query
    can never sum `total` together with its own purpose breakdown."""
    return "HN2#%d#%s#%s" % (hn2, dimension, purpose)


def path_ancestors(node_path: str):
    """'A|B|C' -> ['A', 'A|B', 'A|B|C'] — the claim-propagation chain."""
    segs = node_path.split("|")
    return ["|".join(segs[: i + 1]) for i in range(len(segs))]


_PATH_ANCESTORS_UDF = F.udf(path_ancestors, T.ArrayType(T.StringType()))
_GSI1PK_UDF = F.udf(build_gsi1pk, T.StringType())
```

Replace `build_rollups`:

```python
def build_rollups(df: DataFrame, matrix: dict, run_at_iso: str) -> DataFrame:
    """Aggregate counter rows into per-node/energy_type/purpose/gran/bucket items.

    Three series come out:
      total        Σ of every descendant sensor of that energy type, each at its
                   weight — 1 unless the matrix lists an exception for this
                   (node_path, sensor).
      <purpose>    Σ of the declared claims, rolled up to every ancestor of the
                   declaring node.
      unallocated  total − Σ(claims whose `allocates` flag is set).

    `matrix` is {"claims": [...], "total_overrides": [...]} from hierarchy_matrix.
    All formula semantics were resolved upstream; this function only joins and sums.
    """
    spark = df.sparkSession

    with_buckets = df.withColumn(
        "gb",
        F.explode(F.array(
            F.struct(F.lit("h").alias("gran"),
                     F.date_format(F.col("resample_timestamp"), "yyyy-MM-dd'T'HH").alias("bucket")),
            F.struct(F.lit("d").alias("gran"),
                     F.date_format(F.col("resample_timestamp"), "yyyy-MM-dd").alias("bucket")),
        )),
    ).select("*", F.col("gb.gran").alias("gran"), F.col("gb.bucket").alias("bucket"))

    exploded = with_buckets.withColumn(
        "node_path",
        F.explode(_ancestor_keys_udf(
            *[F.col("hn%d" % i) for i in range(2, 10)], F.col("logical_id"))))

    # ── total: left-join the exception list, default weight 1 ──
    ov_schema = T.StructType([
        T.StructField("o_node_path", T.StringType()),
        T.StructField("o_energy_type", T.StringType()),
        T.StructField("o_logical_id", T.IntegerType()),
        T.StructField("o_weight", T.DoubleType()),
    ])
    ov_rows = [(o["node_path"], o["energy_type"], int(o["sensor_id"]), float(o["coefficient"]))
               for o in matrix["total_overrides"]]
    overrides = spark.createDataFrame(ov_rows, ov_schema)

    totals = (exploded
        .join(F.broadcast(overrides),
              (exploded.node_path == overrides.o_node_path)
              & (exploded.logical_id == overrides.o_logical_id)
              & (exploded.energy_type == overrides.o_energy_type), "left")
        .withColumn("weight", F.coalesce(F.col("o_weight"), F.lit(1.0)))
        .withColumn("contrib", F.col("resample_value") * F.col("weight"))
        .groupBy("hn2", "node_path", "energy_type", "gran", "bucket")
        .agg(F.sum("contrib").alias("sum"),
             F.count("contrib").alias("count"),
             F.max("unit").alias("unit"))
        .withColumn("purpose", F.lit("total"))
        .withColumn("derived", F.lit(False))
        .withColumn("allocates", F.lit(False)))

    # ── declared purposes ──
    if matrix["claims"]:
        cl_schema = T.StructType([
            T.StructField("declaring_node", T.StringType()),
            T.StructField("c_energy_type", T.StringType()),
            T.StructField("purpose", T.StringType()),
            T.StructField("c_logical_id", T.IntegerType()),
            T.StructField("coefficient", T.DoubleType()),
            T.StructField("derived", T.BooleanType()),
            T.StructField("allocates", T.BooleanType()),
        ])
        claims = spark.createDataFrame(
            [(c["declaring_node"], c["energy_type"], c["purpose"], int(c["sensor_id"]),
              float(c["coefficient"]), bool(c["derived"]), bool(c["allocates"]))
             for c in matrix["claims"]],
            cl_schema)
        claimed = (with_buckets
            .join(F.broadcast(claims),
                  with_buckets.logical_id == claims.c_logical_id, "inner")
            # A claim declared on a node counts for that node AND every ancestor.
            .withColumn("node_path", F.explode(_PATH_ANCESTORS_UDF(F.col("declaring_node"))))
            .withColumn("contrib", F.col("resample_value") * F.col("coefficient"))
            .groupBy("hn2", "node_path", "c_energy_type", "purpose", "gran", "bucket",
                     "derived", "allocates")
            .agg(F.sum("contrib").alias("sum"),
                 F.count("contrib").alias("count"),
                 F.max("unit").alias("unit"))
            .withColumnRenamed("c_energy_type", "energy_type"))
    else:
        claimed = totals.limit(0)

    # ── unallocated = total − Σ(physical, non-outflow claims) ──
    # `allocates` is computed in crates/model (derived rows and outflow purposes
    # do not reduce unallocated). The job filters on the flag, never on a purpose name.
    physical = (claimed
        .filter(F.col("allocates"))
        .groupBy("hn2", "node_path", "energy_type", "gran", "bucket")
        .agg(F.sum("sum").alias("claimed_sum")))
    unallocated = (totals
        .join(physical, ["hn2", "node_path", "energy_type", "gran", "bucket"], "left")
        .withColumn("sum", F.col("sum") - F.coalesce(F.col("claimed_sum"), F.lit(0.0)))
        .drop("claimed_sum")
        .withColumn("purpose", F.lit("unallocated")))

    grouped = totals.unionByName(claimed).unionByName(unallocated)

    return grouped.select(
        F.concat(F.lit("HN2#"), F.col("hn2").cast("string")).alias("pk"),
        _SK_UDF("node_path", "energy_type", "purpose", "gran", "bucket").alias("sk"),
        _GSI1PK_UDF("hn2", _DIM_UDF("unit"), "purpose").alias("gsi1pk"),
        _GSI1SK_UDF("node_path", "gran", "bucket").alias("gsi1sk"),
        "energy_type", "purpose", "unit", "sum", "count",
        F.lit(run_at_iso).alias("updated_at"),
        _TTL_UDF("gran", "bucket").alias("ttl"),
    )
```

Rename the `purpose` column to `energy_type` in `read_counters`'s `SELECT` and projection.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd infra/daq/data_pipeline/glue && python -m pytest tests/ -q 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 6: Wire the matrix into `main`**

```python
    counters = read_counters(spark, window_start)
    companies = [r["hn2"] for r in counters.select("hn2").distinct().collect()]
    table = hierarchy_matrix.reader_table(args["hierarchy_reader_role_arn"], region)
    matrix = hierarchy_matrix.EMPTY
    for cid in companies:
        matrix = hierarchy_matrix.merge(matrix, hierarchy_matrix.load_matrix(table, cid))
    rollups = build_rollups(counters, matrix, now.strftime("%Y-%m-%dT%H:%M:%S+00:00"))
```

Add `hierarchy_reader_role_arn` to the `required` args list. In `measurements_aggregate_stack.go`, pass that argument, add `hierarchy_matrix.py` via `--extra-py-files`, and grant the Glue role `sts:AssumeRole` on `arn:aws:iam::339712745226:role/HierarchyReaderRole`.

- [ ] **Step 7: Diff and deploy**

```bash
cd infra/daq/data_pipeline && unset GOROOT && \
export AWS_PROFILE=daq_dev && \
eval "$(aws configure export-credentials --profile daq_dev --format env)" && \
npx cdk diff MeasurementsAggregateStack \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
```

Expected: Glue script asset `[~]`, IAM policy `[~]`, a new job argument. **Stop and report** if the `measurements_aggregate` table shows replacement. Then deploy with the same `-c` flags.

- [ ] **Step 8: Wipe and rebuild the view**

The sort key gained a segment, so old rows are unreadable garbage:

```bash
aws dynamodb scan --profile daq_dev --table-name measurements_aggregate \
  --projection-expression 'pk,sk' --query 'Items[*]' --output json \
  > /tmp/claude-1000/-home-sla-projects-ems-rust/dc6f41e2-8599-4c7a-8bce-3e403680bd17/scratchpad/old-rollups.json
# delete in batches of 25 via batch-write-item, then:
aws glue start-job-run --profile daq_dev --job-name measurements-aggregate \
  --arguments '{"--lookback_days":"30"}'
aws glue get-job-runs --profile daq_dev --job-name measurements-aggregate \
  --max-results 1 --query 'JobRuns[0].{State:JobRunState,Error:ErrorMessage}'
```

Expected: `State=SUCCEEDED`.

- [ ] **Step 9: Commit**

```bash
git add infra/daq/data_pipeline/
git commit -m "feat(glue): weighted roll-up from the materialised matrix (total/purpose/unallocated)"
```

---

# Phase 3 — Read side + frontend

---

### Task 12: Aggregations reads the purpose axis

**Files:**
- Modify: `crates/services/aggregations/src/main.rs` — `parse_sk`, `Row` (line 145), `to_rows` (157), `to_rows_dimension` (187), `QueryParams` (236), `query_node` (253), `query_one_resource` (268), `query_dimension` (301), `handle_aggregations` (441), `fetch_node_rows` (550), `tariff_dkk_per_unit` (609), `emission_kg_per_unit` (623), `scale_rows` (654)

**Interfaces:**
- Produces:
  - `Row { level_id, energy_type, purpose, unit, resolution, timestamp, value, contributor_count }`
  - `QueryParams { …, energy_type, purpose }`
  - `sk_prefix(sk_path, energy_type, purpose, gran) -> String`
  - `tariff_dkk_per_unit(energy_type, purpose, unit) -> f64`, `emission_kg_per_unit(energy_type, purpose, unit) -> f64`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/services/aggregations/src/main.rs`:

```rust
    #[test]
    fn parse_sk_reads_the_purpose_segment() {
        let (path, et, purpose, gran, bucket) =
            parse_sk("HN2#2|HN3#9#electricity#lighting#h#2026-06-07T08");
        assert_eq!(path, "HN2#2|HN3#9");
        assert_eq!(et, "electricity");
        assert_eq!(purpose, "lighting");
        assert_eq!(gran, "h");
        assert_eq!(bucket, "2026-06-07T08");
    }

    /// The default purpose is `total`, so existing widgets keep their meaning
    /// at their current cost — one pure BETWEEN range per energy type.
    #[test]
    fn query_prefix_defaults_to_total() {
        assert_eq!(sk_prefix("HN2#2", "electricity", "", Gran::Hour),
                   "HN2#2#electricity#total#h#");
        assert_eq!(sk_prefix("HN2#2", "electricity", "lighting", Gran::Hour),
                   "HN2#2#electricity#lighting#h#");
    }

    /// Exported energy must not be billed or charged CO₂ — this is what replaces
    /// the demo's per-sensor emission factor.
    #[test]
    fn generation_has_no_tariff_or_emissions() {
        assert_eq!(emission_kg_per_unit("electricity", "generation", "kWh"), 0.0);
        assert_eq!(tariff_dkk_per_unit("electricity", "generation", "kWh"), 0.0);
    }

    #[test]
    fn other_purposes_fall_back_to_the_energy_type_factor() {
        assert_eq!(emission_kg_per_unit("electricity", "lighting", "kWh"), 0.12);
        assert_eq!(emission_kg_per_unit("electricity", "total", "kWh"), 0.12);
        assert_eq!(tariff_dkk_per_unit("water", "total", "m3"), 50.0);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aggregations parse_sk_reads 2>&1 | tail -20`
Expected: FAIL — `parse_sk` returns a 4-tuple.

- [ ] **Step 3: Implement**

1. `parse_sk` returns `(path, energy_type, purpose, gran, bucket)`.
2. Extract the prefix builder so it is testable:

```rust
/// `<node_path>#<energy_type>#<purpose>#<gran>#` — the bucket is appended by the
/// caller to form a pure `BETWEEN` key-range. An empty `purpose` means `total`.
fn sk_prefix(sk_path: &str, energy_type: &str, purpose: &str, gran: Gran) -> String {
    let purpose = if purpose.is_empty() { "total" } else { purpose };
    format!("{sk_path}#{energy_type}#{purpose}#{}#", gran.code())
}
```

3. Add `purpose: &'a str` to `QueryParams`; thread it from `handle_aggregations` (`qs.get("purpose")`, default `""`) and from `fetch_node_rows` (always `"total"`). Rename `QueryParams.resource` to `energy_type`.
4. `query_dimension`'s `gsi1pk` becomes `format!("{}#{}#{}", p.pk, dimension, if p.purpose.is_empty() { "total" } else { p.purpose })`.
5. `Row` gains `energy_type: String`; `purpose` now holds the real purpose.
6. The factor tables take a purpose:

```rust
/// Representative unit price (DKK). Outflow purposes are not billed.
fn tariff_dkk_per_unit(energy_type: &str, purpose: &str, unit: &str) -> f64 {
    if purpose == "generation" {
        return 0.0;
    }
    let (per_kwh, per_m3) = match energy_type {
        "electricity" => (2.50, 0.0),
        "district_heating" | "heat" => (0.90, 0.0),
        "district_cooling" => (0.50, 0.0),
        "gas" => (0.0, 8.0),
        "water" => (0.0, 50.0),
        _ => (0.0, 0.0),
    };
    per_unit(per_kwh, per_m3, unit)
}

/// Representative CO₂e factor (kg CO₂e). Exported energy emits nothing here — it
/// is an outflow, which is why no per-sensor emission factor is needed.
fn emission_kg_per_unit(energy_type: &str, purpose: &str, unit: &str) -> f64 {
    if purpose == "generation" {
        return 0.0;
    }
    let (per_kwh, per_m3) = match energy_type {
        "electricity" => (0.12, 0.0),
        "district_heating" | "heat" => (0.06, 0.0),
        "district_cooling" => (0.04, 0.0),
        "gas" => (0.0, 2.05),
        "water" => (0.0, 0.34),
        _ => (0.0, 0.0),
    };
    per_unit(per_kwh, per_m3, unit)
}
```

7. `scale_rows`'s closure becomes `Fn(&str, &str, &str) -> f64`, called as `factor(&r.energy_type, &r.purpose, &r.unit)`.
8. `handle_alarms` and `handle_benchmark` pass `purpose = "total"` explicitly.
9. Update the `#[utoipa::path]` params with `purpose`, and rename the `resource` param to `energy_type`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p aggregations 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/services/aggregations/
git commit -m "feat(aggregations): read the purpose axis; (energy_type, purpose) tariff and CO2 factors"
```

---

### Task 13: `get_purpose_split` query action

**Files:**
- Modify: `crates/services/aggregations/src/main.rs` (route table at lines 387-403; new handler; `openapi_response`)

**Interfaces:**
- Consumes: `sk_prefix`, `QueryParams` (Task 12)
- Produces: `GET /meterdata/query/get_purpose_split?level_id=&energy_type=&start=&end=&resolution=&format=`

- [ ] **Step 1: Write the failing test**

```rust
    /// The split keeps `total` separate and the physical purposes plus
    /// `unallocated` add up to it.
    #[test]
    fn purpose_split_rows_are_grouped_by_purpose() {
        let items = vec![
            agg_item("HN2#2#electricity#total#d#2026-06-07", 100.0),
            agg_item("HN2#2#electricity#lighting#d#2026-06-07", 22.0),
            agg_item("HN2#2#electricity#cooling#d#2026-06-07", 18.0),
            agg_item("HN2#2#electricity#unallocated#d#2026-06-07", 60.0),
        ];
        let rows = to_rows(items, "HN2#2", "daily", Gran::Day);
        let by: std::collections::BTreeMap<_, _> =
            rows.iter().map(|r| (r.purpose.as_str(), r.value)).collect();
        assert_eq!(by["total"], 100.0);
        assert_eq!(by["lighting"] + by["cooling"] + by["unallocated"], by["total"]);
    }
```

Add the `agg_item(sk, sum)` helper (`unit = "kWh"`, `count = 1`) if the module lacks one.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p aggregations purpose_split 2>&1 | tail -20`
Expected: FAIL until `to_rows` populates `purpose` from the new segment.

- [ ] **Step 3: Add the route and handler**

```rust
            "get_purpose_split" => {
                api::finish(handle_purpose_split(client, table, &qs).await, Cors::None)
            }
```

```rust
/// `GET /meterdata/query/get_purpose_split` — a node's end-use breakdown for one
/// energy type: every declared purpose plus `total` and `unallocated`.
#[utoipa::path(
    get,
    path = "/meterdata/query/get_purpose_split",
    tag = "aggregations",
    params(
        ("level_id" = String, Query, description = "Hierarchy node path"),
        ("energy_type" = String, Query, description = "Energy type to break down"),
        ("resolution" = Option<String>, Query, description = "hourly | daily (default)"),
        ("start" = String, Query, description = "ISO-8601 start (required)"),
        ("end" = String, Query, description = "ISO-8601 end (required)"),
        ("format" = Option<String>, Query, description = "json (default) | html"),
    ),
    responses(
        (status = 200, description = "One row per purpose", body = Vec<Row>),
        (status = 400, description = "Missing/invalid parameters", body = api::ErrorResponse),
    ),
)]
async fn handle_purpose_split(
    client: &Client,
    table: &str,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Json);
    let (level_id, resolution, start, end) = window_params(qs)?;
    let energy_type = qs.get("energy_type").cloned().unwrap_or_default();
    if energy_type.is_empty() {
        return Err(ApiError::bad_request("energy_type is required"));
    }

    let gran = Gran::from_resolution(&resolution);
    let (pk, sk_path) = match parse_node_keys(&level_id) {
        Ok(keys) => keys,
        Err(_) => return Ok(rows_response(&[], format)),
    };
    let (start_bucket, end_bucket) = match (bucket_label(&start, gran), bucket_label(&end, gran)) {
        (Ok(s), Ok(e)) => (s, e),
        _ => return Err(ApiError::bad_request("start/end must be ISO-8601 timestamps")),
    };

    // Fan out over every purpose concurrently — the same shape as the existing
    // fan-out over the six energy types.
    let queries = Purpose::all().map(|p| {
        let params = QueryParams {
            table, pk: &pk, sk_path: &sk_path, gran,
            start_bucket: &start_bucket, end_bucket: &end_bucket,
            energy_type: &energy_type, purpose: p.as_str(),
        };
        query_one_energy_type(client, &params, &energy_type)
    });
    let items: Vec<AggItem> = try_join_all(queries)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .into_iter()
        .flatten()
        .collect();

    Ok(rows_response(&to_rows(items, &level_id, &resolution, gran), format))
}
```

Import `model::domain::values::Purpose` alongside `EnergyType`, and add the path to `openapi_response()`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p aggregations 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Build, deploy and verify**

```bash
cargo lambda build --release --arm64 -p aggregations
cd infra/daq/data_pipeline && unset GOROOT && \
export AWS_PROFILE=daq_dev && \
eval "$(aws configure export-credentials --profile daq_dev --format env)" && \
npx cdk diff MeasurementsAggregateStack \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
```

Expected: Lambda `Code` `[~]` and a new HTTP API route only. Deploy, then:

```bash
API=$(aws cloudformation describe-stacks --profile daq_dev \
  --stack-name MeasurementsAggregateStack \
  --query 'Stacks[0].Outputs[?OutputKey==`AggregationsApiUrl`].OutputValue' --output text)
curl -s "$API/meterdata/query/get_purpose_split?level_id=HN2%23997&energy_type=electricity&start=2026-07-01T00:00:00Z&end=2026-07-28T00:00:00Z"
```

Expected: JSON rows including `total` and `unallocated`.

- [ ] **Step 6: Commit**

```bash
git add crates/services/aggregations/ infra/daq/data_pipeline/
git commit -m "feat(aggregations): get_purpose_split end-use breakdown"
```

---

### Task 14: End-use breakdown in the node dashboard

**Files:**
- Create: `frontend/src/components/PurposeSplit.astro`
- Modify: the node Data-tab dashboard hosting the aggregation widgets (`grep -rn "get_aggregations" frontend/src`)

- [ ] **Step 1: Build the widget**

```astro
---
const { levelId, energyType = "electricity", start, end } = Astro.props;
const base = import.meta.env.PUBLIC_AGG_API_BASE_URL;
const url = `${base}/meterdata/query/get_purpose_split` +
  `?level_id=${encodeURIComponent(levelId)}&energy_type=${energyType}` +
  `&start=${start}&end=${end}&resolution=daily&format=html`;
---
<section class="purpose-split">
  <h3>Formålsopdeling</h3>
  <table>
    <thead>
      <tr><th>Formål</th><th>Tid</th><th>Værdi</th><th>Enhed</th><th>Punkter</th></tr>
    </thead>
    <tbody hx-get={url} hx-trigger="load, ems:refresh from:body" hx-swap="innerHTML">
      <tr><td colspan="5" class="muted">Henter…</td></tr>
    </tbody>
  </table>
</section>

<style>
  .purpose-split { display: grid; gap: 8px; }
  .purpose-split table { width: 100%; border-collapse: collapse; }
  .purpose-split th { text-align: left; font-weight: 500; }
</style>
```

Layout uses grid, per the project's CSS rule. Mount it in the node Data tab next to the existing aggregation widgets.

- [ ] **Step 2: Make it soft-nav safe**

Confirm the widget re-fires on soft navigation: content outside `#node-data-panel` must call `htmx.process` on `astro:page-load`. Follow whatever the neighbouring widgets in the same file do.

- [ ] **Step 3: Build and deploy**

```bash
cd frontend && npm run build
cd ../infra/frontend && unset GOROOT && \
export AWS_PROFILE=stel-sb && \
eval "$(aws configure export-credentials --profile stel-sb --format env)" && \
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1 && \
cdk deploy OcamlFrontendStack --require-approval never
```

- [ ] **Step 4: Verify live**

Open `https://d24beiqs2cj89y.cloudfront.net`, go to a company node's Data tab, and confirm the Formålsopdeling table renders rows including `total` and `unallocated`.

- [ ] **Step 5: Commit**

```bash
git add frontend/
git commit -m "feat(frontend): end-use breakdown widget on the node Data tab"
```

---

### Task 15: End-to-end acceptance against the presentation

**Files:** none (verification only)

`docs/hierarchy-presentation.html` is a runnable model of the arithmetic. Reproducing its numbers through the real stack is the acceptance test for the whole plan.

- [ ] **Step 1: Author the fixture company through the UI**

On a scratch company, build the presentation's shape and record:

| Where | What |
|---|---|
| Chiller phase-channel sensors | `contained_in` = the accumulating channel |
| Building A1 DHW submeter sensor | `contained_in` = the main heat sensor |
| Area A1b grid-export sensor | `flow` = **out** (import and production stay `in`) |
| Chiller | `electricity/cooling` = accumulator × 1 |
| Chiller | `district_cooling/cooling` = accumulator × 3.2 |
| Area A1b | `electricity/generation` = export × 1 (reporting only) |
| Building A1 | `district_heating/dhw` = DHW × 1 |
| Building A1 | `district_heating/space_heating` = main × 1 + DHW × −1 |
| Building A2 | `district_heating/dhw` = main × 0.28 |
| Building A2 | `district_heating/space_heating` = main × 0.72 |
| Building B1 | `electricity/space_heating` = heat pump × 0.7 |
| Building B1 | `electricity/dhw` = heat pump × 0.3 |
| Building B1 | `heat/space_heating` = heat pump × 3.5 |
| Building B2 | `heat/space_heating` = gas × 10.45 |

- [ ] **Step 2: Run the roll-up**

```bash
aws glue start-job-run --profile daq_dev --job-name measurements-aggregate \
  --arguments '{"--lookback_days":"2"}'
```

- [ ] **Step 3: Assert the six invariants**

Query `get_purpose_split` per fixture node and check:

1. **No double counting** — the chiller's `electricity/total` equals its `electricity/cooling`; the phase panel's own `electricity/total` equals the phase sum. Both are true at once, which is the point of the overlap rule being relational.
2. **Exact partition** — Building A1's `district_heating`: `space_heating + dhw == total`, `unallocated == 0`. Same for A2 with the 0.28/0.72 split.
3. **Signed total** — Area A1b's `electricity/total` equals `import + production − export`. Remove the production sensor from the fixture and it becomes `import − export`; both are correct, and neither is the 150 that zeroing the export sensor would give.
4. **One sensor, many purposes** — Building B1's `electricity`: `lighting + ventilation + space_heating + dhw == total`, `unallocated == 0`, with the heat pump's 0.7/0.3 split summing to exactly one sensor's reading.
5. **Derived rows float free** — `district_cooling/cooling` and both `heat/space_heating` sources are non-zero while their `total` and `unallocated` are 0.
6. **The matrix is live** — edit one formula, re-run the job, and confirm the numbers move without any manual rebuild.

- [ ] **Step 4: Record the result**

Append an "Acceptance" section to `docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md` with the measured values, so the spec records what was verified rather than what was intended.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md
git commit -m "docs: record node-formula roll-up acceptance results"
```

---

## Self-Review

**Spec coverage.** §2 D1–D8 → Tasks 1-3, 5, 11. §2.1 naming → Task 1. §3.1 deletions → Task 2. §3.2 rename → Tasks 1, 9, 10, 11, 12. §3.3 types → Tasks 1, 2. §3.4 flatten → Task 3. §3.5 subtree rule → Task 4. §3.6 claim propagation → Tasks 3, 11. §3.7 derived → Tasks 3, 11. §3.8 containment + outflow → Tasks 2, 3, 4, 7, 11. §4.1 formula items → Task 5. §4.2 materialised matrix → Tasks 5, 6. §5 hierarchy service → Tasks 6, 7. §6 bridge + reader role → Task 9. §7 Flink/Iceberg → Task 10. §8.1 keys, §8.2 stages, §8.3 dropped attributes, §8.4 no duplicated semantics → Task 11. §9 read side → Tasks 12, 13. §11 testing → distributed. §12 deploy order → Tasks 8, 9, 10, 11, 13, 14. §13 out of scope → not planned, correctly.

**Gap found and closed:** §4.2 says the matrix is recomputed by the *service*, but the triggering commands are listed only in §5.2 prose. Task 6 Step 7 names each arm explicitly and Step 5 tests that `attach_sensor` triggers it — recompute-on-formula-write alone would have left the matrix stale on every sensor change.

**Gap found and closed:** §12 says wipe `measurements_aggregate` and re-run, without saying old rows become unreadable when the sort key gains a segment — made explicit in Task 11 Step 8. Likewise, existing companies have no weight rows after Phase 1, so Task 8 Step 3 seeds them via `rebuild_company_matrix`.

**Gap found and closed:** `contained_in` and `flow` can both apply to one sensor (a covered channel that also exports). Task 3's `total_overrides` resolves it explicitly — the containment 0 row wins over the −1 row at nodes where the container is present, so a covered sensor is never double counted whichever way it flows.

**No Glue change from the `Flow` work.** The roll-up consumes `total_overrides` as opaque `(node_path, sensor, coefficient)` rows and multiplies; a coefficient of −1 needs no code change, which is the payoff of materialising the matrix (D6). Task 11's tests still exercise only 0-weight overrides, which is correct — the −1 path is covered in Task 3 where the rule lives.

**Type consistency.** `Reference`/`Term`/`NodeFormula` (Task 2) are used unchanged in Tasks 3-7. `CompanyGraph`/`Claim`/`TotalOverride`/`Matrix`/`flatten`/`total_weight_at`/`is_derived` (Task 3) are consumed with identical signatures in Tasks 4, 5, 6. The PySpark side (Task 11) uses `sensor_id` where Rust uses `sensor`, and `claims`/`total_overrides` matching `Matrix`'s field names — the codec in Task 5 writes exactly those keys, and Task 11's `load_matrix` reads them. `build_sk` gains its `purpose` parameter in Task 11 and every call site there passes five arguments. `tariff_dkk_per_unit`/`emission_kg_per_unit` gain `purpose` in Task 12 and `scale_rows`'s closure signature is updated in the same task. `query_one_resource` is renamed `query_one_energy_type` in Task 12 and Task 13 calls it by that name.
