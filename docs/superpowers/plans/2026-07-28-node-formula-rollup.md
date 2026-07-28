# Node-Formula Roll-up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move consumption formulas off sensors and onto hierarchy nodes, splitting the single mis-named `purpose` axis into `resource` (what a meter measures) and `purpose` (what the energy is spent on), and make the Glue roll-up evaluate those formulas.

**Architecture:** A sensor becomes a raw value carrying only its `Resource`. Hierarchy nodes hold `NodeFormula` items declaring a `(resource, purpose)` output as weighted linear terms over their own descendants. `crates/model` owns the flattening into a `(node, resource, purpose, sensor, coefficient)` weight matrix; the Glue job reads `hierarchy_new` cross-account, rebuilds the same matrix in PySpark, and joins it to `logical_meter_data`. Purely linear terms mean evaluation commutes with hour/day bucketing, so the roll-up keeps its single explode + groupBy shape.

**Tech Stack:** Rust (workspace: `model`, `api`, `services/hierarchy`, `services/aggregations`), maud + HTMX server-rendered HTML, DynamoDB (`hierarchy_new`, `measurements_aggregate`), Scala/Flink on MSF, Iceberg S3 Tables, PySpark on Glue, Go CDK, Astro frontend.

**Spec:** `docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md` — read it before starting. The presentation `docs/hierarchy-presentation.html` is a runnable model of the arithmetic; its numbers are the acceptance values used throughout this plan.

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
- Roll-up sort key after this change: `<node_path>#<resource>#<purpose>#<gran>#<bucket>`. GSI: `gsi1pk = HN2#<id>#<dimension>#<purpose>`, `gsi1sk = <node_path>#<gran>#<bucket>`.

---

# Phase 1 — Domain + hierarchy service

Ships independently: formulas can be authored, validated, listed and rendered. Nothing downstream reads them yet.

---

### Task 1: `Purpose` value type

**Files:**
- Modify: `crates/model/src/domain/values.rs` (append after the `Resource` block, ~line 298)
- Test: `crates/model/src/domain/values.rs` (the existing `mod tests` at the bottom)

**Interfaces:**
- Consumes: `Resource` (already in this file)
- Produces: `Purpose` with `as_str() -> &'static str`, `all() -> impl Iterator<Item = Purpose>`, `declarable() -> bool`, `is_outflow() -> bool`

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

    /// `unallocated` is emitted only by the roll-up job; everything else,
    /// including `total` (which carries weight overrides), may be declared.
    #[test]
    fn purpose_declarable() {
        assert!(!Purpose::Unallocated.declarable());
        assert!(Purpose::Total.declarable());
        assert!(Purpose::Dhw.declarable());
    }

    /// Generation is an outflow — its claims are removed from `total`, not added.
    #[test]
    fn purpose_outflow() {
        assert!(Purpose::Generation.is_outflow());
        assert!(!Purpose::Cooling.is_outflow());
        assert!(!Purpose::Total.is_outflow());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model purpose_ 2>&1 | tail -20`
Expected: FAIL — `cannot find type Purpose in this scope`.

- [ ] **Step 3: Implement `Purpose`**

Insert into `crates/model/src/domain/values.rs` immediately after the `impl Resource { … }` block:

```rust
// ---------------------------------------------------------------------------
// Purpose
// ---------------------------------------------------------------------------

/// The **formål** — what the energy is spent on. Independent of [`Resource`]
/// (the energiart a meter physically measures): electricity serves lighting,
/// cooling and ventilation alike, and space heating can arrive as district
/// heating, gas or a heat pump. The taxonomy follows Energihåndbogen 2019's
/// chapters.
///
/// The string form (`strum` serialize, always lower-case) is the wire/storage
/// contract: it is the `<purpose>` segment of the `measurements_aggregate` sort
/// key and the `formula#<resource>#<purpose>` sort key in `hierarchy_new`.
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
    /// Egenproduktion (PV export). An **outflow**: removed from `Total`, never
    /// added to it, so tariffs and emission factors don't bill exported energy.
    #[strum(serialize = "generation")]
    Generation,
    /// The default Σ series. Declarable, but only to carry **weight overrides**
    /// for meters nested inside other meters (see `logic::formulas`).
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
        !matches!(self, Purpose::Unallocated)
    }

    /// Whether claims of this purpose leave the site rather than being consumed
    /// on it — such sensors contribute 0 to `Total`.
    pub const fn is_outflow(self) -> bool {
        matches!(self, Purpose::Generation)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p model purpose_ 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/model/src/domain/values.rs
git commit -m "feat(model): add Purpose value type (formål axis, separate from Resource)"
```

---

### Task 2: Node-formula domain types; delete sensor formulas; rename `Sensor.purpose` → `resource`

**Files:**
- Create: `crates/model/src/domain/node_formula.rs`
- Delete: `crates/model/src/domain/formula.rs`
- Modify: `crates/model/src/domain/mod.rs`, `crates/model/src/domain/sensor.rs:28-29`
- Modify: `crates/model/src/repository/dynamodb/codec.rs` (sensor item: `formula` attribute, `purpose` attribute)
- Modify: `crates/model/src/logic/sensors.rs` (delete `walk_refs_sync`, `has_cycle`, `set_formula`, `evaluate`; drop the `formula` parameter from `attach`)
- Test: `crates/model/src/domain/node_formula.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `Purpose` (Task 1), `Resource`, `NodeId`, `SensorId`
- Produces:
  - `Reference` — `enum { Sensor(SensorId), Node(NodeId) }`, with `Reference::parse(&str) -> Result<Reference, String>` and `Display`
  - `Term { reference: Reference, coefficient: f64 }`
  - `NodeFormula { node: NodeId, resource: Resource, purpose: Purpose, terms: Vec<Term>, note: Option<String> }`
  - `NodeFormula::sk(&self) -> String` → `"formula#<resource>#<purpose>"`
  - `Sensor.resource` (renamed from `Sensor.purpose`); `Sensor.formula` no longer exists
  - `sensors::attach` loses its `formula: Formula` parameter (was the 6th positional argument)

- [ ] **Step 1: Write the failing tests**

Create `crates/model/src/domain/node_formula.rs` containing only the test module for now:

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
    fn formula_sk_is_resource_then_purpose() {
        let f = NodeFormula {
            node: NodeId::make(Level::Hn4, 30),
            resource: Resource::DistrictHeating,
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
            resource: Resource::DistrictHeating,
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

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model node_formula 2>&1 | tail -20`
Expected: FAIL — the module is not declared in `domain/mod.rs`, then `cannot find type Reference`.

- [ ] **Step 3: Implement the types**

Prepend to `crates/model/src/domain/node_formula.rs` (above the test module):

```rust
//! Node formulas — the replacement for the deleted per-sensor `Formula`.
//!
//! A node declares a `(resource, purpose)` output as a weighted linear
//! combination of **its own descendants**. Terms store only what differs from
//! the default weight of 1, so attaching a meter always moves the numbers and
//! nothing is silently dropped.

use std::fmt;

use crate::domain::ids::{NodeId, SensorId};
use crate::domain::values::{Purpose, Resource};

/// What a term points at: a sensor, or a child node's result for the same resource.
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
/// include = 1, subtract = −1, exclude = 0, apportion = 0.28, COP = 3.2,
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
    pub resource: Resource,
    pub purpose: Purpose,
    pub terms: Vec<Term>,
    pub note: Option<String>,
}

impl NodeFormula {
    /// DynamoDB sort key within the node's own partition.
    pub fn sk(&self) -> String {
        format!("formula#{}#{}", self.resource, self.purpose)
    }
}
```

- [ ] **Step 4: Wire the module and run the tests**

In `crates/model/src/domain/mod.rs`, replace the `pub mod formula;` line with `pub mod node_formula;`.

Run: `cargo test -p model node_formula 2>&1 | tail -20`
Expected: PASS (4 tests). The rest of the crate will not compile yet — that is the next step.

- [ ] **Step 5: Delete sensor formulas and rename the field**

1. `rm crates/model/src/domain/formula.rs`
2. In `crates/model/src/domain/sensor.rs`, delete the `use crate::domain::formula::Formula;` import and these two lines from the struct:

```rust
    #[builder(default = Formula::Identity)]
    pub formula: Formula,
```

   and rename the field `pub purpose: Resource,` to `pub resource: Resource,`.
3. In `crates/model/src/logic/sensors.rs`: delete `walk_refs_sync`, `has_cycle`, `set_formula` and `evaluate` (and their tests); remove the `formula: Formula` parameter from `attach` and the post-allocation cycle check plus its rollback; rename the `purpose: Resource` parameter to `resource: Resource`.
4. In `crates/model/src/repository/dynamodb/codec.rs`: drop the `formula` attribute from `sensor_to_item`/`sensor_of_item`, and rename the DynamoDB attribute `purpose` → `resource` on the sensor item.
5. Fix the fallout the compiler points at across `crates/services/hierarchy` (`json.rs`, `dispatch.rs`, `command.rs`, `html/forms.rs`, `html/node.rs`) — Task 7 rewrites the UI properly; here just delete the formula plumbing and rename `purpose` → `resource` so the workspace builds.

Run: `cargo build 2>&1 | tail -30`
Expected: clean build.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS. Any test still referencing `Formula` or `sensor.purpose` should be deleted or renamed, not adapted — sensor formulas are gone.

- [ ] **Step 7: Commit**

```bash
git add -A crates/
git commit -m "feat(model)!: node-formula types; delete sensor formulas; Sensor.purpose -> Sensor.resource"
```

---

### Task 3: Flattening — `logic/formulas.rs`

**Files:**
- Create: `crates/model/src/logic/formulas.rs`
- Modify: `crates/model/src/logic/mod.rs` (add `pub mod formulas;`)
- Test: `crates/model/src/logic/formulas.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `NodeFormula`, `Term`, `Reference` (Task 2), `Purpose` (Task 1), `Node`, `Sensor`
- Produces:
  - `CompanyGraph { nodes: Vec<Node>, sensors: Vec<Sensor>, formulas: Vec<NodeFormula> }`
  - `WeightRow { declaring_node: String, resource: Resource, purpose: Purpose, sensor: SensorId, coefficient: f64, derived: bool }`
  - `flatten(&CompanyGraph) -> Vec<WeightRow>`
  - `total_weight(&CompanyGraph, &Sensor) -> f64`
  - `is_derived(&CompanyGraph, &NodeFormula) -> bool`

**Semantics being implemented (spec §3.6–§3.8):**
- A term referencing a **sensor** emits one row.
- A term referencing a **node** expands to every sensor under that node whose `resource` equals the formula's `resource`, each at `coefficient × total_weight(sensor)`.
- `total_weight(sensor)` = 0 if the sensor is claimed by an outflow purpose at any ancestor; else the **deepest** declared `Total` override mentioning it at any ancestor; else 1.
- `is_derived(formula)` = any referenced **sensor**'s resource differs from the formula's output resource. Node references never make a formula derived (a node's result is already in the formula's resource).

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

    fn sensor(id: u32, path: &str, resource: Resource) -> Sensor {
        Sensor::builder()
            .id(SensorId::make(id))
            .daq_id(format!("daq{id}"))
            .path(format!("{path}|S#{id}"))
            .resource(resource)
            .meter_type(MeterType::Counter)
            .build()
    }

    fn term(r: Reference, c: f64) -> Term {
        Term { reference: r, coefficient: c }
    }

    /// Chiller: the accumulator already contains the three phase meters, so the
    /// phases are overridden to weight 0 in `total`. Without the override the
    /// node would read 80 kWh instead of 40.
    fn chiller_graph() -> CompanyGraph {
        let a_path = format!("{CO}|HN5#5");
        CompanyGraph {
            nodes: vec![node(Level::Hn2, 997, CO), node(Level::Hn5, 5, &a_path)],
            sensors: vec![
                sensor(1, &a_path, Resource::Electricity), // accumulator, 40
                sensor(2, &a_path, Resource::Electricity), // phase, 13
                sensor(3, &a_path, Resource::Electricity), // phase, 14
            ],
            formulas: vec![
                NodeFormula {
                    node: NodeId::make(Level::Hn5, 5),
                    resource: Resource::Electricity,
                    purpose: Purpose::Total,
                    terms: vec![
                        term(Reference::Sensor(SensorId::make(2)), 0.0),
                        term(Reference::Sensor(SensorId::make(3)), 0.0),
                    ],
                    note: Some("phases are inside the accumulator".to_string()),
                },
                NodeFormula {
                    node: NodeId::make(Level::Hn5, 5),
                    resource: Resource::Electricity,
                    purpose: Purpose::Cooling,
                    terms: vec![term(Reference::Sensor(SensorId::make(1)), 1.0)],
                    note: None,
                },
            ],
        }
    }

    #[test]
    fn unlisted_sensors_default_to_weight_one() {
        let g = chiller_graph();
        let acc = g.sensors.iter().find(|s| s.id == SensorId::make(1)).unwrap();
        assert_eq!(total_weight(&g, acc), 1.0);
    }

    #[test]
    fn total_override_zeroes_nested_meters() {
        let g = chiller_graph();
        for id in [2u32, 3] {
            let s = g.sensors.iter().find(|s| s.id == SensorId::make(id)).unwrap();
            assert_eq!(total_weight(&g, s), 0.0, "phase {id} is inside the accumulator");
        }
    }

    /// A `total` override declared on a node applies to that node AND every
    /// ancestor — a meter nested inside another is nested all the way up.
    #[test]
    fn total_override_propagates_to_ancestors() {
        let g = chiller_graph();
        let phase = g.sensors.iter().find(|s| s.id == SensorId::make(2)).unwrap();
        // The override is declared on HN5#5; the sensor hangs off HN5#5, so every
        // ancestor path (company included) sees weight 0.
        assert_eq!(total_weight(&g, phase), 0.0);
    }

    /// Generation is an outflow: claimed sensors leave `total` entirely.
    #[test]
    fn generation_claim_removes_sensor_from_total() {
        let a_path = format!("{CO}|HN5#7");
        let g = CompanyGraph {
            nodes: vec![node(Level::Hn2, 997, CO), node(Level::Hn5, 7, &a_path)],
            sensors: vec![
                sensor(10, &a_path, Resource::Electricity), // import
                sensor(11, &a_path, Resource::Electricity), // export
            ],
            formulas: vec![NodeFormula {
                node: NodeId::make(Level::Hn5, 7),
                resource: Resource::Electricity,
                purpose: Purpose::Generation,
                terms: vec![term(Reference::Sensor(SensorId::make(11)), 1.0)],
                note: None,
            }],
        };
        let imp = g.sensors.iter().find(|s| s.id == SensorId::make(10)).unwrap();
        let exp = g.sensors.iter().find(|s| s.id == SensorId::make(11)).unwrap();
        assert_eq!(total_weight(&g, imp), 1.0);
        assert_eq!(total_weight(&g, exp), 0.0);
    }

    #[test]
    fn flatten_emits_one_row_per_sensor_term() {
        let rows = flatten(&chiller_graph());
        let cooling: Vec<_> = rows.iter().filter(|r| r.purpose == Purpose::Cooling).collect();
        assert_eq!(cooling.len(), 1);
        assert_eq!(cooling[0].sensor, SensorId::make(1));
        assert_eq!(cooling[0].coefficient, 1.0);
        assert_eq!(cooling[0].declaring_node, format!("{CO}|HN5#5"));
        assert!(!cooling[0].derived);
    }

    /// A node reference expands to that node's sensors of the same resource,
    /// each scaled by the term coefficient AND its own total weight.
    #[test]
    fn node_reference_expands_to_weighted_descendants() {
        let mut g = chiller_graph();
        let b_path = format!("{CO}|HN4#4");
        g.nodes.push(node(Level::Hn4, 4, &b_path));
        g.formulas.push(NodeFormula {
            node: NodeId::make(Level::Hn4, 4),
            resource: Resource::Electricity,
            purpose: Purpose::Process,
            terms: vec![term(Reference::Node(NodeId::make(Level::Hn5, 5)), 0.5)],
            note: None,
        });
        // HN5#5 is not under HN4#4 in this fixture's paths, so widen the area path.
        let rows = flatten(&g);
        let process: Vec<_> = rows.iter().filter(|r| r.purpose == Purpose::Process).collect();
        // Only the accumulator survives: the phases carry total weight 0.
        assert_eq!(process.len(), 1);
        assert_eq!(process[0].sensor, SensorId::make(1));
        assert_eq!(process[0].coefficient, 0.5);
    }

    /// Output resource differing from the referenced sensors' resource marks the
    /// row derived — gas m³ × brændværdi × virkningsgrad is delivered heat, not
    /// metered consumption, so it must never fold into a total.
    #[test]
    fn cross_resource_formula_is_derived() {
        let b_path = format!("{CO}|HN4#8");
        let g = CompanyGraph {
            nodes: vec![node(Level::Hn2, 997, CO), node(Level::Hn4, 8, &b_path)],
            sensors: vec![sensor(20, &b_path, Resource::Gas)],
            formulas: vec![NodeFormula {
                node: NodeId::make(Level::Hn4, 8),
                resource: Resource::Heat,
                purpose: Purpose::SpaceHeating,
                terms: vec![term(Reference::Sensor(SensorId::make(20)), 10.45)],
                note: Some("brændværdi 11,0 kWh/m³ × virkningsgrad 0,95".to_string()),
            }],
        };
        assert!(is_derived(&g, &g.formulas[0]));
        assert!(flatten(&g).iter().all(|r| r.derived));
    }

    #[test]
    fn same_resource_formula_is_not_derived() {
        let g = chiller_graph();
        assert!(!is_derived(&g, &g.formulas[1]));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model formulas:: 2>&1 | tail -20`
Expected: FAIL — `cannot find type CompanyGraph`.

- [ ] **Step 3: Implement the flattener**

Prepend to `crates/model/src/logic/formulas.rs`:

```rust
//! Flattening node formulas into a per-`(node, resource, purpose, sensor)`
//! weight matrix.
//!
//! Pure functions over an in-memory company graph — no effects, no repository
//! access. The Glue roll-up job re-implements exactly these rules in PySpark
//! (see `infra/daq/data_pipeline/glue/hierarchy_matrix.py`); a golden fixture
//! keeps the two honest.

use crate::domain::ids::{NodeId, SensorId};
use crate::domain::node::{Node, PATH_SEP};
use crate::domain::node_formula::{NodeFormula, Reference, Term};
use crate::domain::sensor::Sensor;
use crate::domain::values::{Purpose, Resource};

/// Everything under one company (HN2) needed to evaluate its formulas.
#[derive(Clone, Debug, Default)]
pub struct CompanyGraph {
    pub nodes: Vec<Node>,
    pub sensors: Vec<Sensor>,
    pub formulas: Vec<NodeFormula>,
}

/// One `(declaring node, resource, purpose, sensor)` weight. The roll-up job
/// multiplies each meter reading by `coefficient` and groups by the declaring
/// node's ancestor paths.
#[derive(Clone, Debug, PartialEq)]
pub struct WeightRow {
    pub declaring_node: String,
    pub resource: Resource,
    pub purpose: Purpose,
    pub sensor: SensorId,
    pub coefficient: f64,
    pub derived: bool,
}

impl CompanyGraph {
    fn node_path(&self, id: &NodeId) -> Option<&str> {
        self.nodes.iter().find(|n| &n.id == id).map(|n| n.path.as_str())
    }

    fn sensor(&self, id: SensorId) -> Option<&Sensor> {
        self.sensors.iter().find(|s| s.id == id)
    }

    /// Sensors whose path lies under `node_path` (inclusive of the node itself).
    fn sensors_under(&self, node_path: &str) -> impl Iterator<Item = &Sensor> {
        let prefix = format!("{node_path}{PATH_SEP}");
        self.sensors
            .iter()
            .filter(move |s| s.path.starts_with(&prefix))
    }
}

/// True when `descendant_path` is at or below `ancestor_path`.
fn is_at_or_under(descendant_path: &str, ancestor_path: &str) -> bool {
    descendant_path == ancestor_path
        || descendant_path.starts_with(&format!("{ancestor_path}{PATH_SEP}"))
}

/// A sensor's weight in the `total` series (spec §3.8):
/// 0 if claimed by an outflow purpose at any ancestor; else the **deepest**
/// declared `Total` override that mentions it; else the default 1.
pub fn total_weight(g: &CompanyGraph, s: &Sensor) -> f64 {
    let mentions = |f: &NodeFormula| {
        f.terms
            .iter()
            .any(|t| t.reference == Reference::Sensor(s.id))
    };
    let governs = |f: &NodeFormula| {
        g.node_path(&f.node)
            .is_some_and(|p| is_at_or_under(&s.path, p))
    };

    if g.formulas
        .iter()
        .any(|f| f.purpose.is_outflow() && governs(f) && mentions(f))
    {
        return 0.0;
    }

    g.formulas
        .iter()
        .filter(|f| {
            f.purpose == Purpose::Total && f.resource == s.resource && governs(f) && mentions(f)
        })
        // Deepest declaration wins — a longer path is further down the tree.
        .max_by_key(|f| g.node_path(&f.node).map_or(0, str::len))
        .and_then(|f| {
            f.terms
                .iter()
                .find(|t| t.reference == Reference::Sensor(s.id))
                .map(|t| t.coefficient)
        })
        .unwrap_or(1.0)
}

/// A formula is derived when its declared output resource differs from the
/// resource of any sensor it references directly. Node references resolve to
/// the formula's own resource, so they never make it derived.
pub fn is_derived(g: &CompanyGraph, f: &NodeFormula) -> bool {
    f.terms.iter().any(|t| match &t.reference {
        Reference::Sensor(id) => g.sensor(*id).is_some_and(|s| s.resource != f.resource),
        Reference::Node(_) => false,
    })
}

/// Expand one term into `(sensor, coefficient)` pairs.
fn expand<'a>(
    g: &'a CompanyGraph,
    f: &'a NodeFormula,
    t: &'a Term,
) -> Vec<(SensorId, f64)> {
    match &t.reference {
        Reference::Sensor(id) => vec![(*id, t.coefficient)],
        Reference::Node(id) => match g.node_path(id) {
            None => vec![],
            Some(path) => g
                .sensors_under(path)
                .filter(|s| s.resource == f.resource)
                .map(|s| (s.id, t.coefficient * total_weight(g, s)))
                .filter(|(_, c)| *c != 0.0)
                .collect(),
        },
    }
}

/// Flatten every declared formula into weight rows. `Total` formulas are weight
/// overrides consumed by [`total_weight`], not claims, so they are not emitted.
pub fn flatten(g: &CompanyGraph) -> Vec<WeightRow> {
    let mut out = Vec::new();
    for f in g.formulas.iter().filter(|f| f.purpose != Purpose::Total) {
        let Some(declaring_node) = g.node_path(&f.node) else {
            continue;
        };
        let derived = is_derived(g, f);
        for t in &f.terms {
            for (sensor, coefficient) in expand(g, f, t) {
                out.push(WeightRow {
                    declaring_node: declaring_node.to_string(),
                    resource: f.resource,
                    purpose: f.purpose,
                    sensor,
                    coefficient,
                    derived,
                });
            }
        }
    }
    out
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

### Task 4: Formula validation

**Files:**
- Modify: `crates/model/src/logic/formulas.rs` (append `validate` + tests)

**Interfaces:**
- Consumes: `CompanyGraph`, `NodeFormula` (Task 3)
- Produces: `validate(&CompanyGraph, &NodeFormula) -> Result<(), String>`

- [ ] **Step 1: Write the failing tests**

Append inside the existing `mod tests` in `crates/model/src/logic/formulas.rs`:

```rust
    fn ok_formula() -> NodeFormula {
        NodeFormula {
            node: NodeId::make(Level::Hn5, 5),
            resource: Resource::Electricity,
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
    fn validate_rejects_unallocated() {
        let f = NodeFormula { purpose: Purpose::Unallocated, ..ok_formula() };
        assert!(validate(&chiller_graph(), &f).unwrap_err().contains("unallocated"));
    }

    #[test]
    fn validate_rejects_zero_coefficient_on_a_claim() {
        let f = NodeFormula {
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 0.0)],
            ..ok_formula()
        };
        assert!(validate(&chiller_graph(), &f).unwrap_err().contains("non-zero"));
    }

    /// Weight 0 is the whole point of a `total` override.
    #[test]
    fn validate_allows_zero_coefficient_on_a_total_override() {
        let f = NodeFormula {
            purpose: Purpose::Total,
            terms: vec![term(Reference::Sensor(SensorId::make(2)), 0.0)],
            ..ok_formula()
        };
        assert!(validate(&chiller_graph(), &f).is_ok());
    }

    #[test]
    fn validate_rejects_non_finite_coefficient() {
        let f = NodeFormula {
            terms: vec![term(Reference::Sensor(SensorId::make(1)), f64::NAN)],
            ..ok_formula()
        };
        assert!(validate(&chiller_graph(), &f).unwrap_err().contains("finite"));
    }

    /// The subtree rule: a formula may only reference its own descendants. This
    /// is what makes the reference graph acyclic and reparenting safe.
    #[test]
    fn validate_rejects_reference_outside_the_subtree() {
        let mut g = chiller_graph();
        let other = format!("{CO}|HN5#99");
        g.nodes.push(node(Level::Hn5, 99, &other));
        g.sensors.push(sensor(77, &other, Resource::Electricity));
        let f = NodeFormula {
            terms: vec![term(Reference::Sensor(SensorId::make(77)), 1.0)],
            ..ok_formula()
        };
        assert!(validate(&g, &f).unwrap_err().contains("descendant"));
    }

    /// A sensor may be claimed for one (resource, purpose) by ONE node only —
    /// otherwise a shared ancestor double counts it when claims propagate up.
    #[test]
    fn validate_rejects_a_sensor_claimed_twice_for_the_same_purpose() {
        let mut g = chiller_graph();
        let sibling = format!("{CO}|HN5#5|HN6#6");
        g.nodes.push(node(Level::Hn6, 6, &sibling));
        let f = NodeFormula {
            node: NodeId::make(Level::Hn6, 6),
            resource: Resource::Electricity,
            purpose: Purpose::Cooling, // already claimed by HN5#5 in chiller_graph()
            terms: vec![term(Reference::Sensor(SensorId::make(1)), 1.0)],
            note: None,
        };
        let err = validate(&g, &f).unwrap_err();
        assert!(err.contains("already claimed"), "got: {err}");
    }

    /// Re-declaring the SAME (node, resource, purpose) is an upsert, not a conflict.
    #[test]
    fn validate_allows_upserting_the_same_formula() {
        let g = chiller_graph();
        assert!(validate(&g, &g.formulas[1].clone()).is_ok());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model validate_ 2>&1 | tail -20`
Expected: FAIL — `cannot find function validate`.

- [ ] **Step 3: Implement `validate`**

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
        if f.purpose != Purpose::Total && t.coefficient == 0.0 {
            return Err(format!(
                "coefficient for {} must be non-zero (weight 0 belongs on a total override)",
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

    // One claim per sensor per (resource, purpose) across the company. Re-declaring
    // the same (node, resource, purpose) is an upsert and never conflicts.
    if f.purpose != Purpose::Total {
        for t in &f.terms {
            let Reference::Sensor(id) = &t.reference else {
                continue;
            };
            if let Some(other) = g.formulas.iter().find(|o| {
                o.resource == f.resource
                    && o.purpose == f.purpose
                    && o.node != f.node
                    && o.terms.iter().any(|ot| ot.reference == t.reference)
            }) {
                return Err(format!(
                    "sensor {} is already claimed for {}/{} by node {}",
                    id, f.resource, f.purpose, other.node
                ));
            }
        }
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
git commit -m "feat(model): validate node formulas (subtree rule, one claim per sensor)"
```

---

### Task 5: Formula persistence

**Files:**
- Create: `crates/model/src/repository/dynamodb/node_formula.rs`
- Modify: `crates/model/src/repository/dynamodb/mod.rs`, `crates/model/src/repository/dynamodb/codec.rs`, `crates/model/src/repository/memory.rs`
- Test: `crates/model/src/repository/dynamodb/codec.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `NodeFormula` (Task 2)
- Produces:
  - `codec::node_formula_to_item(&NodeFormula, node_path: &str, company_path: &str) -> Item`
  - `codec::node_formula_of_item(&Item) -> Result<NodeFormula, CodecError>`
  - `codec::formula_gsi1pk(company_path: &str) -> String` → `"F#HN2#<id>"`
  - `node_formula::put_node_formula(client, table, &NodeFormula, node_path, company_path) -> Result<(), RepositoryError>`
  - `node_formula::delete_node_formula(client, table, &NodeId, Resource, Purpose) -> Result<(), RepositoryError>`
  - `node_formula::list_node_formulas(client, table, &NodeId) -> Result<Vec<NodeFormula>, RepositoryError>`
  - `node_formula::list_company_formulas(client, table, company_path) -> Result<Vec<NodeFormula>, RepositoryError>`

**Item shape (spec §4):**

| Field | Value |
|---|---|
| `pk` | `<NodeId>` — same partition as the node item |
| `sk` | `formula#<resource>#<purpose>` |
| `gsi1pk` | `F#HN2#<company_id>` |
| `gsi1sk` | `<node_path>#<resource>#<purpose>` |
| `terms` | `L` of `M{ ref: S, coefficient: N }` |
| `note` | `S`, optional |
| `created` / `updated` | `S` RFC 3339 |

- [ ] **Step 1: Write the failing codec test**

Add to `mod tests` in `crates/model/src/repository/dynamodb/codec.rs`:

```rust
    #[test]
    fn node_formula_item_round_trips() {
        use crate::domain::node_formula::{NodeFormula, Reference, Term};
        use crate::domain::values::Purpose;

        let f = NodeFormula {
            node: NodeId::make(Level::Hn4, 30),
            resource: Resource::DistrictHeating,
            purpose: Purpose::SpaceHeating,
            terms: vec![
                Term { reference: Reference::Sensor(SensorId::make(1)), coefficient: 1.0 },
                Term { reference: Reference::Sensor(SensorId::make(2)), coefficient: -1.0 },
            ],
            note: Some("bimåler".to_string()),
        };
        let node_path = "HN0#root|HN1#1|HN2#997|HN3#3|HN4#30";
        let item = node_formula_to_item(&f, node_path, "HN0#root|HN1#1|HN2#997");

        assert_eq!(
            item.get("sk").and_then(|v| v.as_s().ok()).map(String::as_str),
            Some("formula#district_heating#space_heating")
        );
        assert_eq!(
            item.get("gsi1pk").and_then(|v| v.as_s().ok()).map(String::as_str),
            Some("F#HN2#997")
        );
        assert_eq!(
            item.get("gsi1sk").and_then(|v| v.as_s().ok()).map(String::as_str),
            Some("HN0#root|HN1#1|HN2#997|HN3#3|HN4#30#district_heating#space_heating")
        );
        assert_eq!(node_formula_of_item(&item).unwrap(), f);
    }

    #[test]
    fn node_formula_item_without_note_round_trips() {
        use crate::domain::node_formula::{NodeFormula, Reference, Term};
        use crate::domain::values::Purpose;

        let f = NodeFormula {
            node: NodeId::make(Level::Hn5, 5),
            resource: Resource::Electricity,
            purpose: Purpose::Total,
            terms: vec![Term {
                reference: Reference::Node(NodeId::make(Level::Hn6, 6)),
                coefficient: 0.0,
            }],
            note: None,
        };
        let item = node_formula_to_item(&f, "HN0#root|HN2#997|HN5#5", "HN0#root|HN2#997");
        assert_eq!(node_formula_of_item(&item).unwrap(), f);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model node_formula_item 2>&1 | tail -20`
Expected: FAIL — `cannot find function node_formula_to_item`.

- [ ] **Step 3: Implement the codec**

Append to `crates/model/src/repository/dynamodb/codec.rs`:

```rust
// ---------------------------------------------------------------------------
// Node formulas
// ---------------------------------------------------------------------------

use crate::domain::node_formula::{NodeFormula, Reference, Term};
use crate::domain::values::Purpose;

/// `gsi1pk = "F#HN2#<id>"` — one partition per company, so the roll-up job
/// loads a whole company's matrix in a single query.
pub(crate) fn formula_gsi1pk(company_path: &str) -> String {
    let hn2 = company_path
        .split(crate::domain::node::PATH_SEP)
        .find(|s| s.starts_with("HN2#"))
        .unwrap_or("HN2#0");
    format!("F#{hn2}")
}

pub fn node_formula_to_item(f: &NodeFormula, node_path: &str, company_path: &str) -> Item {
    let now = chrono::Utc::now().to_rfc3339();
    let terms: Vec<AttributeValue> = f
        .terms
        .iter()
        .map(|t| {
            let mut m = std::collections::HashMap::new();
            m.insert("ref".to_string(), s(t.reference.to_string()));
            m.insert(
                "coefficient".to_string(),
                AttributeValue::N(t.coefficient.to_string()),
            );
            AttributeValue::M(m)
        })
        .collect();

    let mut item: Item = std::collections::HashMap::new();
    item.insert("pk".to_string(), s(f.node.to_string()));
    item.insert("sk".to_string(), s(f.sk()));
    item.insert("gsi1pk".to_string(), s(formula_gsi1pk(company_path)));
    item.insert(
        "gsi1sk".to_string(),
        s(format!("{node_path}#{}#{}", f.resource, f.purpose)),
    );
    item.insert("resource".to_string(), s(f.resource.to_string()));
    item.insert("purpose".to_string(), s(f.purpose.to_string()));
    item.insert("terms".to_string(), AttributeValue::L(terms));
    if let Some(n) = &f.note {
        item.insert("note".to_string(), s(n.clone()));
    }
    item.insert("updated".to_string(), s(now));
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
    let resource: Resource = get("resource")?.parse().map_err(|_| {
        CodecError::from(format!("bad resource {:?}", item.get("resource")))
    })?;
    let purpose: Purpose = get("purpose")?
        .parse()
        .map_err(|_| CodecError::from(format!("bad purpose {:?}", item.get("purpose"))))?;

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
        resource,
        purpose,
        terms,
        note: item.get("note").and_then(|v| v.as_s().ok()).cloned(),
    })
}
```

Reuse whatever `s(..)`, `Item` and `CodecError` helpers already exist in this file rather than redeclaring them; adjust the `use` lines to match the file's existing imports.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p model node_formula_item 2>&1 | tail -20`
Expected: PASS (2 tests).

- [ ] **Step 5: Implement the DynamoDB adapter**

Create `crates/model/src/repository/dynamodb/node_formula.rs`:

```rust
//! Formula items live in the node's own partition (`pk = <NodeId>`), indexed by
//! `gsi1pk = "F#HN2#<id>"` so a whole company's matrix is one query.

use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client;

use crate::domain::ids::NodeId;
use crate::domain::node_formula::NodeFormula;
use crate::domain::values::{Purpose, Resource};
use crate::errors::RepositoryError;
use crate::repository::dynamodb::codec;

const FORMULA_SK_PREFIX: &str = "formula#";

pub async fn put_node_formula(
    client: &Client,
    table: &str,
    f: &NodeFormula,
    node_path: &str,
    company_path: &str,
) -> Result<(), RepositoryError> {
    client
        .put_item()
        .table_name(table)
        .set_item(Some(codec::node_formula_to_item(f, node_path, company_path)))
        .send()
        .await
        .map(|_| ())
        .map_err(|e| RepositoryError::Aws(format!("put_node_formula: {e:?}")))
}

pub async fn delete_node_formula(
    client: &Client,
    table: &str,
    node: &NodeId,
    resource: Resource,
    purpose: Purpose,
) -> Result<(), RepositoryError> {
    client
        .delete_item()
        .table_name(table)
        .key("pk", AttributeValue::S(node.to_string()))
        .key("sk", AttributeValue::S(format!("{FORMULA_SK_PREFIX}{resource}#{purpose}")))
        .send()
        .await
        .map(|_| ())
        .map_err(|e| RepositoryError::Aws(format!("delete_node_formula: {e:?}")))
}

pub async fn list_node_formulas(
    client: &Client,
    table: &str,
    node: &NodeId,
) -> Result<Vec<NodeFormula>, RepositoryError> {
    let rows = client
        .query()
        .table_name(table)
        .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
        .expression_attribute_names("#pk", "pk")
        .expression_attribute_names("#sk", "sk")
        .expression_attribute_values(":pk", AttributeValue::S(node.to_string()))
        .expression_attribute_values(":sk", AttributeValue::S(FORMULA_SK_PREFIX.to_string()))
        .into_paginator()
        .items()
        .send()
        .collect::<Result<Vec<_>, _>>()
        .await
        .map_err(|e| RepositoryError::Aws(format!("list_node_formulas: {e:?}")))?;

    Ok(rows.iter().filter_map(|i| codec::node_formula_of_item(i).ok()).collect())
}

pub async fn list_company_formulas(
    client: &Client,
    table: &str,
    company_path: &str,
) -> Result<Vec<NodeFormula>, RepositoryError> {
    let rows = client
        .query()
        .table_name(table)
        .index_name("gsi1")
        .key_condition_expression("#pk = :pk")
        .expression_attribute_names("#pk", "gsi1pk")
        .expression_attribute_values(
            ":pk",
            AttributeValue::S(codec::formula_gsi1pk(company_path)),
        )
        .into_paginator()
        .items()
        .send()
        .collect::<Result<Vec<_>, _>>()
        .await
        .map_err(|e| RepositoryError::Aws(format!("list_company_formulas: {e:?}")))?;

    Ok(rows.iter().filter_map(|i| codec::node_formula_of_item(i).ok()).collect())
}
```

Add `pub mod node_formula;` to `crates/model/src/repository/dynamodb/mod.rs`, and mirror the four functions in `crates/model/src/repository/memory.rs` over its in-memory store (following the shape of the existing sensor helpers there).

- [ ] **Step 6: Verify the workspace builds and tests pass**

Run: `cargo test -p model 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/model/src/repository/
git commit -m "feat(model): persist node formulas as first-class items with an F#HN2 GSI"
```

---

### Task 6: `set_node_formula` / `delete_node_formula` commands

**Files:**
- Modify: `crates/services/hierarchy/src/command.rs:135` (add two variants before the closing brace of `enum Command`)
- Modify: `crates/services/hierarchy/src/dispatch.rs` (add two arms to `run`, plus handlers)
- Modify: `crates/services/hierarchy/src/repo_fns.rs` (add closure factories)
- Test: `crates/services/hierarchy/src/command.rs` (`mod tests`), `crates/services/hierarchy/src/dispatch.rs` (`mod tests`)

**Interfaces:**
- Consumes: `validate` (Task 4), `put_node_formula` / `delete_node_formula` / `list_company_formulas` (Task 5)
- Produces:
  - `Command::SetNodeFormula { node_id, resource, purpose, terms: Value, note: Option<String> }`
  - `Command::DeleteNodeFormula { node_id, resource, purpose }`
  - `dispatch::handle_set_node_formula(..) -> Value`, `dispatch::handle_delete_node_formula(..) -> Value`

**Wire format** — `terms` is accepted as either a JSON array (programmatic callers) or a JSON-encoded string (the HTML form, which builds it client-side to avoid dynamic field names):

```
action=set_node_formula
node_id=HN4#30
resource=district_heating
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
            "resource": "district_heating",
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
                    &resource=electricity\
                    &purpose=total\
                    &terms=%5B%7B%22ref%22%3A%22S%232%22%2C%22coefficient%22%3A0%7D%5D";
        let cmd = parse_command(form, Some("application/x-www-form-urlencoded")).unwrap();
        assert!(matches!(&cmd, Command::SetNodeFormula { terms, .. }
                         if terms.is_string() || terms.is_array()));
    }

    #[test]
    fn parse_delete_node_formula() {
        let form = "action=delete_node_formula&node_id=HN4%2330\
                    &resource=district_heating&purpose=dhw";
        let cmd = parse_command(form, Some("application/x-www-form-urlencoded")).unwrap();
        assert!(matches!(&cmd, Command::DeleteNodeFormula { purpose, .. } if purpose == "dhw"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p hierarchy set_node_formula 2>&1 | tail -20`
Expected: FAIL — `no variant named SetNodeFormula`.

- [ ] **Step 3: Add the command variants**

Insert into `enum Command` in `crates/services/hierarchy/src/command.rs`, before the closing brace:

```rust
    /// `set_node_formula` — upsert a node's `(resource, purpose)` formula.
    /// `terms` is a JSON array of `{ref, coefficient}`, or a JSON-encoded string
    /// of the same (the HTML form builds it client-side).
    SetNodeFormula {
        node_id: String,
        resource: String,
        purpose: String,
        terms: Value,
        #[serde(default)]
        note: Option<String>,
    },

    /// `delete_node_formula` — remove a node's `(resource, purpose)` formula.
    DeleteNodeFormula {
        node_id: String,
        resource: String,
        purpose: String,
    },
```

- [ ] **Step 4: Run parse tests to verify they pass**

Run: `cargo test -p hierarchy node_formula 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 5: Write the failing handler tests**

Add to `mod tests` in `crates/services/hierarchy/src/dispatch.rs`, following the in-memory-repo style already used there:

```rust
    /// A well-formed formula is stored and echoed back.
    #[tokio::test]
    async fn set_node_formula_stores_the_formula() {
        let store = memory_store_with_company();  // existing helper in this test module
        let out = handle_set_node_formula(
            "HN5#5".to_string(),
            "electricity".to_string(),
            "cooling".to_string(),
            serde_json::json!([{"ref": "S#1", "coefficient": 1}]),
            None,
            store.get_node_fn(),
            store.list_company_formulas_fn(),
            store.list_company_sensors_fn(),
            store.list_company_nodes_fn(),
            store.put_node_formula_fn(),
        )
        .await;
        assert_eq!(out["ok"], serde_json::json!(true));
        assert_eq!(out["formula"]["purpose"], serde_json::json!("cooling"));
    }

    /// The subtree rule is enforced at the command boundary, not just in the UI.
    #[tokio::test]
    async fn set_node_formula_rejects_out_of_subtree_reference() {
        let store = memory_store_with_company();
        let out = handle_set_node_formula(
            "HN5#5".to_string(),
            "electricity".to_string(),
            "cooling".to_string(),
            serde_json::json!([{"ref": "S#77", "coefficient": 1}]),
            None,
            store.get_node_fn(),
            store.list_company_formulas_fn(),
            store.list_company_sensors_fn(),
            store.list_company_nodes_fn(),
            store.put_node_formula_fn(),
        )
        .await;
        assert_eq!(out["error"]["code"], serde_json::json!("Validation"));
    }

    #[tokio::test]
    async fn set_node_formula_rejects_malformed_terms() {
        let store = memory_store_with_company();
        let out = handle_set_node_formula(
            "HN5#5".to_string(),
            "electricity".to_string(),
            "cooling".to_string(),
            serde_json::json!("not-json"),
            None,
            store.get_node_fn(),
            store.list_company_formulas_fn(),
            store.list_company_sensors_fn(),
            store.list_company_nodes_fn(),
            store.put_node_formula_fn(),
        )
        .await;
        assert_eq!(out["error"]["code"], serde_json::json!("Bad_request"));
    }
```

If `memory_store_with_company` and the `*_fn()` accessors don't exist in the test module yet, add them alongside the existing in-memory helpers; they must build a company containing `HN5#5` with sensors `S#1` (under `HN5#5`) and `S#77` (under a sibling node).

- [ ] **Step 6: Run handler tests to verify they fail**

Run: `cargo test -p hierarchy set_node_formula_ 2>&1 | tail -20`
Expected: FAIL — `cannot find function handle_set_node_formula`.

- [ ] **Step 7: Implement the handlers**

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

#[allow(clippy::too_many_arguments)]
pub async fn handle_set_node_formula<FGN, FGNFut, FLF, FLFFut, FLS, FLSFut, FLN, FLNFut, FPF, FPFFut>(
    node_id: String,
    resource: String,
    purpose: String,
    terms: Value,
    note: Option<String>,
    get_node: FGN,
    list_company_formulas: FLF,
    list_company_sensors: FLS,
    list_company_nodes: FLN,
    put_node_formula: FPF,
) -> Value
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FLF: FnOnce(String) -> FLFFut,
    FLFFut: Future<Output = Result<Vec<NodeFormula>, RepositoryError>>,
    FLS: FnOnce(String) -> FLSFut,
    FLSFut: Future<Output = Result<Vec<Sensor>, RepositoryError>>,
    FLN: FnOnce(String) -> FLNFut,
    FLNFut: Future<Output = Result<Vec<Node>, RepositoryError>>,
    FPF: FnOnce(NodeFormula, String, String) -> FPFFut,
    FPFFut: Future<Output = Result<(), RepositoryError>>,
{
    let (node, resource, purpose, terms) = match (
        NodeId::parse(&node_id),
        resource.parse::<Resource>(),
        purpose.parse::<Purpose>(),
        parse_terms(&terms),
    ) {
        (Ok(n), Ok(r), Ok(p), Ok(t)) => (n, r, p, t),
        (n, r, p, t) => {
            let msg = n.err().map(|e| e.to_string())
                .or_else(|| r.err().map(|_| format!("unknown resource {resource:?}")))
                .or_else(|| p.err().map(|_| format!("unknown purpose {purpose:?}")))
                .or_else(|| t.err())
                .unwrap_or_default();
            return bad_request(msg);
        }
    };

    let Ok(Some(node_row)) = get_node(node.clone()).await else {
        return repo_error_response(&RepositoryError::NotFound(node));
    };
    let Some(company_path) = company_prefix_of_path(&node_row.path) else {
        return bad_request(format!("{node} has no HN2 company ancestor"));
    };

    let (formulas, sensors, nodes) = match futures::try_join!(
        list_company_formulas(company_path.clone()),
        list_company_sensors(company_path.clone()),
        list_company_nodes(company_path.clone()),
    ) {
        Ok(t) => t,
        Err(e) => return repo_error_response(&e),
    };

    let f = NodeFormula { node, resource, purpose, terms, note };
    let graph = CompanyGraph { nodes, sensors, formulas };
    if let Err(msg) = formulas_logic::validate(&graph, &f) {
        return validation_error_response(msg);
    }
    match put_node_formula(f.clone(), node_row.path.clone(), company_path).await {
        Ok(()) => serde_json::json!({ "ok": true, "formula": formula_to_json(&f) }),
        Err(e) => repo_error_response(&e),
    }
}

pub async fn handle_delete_node_formula<FDF, FDFFut>(
    node_id: String,
    resource: String,
    purpose: String,
    delete_node_formula: FDF,
) -> Value
where
    FDF: FnOnce(NodeId, Resource, Purpose) -> FDFFut,
    FDFFut: Future<Output = Result<(), RepositoryError>>,
{
    let (node, resource, purpose) = match (
        NodeId::parse(&node_id),
        resource.parse::<Resource>(),
        purpose.parse::<Purpose>(),
    ) {
        (Ok(n), Ok(r), Ok(p)) => (n, r, p),
        _ => return bad_request("bad node_id, resource or purpose".to_string()),
    };
    match delete_node_formula(node, resource, purpose).await {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(e) => repo_error_response(&e),
    }
}
```

Reuse the file's existing `bad_request` / `repo_error_response` helpers (add `validation_error_response` mapping to `{"error":{"code":"Validation","message":msg}}` if it isn't already there), and add `formula_to_json` to `crates/services/hierarchy/src/json.rs` emitting `{node, resource, purpose, terms:[{ref, coefficient}], note}`.

Add the two `run` arms:

```rust
        Command::SetNodeFormula { node_id, resource, purpose, terms, note } => {
            handle_set_node_formula(
                node_id, resource, purpose, terms, note,
                get_node_fn(ddb, table.clone()),
                list_company_formulas_fn(ddb, table.clone()),
                list_company_sensors_fn(ddb, table.clone()),
                list_company_nodes_fn(ddb, table.clone()),
                put_node_formula_fn(ddb, table.clone()),
            )
            .await
        }

        Command::DeleteNodeFormula { node_id, resource, purpose } => {
            handle_delete_node_formula(
                node_id, resource, purpose,
                delete_node_formula_fn(ddb, table.clone()),
            )
            .await
        }
```

Add the four closure factories to `crates/services/hierarchy/src/repo_fns.rs`, following the existing pattern exactly:

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

pub fn delete_node_formula_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(NodeId, Resource, Purpose) -> RepoFut<()> + Clone {
    move |node, resource, purpose| {
        let t = table.clone();
        Box::pin(async move {
            ddb_formula::delete_node_formula(ddb, &t, &node, resource, purpose).await
        })
    }
}

pub fn list_company_formulas_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(String) -> RepoFut<Vec<NodeFormula>> + Clone {
    move |company_path| {
        let t = table.clone();
        Box::pin(async move { ddb_formula::list_company_formulas(ddb, &t, &company_path).await })
    }
}

pub fn list_node_formulas_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(NodeId) -> RepoFut<Vec<NodeFormula>> + Clone {
    move |node| {
        let t = table.clone();
        Box::pin(async move { ddb_formula::list_node_formulas(ddb, &t, &node).await })
    }
}
```

`list_company_sensors_fn` and `list_company_nodes_fn` wrap the existing
`ddb_sensor::list_sensors_under_path` and `ddb_node::list_by_gsi1_prefix`
(fanned out over `HN2`..`HN9`) the same way.

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p hierarchy 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 9: Confirm the write gate**

Formula edits must sit behind the `writes` edge, the same gate as `attach_sensor`. Check how `run` gates `Command::AttachSensor` (`crates/services/hierarchy/src/dispatch.rs:821` onward) and apply the identical guard to both new arms.

Run: `cargo test -p hierarchy 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/services/hierarchy/src/
git commit -m "feat(hierarchy): set_node_formula / delete_node_formula commands"
```

---

### Task 7: Formler tab; remove the sensor-formula UI

**Files:**
- Modify: `crates/services/hierarchy/src/query.rs` (add a `node_formulas` action next to `"sensors"` at line ~930)
- Modify: `crates/services/hierarchy/src/html/node.rs` (delete lines ~308-500, the formula dialog; add the Formler tab renderer)
- Modify: `crates/services/hierarchy/src/html/forms.rs` (drop the formula row from the add-sensor form; rename its `purpose` select to `resource`)
- Test: `crates/services/hierarchy/tests/node_forms_html.rs`

**Interfaces:**
- Consumes: `list_node_formulas_fn`, `list_company_sensors_fn` (Task 6)
- Produces: `GET /hierarchy/query/node_formulas?node=<NodeId>` → HTML fragment; `html::node::render_node_formulas(&[NodeFormula], &[Sensor], &[Node]) -> Markup`

- [ ] **Step 1: Write the failing HTML tests**

Add to `crates/services/hierarchy/tests/node_forms_html.rs`:

```rust
/// The Formler tab renders one card per formula, headed by (resource, purpose).
#[test]
fn node_formulas_render_one_card_per_formula() {
    let html = render_node_formulas_fixture(); // helper below
    assert!(html.contains("district_heating"), "resource in the heading");
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

/// The old per-sensor formula dialog is gone for good.
#[test]
fn add_sensor_form_has_no_formula_controls() {
    let html = render_add_sensor_form_fixture(); // existing helper in this file
    assert!(!html.contains("formula-dialog"));
    assert!(!html.contains("data.formula.kind"));
    assert!(html.contains("name=\"resource\""), "purpose select renamed to resource");
}
```

Add the `render_node_formulas_fixture` helper in the same file, building two `NodeFormula`s on `HN4#30` (`district_heating/dhw` and `district_heating/space_heating` with the −1 term and the note `"bimåler"`), a descendant sensor `S#1` and a non-descendant `S#77`, then calling `hierarchy::html::node::render_node_formulas`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p hierarchy --test node_forms_html 2>&1 | tail -20`
Expected: FAIL — `cannot find function render_node_formulas`.

- [ ] **Step 3: Delete the sensor-formula UI**

In `crates/services/hierarchy/src/html/node.rs`, delete the whole formula block (the `<dialog id="formula-dialog">`, the kind select, the expression input, the alias→sensor ref rows, the three hidden `data.formula.*` inputs, the "Edit formula…" button, the summary span, and their inline `<script>`). In `crates/services/hierarchy/src/html/forms.rs`, remove the Formula row from the add-sensor form and rename the `purpose` select to `resource`. Remove the company-sensor `<option>` fragment endpoint if nothing else uses it.

Run: `cargo test -p hierarchy add_sensor_form_has_no_formula_controls 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 4: Implement the Formler tab**

Add to `crates/services/hierarchy/src/html/node.rs`:

```rust
/// The node panel's **Formler** tab: one card per declared formula, plus an
/// empty card for adding one. `descendants` is what the reference picker offers
/// — the subtree rule made visible.
pub fn render_node_formulas(
    node: &Node,
    formulas: &[NodeFormula],
    descendant_sensors: &[Sensor],
    descendant_nodes: &[Node],
) -> Markup {
    html! {
        div class="formulas" {
            @for f in formulas {
                section class="formula-card" data-resource=(f.resource) data-purpose=(f.purpose) {
                    header {
                        span class="res" { (f.resource) }
                        span class="pur" { (f.purpose) }
                        @if let Some(n) = &f.note { span class="note" { (n) } }
                    }
                    form hx-post="/hierarchy/command" hx-swap="none" {
                        input type="hidden" name="action" value="set_node_formula";
                        input type="hidden" name="node_id" value=(node.id);
                        input type="hidden" name="resource" value=(f.resource);
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
                        input type="hidden" name="resource" value=(f.resource);
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
                    (s.daq_id) " (" (s.resource) ")"
                }
            }
        }
    }
}
```

Add `terms_json` (serialising `&[Term]` to the `[{"ref":…,"coefficient":…}]` wire form) and `new_formula_card` (the same card with an empty term list plus `resource`/`purpose` selects populated from `Resource::all()` and `Purpose::all().filter(|p| p.declarable())`). Add a small inline script that keeps the hidden `terms` input in sync with the rows on submit — the same technique the deleted formula dialog used for `data.formula.refs`.

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

`handle_node_formulas` loads the node, filters the company's sensors and nodes down to strict descendants of `node.path`, and renders `render_node_formulas`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p hierarchy 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Add the tab to the node panel**

In the node panel's tab strip in `crates/services/hierarchy/src/html/node.rs`, add a **Formler** tab whose content loads via `hx-get="/hierarchy/query/node_formulas?node=<id>"`, matching how the existing Data/sensor tabs load.

Run: `cargo test -p hierarchy 2>&1 | tail -5 && cargo clippy --all-targets 2>&1 | grep -c warning`
Expected: tests PASS, `0` warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/services/hierarchy/
git commit -m "feat(hierarchy): Formler tab for node formulas; drop the sensor formula dialog"
```

---

### Task 8: Deploy Phase 1

**Files:**
- Modify: none (deploy only)

- [ ] **Step 1: Build the lambda**

```bash
cargo lambda build --release --arm64 -p hierarchy
```

- [ ] **Step 2: Diff the stack**

```bash
cd infra/hierarchy && unset GOROOT && \
export AWS_PROFILE=stel-sb && \
eval "$(aws configure export-credentials --profile stel-sb --format env)" && \
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1 && \
cdk diff OcamlHierarchyStack
```

Expected: only Lambda `Code` `[~]` updates. **Stop and report** if the DynamoDB table shows any change.

- [ ] **Step 3: Deploy**

```bash
cd infra/hierarchy && unset GOROOT && \
export AWS_PROFILE=stel-sb && \
eval "$(aws configure export-credentials --profile stel-sb --format env)" && \
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1 && \
cdk deploy OcamlHierarchyStack --require-approval never
```

- [ ] **Step 4: Verify live**

```bash
aws lambda get-function-configuration --profile stel-sb \
  --function-name rust-lambda-hierarchy \
  --query '{State:State,Last:LastUpdateStatus}'
curl -s "https://d24beiqs2cj89y.cloudfront.net/hierarchy/query/node_formulas?node=HN2%23997" | head -20
```

Expected: `State=Active`, `LastUpdateStatus=Successful`, and an HTML fragment (empty formula list is fine).

- [ ] **Step 5: Commit**

```bash
git commit --allow-empty -m "chore(hierarchy): deploy node-formula authoring (phase 1)"
```

---

# Phase 2 — Pipeline

Renames the `purpose` column to `resource` end-to-end and teaches the roll-up job to evaluate formulas. **Tasks 9–12 must land together** — there is no fallback in the contract.

---

### Task 9: Bridge rename + cross-account reader role

**Files:**
- Modify: `infra/hierarchy/app.go:285-360` (the inlined Python bridge `_item` builder)
- Modify: `infra/hierarchy/app.go` (add the `HierarchyReaderRole` construct)

**Interfaces:**
- Produces: `meter-identity` items with `resource` instead of `purpose` and **no** `formula`; an IAM role ARN `arn:aws:iam::339712745226:role/HierarchyReaderRole` consumed by Task 11.

- [ ] **Step 1: Edit the bridge item builder**

In the inlined Python in `infra/hierarchy/app.go`, change `_item` so it emits `"resource": {"S": img["resource"]["S"]}` instead of `"purpose"`, and **delete** the two lines that copy `formula`:

```python
        if "formula" in img:
            out["formula"] = {"S": json.dumps(_d.deserialize(img["formula"]), default=str)}
```

- [ ] **Step 2: Add the reader role**

Add to `infra/hierarchy/app.go`, alongside the existing bridge constructs:

```go
	// The DAQ account's Glue roll-up job assumes this to read the node/formula
	// graph directly (spec §6). Read-only, scoped to hierarchy_new + its GSI.
	readerRole := awsiam.NewRole(stack, jsii.String("HierarchyReaderRole"), &awsiam.RoleProps{
		RoleName: jsii.String("HierarchyReaderRole"),
		AssumedBy: awsiam.NewArnPrincipal(
			jsii.String("arn:aws:iam::891377204778:role/MeasurementsAggregateGlueRole")),
	})
	readerRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Actions: jsii.Strings("dynamodb:Query", "dynamodb:GetItem"),
		Resources: jsii.Strings(
			*hierarchyTable.TableArn(),
			*hierarchyTable.TableArn()+"/index/gsi1",
		),
	}))
```

Use the actual Glue job role name from `infra/daq/data_pipeline/measurements_aggregate_stack.go` — check it with `grep -n 'NewRole\|RoleName' infra/daq/data_pipeline/measurements_aggregate_stack.go` and use that exact ARN.

- [ ] **Step 3: Diff and deploy**

```bash
cd infra/hierarchy && unset GOROOT && \
export AWS_PROFILE=stel-sb && \
eval "$(aws configure export-credentials --profile stel-sb --format env)" && \
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1 && \
cdk diff OcamlHierarchyStack
```

Expected: a new IAM role, plus the bridge Lambda's inline code `[~]`. **Stop and report** if the DynamoDB table shows any change. Then deploy with `cdk deploy OcamlHierarchyStack --require-approval never`.

- [ ] **Step 4: Verify the bridge writes the new shape**

Attach or re-save a sensor through the UI, then:

```bash
aws dynamodb scan --profile daq_dev --table-name meter-identity --limit 3 \
  --query 'Items[].{daq:sk.S,resource:resource.S,purpose:purpose.S,formula:formula.S}'
```

Expected: `resource` populated, `purpose` and `formula` absent on freshly written rows.

- [ ] **Step 5: Commit**

```bash
git add infra/hierarchy/app.go
git commit -m "feat(bridge): purpose -> resource, drop formula; add HierarchyReaderRole"
```

---

### Task 10: Flink + Iceberg column rename

**Files:**
- Modify: `infra/daq/data_pipeline/flink_app_scala/src/main/scala/com/enity/flink/enrichment/MeterMapping.scala:25,72`
- Modify: `.../enrichment/DdbBootstrapLoader.scala:27,39`, `.../enrichment/DdbStreamDeserializer.scala:35,45`, `.../enrichment/MeterEnrichmentFunction.scala:109`
- Modify: `.../flink/Main.scala:304,336,345`
- Modify: `infra/daq/data_pipeline/s3tables_stack.go:73`
- Modify: the four Scala spec files under `src/test/scala/` that reference `purpose`

- [ ] **Step 1: Rename in Scala and its tests**

Rename the `purpose` field to `resource` in `MeterMapping`, both deserialisers, the enrichment function, and `Main.scala`'s table schema and column list. Update the four spec files.

Run: `cd infra/daq/data_pipeline/flink_app_scala && sbt test 2>&1 | tail -20`
Expected: all specs PASS.

- [ ] **Step 2: Rename the Iceberg column**

In `infra/daq/data_pipeline/s3tables_stack.go:73`, change `field("purpose", "string", false)` to `field("resource", "string", false)` — in **both** the `raw_data` and `logical_meter_data` table definitions.

- [ ] **Step 3: Two-step delete/recreate (destructive — confirm first)**

`AWS::S3Tables::Table` cannot be replaced in place: create-before-delete fails with `409 "table with an identical name already exists"`. **This clears both tables' data.** Kinesis retention is 24 h, so ~1 day is replayable.

First stop the Flink app so it isn't writing to a table that's being dropped:

```bash
aws kinesisanalyticsv2 stop-application --profile daq_dev \
  --application-name flink-iceberg-processor --force
```

Then step (1) — comment out the `raw_data` and `logical_meter_data` resources in `s3tables_stack.go` and deploy so CFN deletes them:

```bash
cd infra/daq/data_pipeline && unset GOROOT && \
export AWS_PROFILE=daq_dev && \
eval "$(aws configure export-credentials --profile daq_dev --format env)" && \
npx cdk diff S3TablesStack -c TableBucketName=measurements && \
npx cdk deploy S3TablesStack --require-approval never -c TableBucketName=measurements
```

Step (2) — restore both resources with the `resource` column and deploy again (same commands). Confirm the diff creates exactly the two tables.

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
aws athena start-query-execution --profile daq_dev \
  --work-group daq-workgroup \
  --query-string "SELECT resource, count(*) FROM all.raw_data GROUP BY resource LIMIT 10"
```

Expected: rows grouped by populated `resource` values.

- [ ] **Step 6: Commit**

```bash
git add infra/daq/data_pipeline/
git commit -m "feat(pipeline)!: rename purpose -> resource in Flink and the Iceberg tables"
```

---

### Task 11: Glue weight-matrix loader

**Files:**
- Create: `infra/daq/data_pipeline/glue/hierarchy_matrix.py`
- Create: `infra/daq/data_pipeline/glue/tests/test_hierarchy_matrix.py`
- Create: `docs/fixtures/node-formula-parity.json`, `docs/fixtures/node-formula-parity-expected.json`

**Interfaces:**
- Consumes: `HierarchyReaderRole` (Task 9)
- Produces:
  - `hierarchy_matrix.total_weight(sensor, formulas, nodes) -> float`
  - `hierarchy_matrix.is_derived(formula, sensors) -> bool`
  - `hierarchy_matrix.flatten(graph) -> list[dict]` with keys `declaring_node, resource, purpose, sensor_id, coefficient, derived`
  - `hierarchy_matrix.total_weights(graph) -> dict[int, float]`
  - `hierarchy_matrix.load_company(ddb, company_id) -> graph`

The Python must implement **exactly** the rules in `crates/model/src/logic/formulas.rs` (Task 3). The golden fixture below is what keeps them aligned.

- [ ] **Step 1: Write the golden fixture**

Create `docs/fixtures/node-formula-parity.json` — the chiller + bimåler + generation + gas cases from the presentation:

```json
{
  "company_path": "HN0#root|HN1#1|HN2#997",
  "nodes": [
    {"id": "HN2#997", "path": "HN0#root|HN1#1|HN2#997"},
    {"id": "HN4#30",  "path": "HN0#root|HN1#1|HN2#997|HN4#30"},
    {"id": "HN5#5",   "path": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#5"},
    {"id": "HN5#7",   "path": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#7"},
    {"id": "HN4#8",   "path": "HN0#root|HN1#1|HN2#997|HN4#8"}
  ],
  "sensors": [
    {"id": 1,  "path": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#5|S#1",  "resource": "electricity"},
    {"id": 2,  "path": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#5|S#2",  "resource": "electricity"},
    {"id": 3,  "path": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#5|S#3",  "resource": "electricity"},
    {"id": 10, "path": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#7|S#10", "resource": "electricity"},
    {"id": 11, "path": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#7|S#11", "resource": "electricity"},
    {"id": 20, "path": "HN0#root|HN1#1|HN2#997|HN4#30|S#20", "resource": "district_heating"},
    {"id": 21, "path": "HN0#root|HN1#1|HN2#997|HN4#30|S#21", "resource": "district_heating"},
    {"id": 30, "path": "HN0#root|HN1#1|HN2#997|HN4#8|S#30",  "resource": "gas"}
  ],
  "formulas": [
    {"node": "HN5#5", "resource": "electricity", "purpose": "total",
     "terms": [{"ref": "S#2", "coefficient": 0}, {"ref": "S#3", "coefficient": 0}]},
    {"node": "HN5#5", "resource": "electricity", "purpose": "cooling",
     "terms": [{"ref": "S#1", "coefficient": 1}]},
    {"node": "HN5#5", "resource": "district_cooling", "purpose": "cooling",
     "terms": [{"ref": "S#1", "coefficient": 3.2}]},
    {"node": "HN5#7", "resource": "electricity", "purpose": "generation",
     "terms": [{"ref": "S#11", "coefficient": 1}]},
    {"node": "HN4#30", "resource": "district_heating", "purpose": "total",
     "terms": [{"ref": "S#21", "coefficient": 0}]},
    {"node": "HN4#30", "resource": "district_heating", "purpose": "dhw",
     "terms": [{"ref": "S#21", "coefficient": 1}]},
    {"node": "HN4#30", "resource": "district_heating", "purpose": "space_heating",
     "terms": [{"ref": "S#20", "coefficient": 1}, {"ref": "S#21", "coefficient": -1}]},
    {"node": "HN4#8", "resource": "heat", "purpose": "space_heating",
     "terms": [{"ref": "S#30", "coefficient": 10.45}]}
  ]
}
```

Create `docs/fixtures/node-formula-parity-expected.json`:

```json
{
  "total_weights": {"1": 1.0, "2": 0.0, "3": 0.0, "10": 1.0, "11": 0.0, "20": 1.0, "21": 0.0, "30": 1.0},
  "rows": [
    {"declaring_node": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#5", "resource": "electricity",
     "purpose": "cooling", "sensor_id": 1, "coefficient": 1.0, "derived": false},
    {"declaring_node": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#5", "resource": "district_cooling",
     "purpose": "cooling", "sensor_id": 1, "coefficient": 3.2, "derived": true},
    {"declaring_node": "HN0#root|HN1#1|HN2#997|HN4#30|HN5#7", "resource": "electricity",
     "purpose": "generation", "sensor_id": 11, "coefficient": 1.0, "derived": false},
    {"declaring_node": "HN0#root|HN1#1|HN2#997|HN4#30", "resource": "district_heating",
     "purpose": "dhw", "sensor_id": 21, "coefficient": 1.0, "derived": false},
    {"declaring_node": "HN0#root|HN1#1|HN2#997|HN4#30", "resource": "district_heating",
     "purpose": "space_heating", "sensor_id": 20, "coefficient": 1.0, "derived": false},
    {"declaring_node": "HN0#root|HN1#1|HN2#997|HN4#30", "resource": "district_heating",
     "purpose": "space_heating", "sensor_id": 21, "coefficient": -1.0, "derived": false},
    {"declaring_node": "HN0#root|HN1#1|HN2#997|HN4#8", "resource": "heat",
     "purpose": "space_heating", "sensor_id": 30, "coefficient": 10.45, "derived": true}
  ]
}
```

- [ ] **Step 2: Write the failing PySpark-side test**

Create `infra/daq/data_pipeline/glue/tests/test_hierarchy_matrix.py`:

```python
import json, os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import hierarchy_matrix as hm

FIX = os.path.join(os.path.dirname(__file__), "..", "..", "..", "..",
                   "docs", "fixtures")


def _load(name):
    with open(os.path.join(FIX, name)) as fh:
        return json.load(fh)


def _key(r):
    return (r["declaring_node"], r["resource"], r["purpose"], r["sensor_id"])


def test_total_weights_match_the_golden_fixture():
    graph, expected = _load("node-formula-parity.json"), _load("node-formula-parity-expected.json")
    got = hm.total_weights(graph)
    assert {str(k): v for k, v in got.items()} == expected["total_weights"]


def test_flatten_matches_the_golden_fixture():
    graph, expected = _load("node-formula-parity.json"), _load("node-formula-parity-expected.json")
    got = sorted(hm.flatten(graph), key=_key)
    want = sorted(expected["rows"], key=_key)
    assert got == want


def test_total_formulas_are_not_emitted_as_claims():
    graph = _load("node-formula-parity.json")
    assert all(r["purpose"] != "total" for r in hm.flatten(graph))
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd infra/daq/data_pipeline/glue && python -m pytest tests/test_hierarchy_matrix.py -q 2>&1 | tail -10`
Expected: FAIL — `ModuleNotFoundError: No module named 'hierarchy_matrix'`.

- [ ] **Step 4: Implement the matrix module**

Create `infra/daq/data_pipeline/glue/hierarchy_matrix.py`:

```python
"""Load and flatten a company's node formulas into a weight matrix.

Mirrors crates/model/src/logic/formulas.rs exactly. Both sides are pinned to
docs/fixtures/node-formula-parity{,-expected}.json — change one, change both.
"""

SEP = "|"
OUTFLOW_PURPOSES = {"generation"}


def _node_path(graph, node_id):
    for n in graph["nodes"]:
        if n["id"] == node_id:
            return n["path"]
    return None


def _sensor(graph, sensor_id):
    for s in graph["sensors"]:
        if s["id"] == sensor_id:
            return s
    return None


def _ref_sensor_id(ref):
    """'S#21' -> 21; node references return None."""
    return int(ref[2:]) if ref.startswith("S#") else None


def _at_or_under(descendant, ancestor):
    return descendant == ancestor or descendant.startswith(ancestor + SEP)


def _mentions(formula, sensor_id):
    return any(_ref_sensor_id(t["ref"]) == sensor_id for t in formula["terms"])


def _governs(graph, formula, sensor):
    p = _node_path(graph, formula["node"])
    return p is not None and _at_or_under(sensor["path"], p)


def total_weight(graph, sensor):
    """0 if claimed as an outflow; else the DEEPEST total override mentioning it;
    else the default 1."""
    for f in graph["formulas"]:
        if (f["purpose"] in OUTFLOW_PURPOSES
                and _governs(graph, f, sensor) and _mentions(f, sensor["id"])):
            return 0.0
    candidates = [
        f for f in graph["formulas"]
        if f["purpose"] == "total" and f["resource"] == sensor["resource"]
        and _governs(graph, f, sensor) and _mentions(f, sensor["id"])
    ]
    if not candidates:
        return 1.0
    deepest = max(candidates, key=lambda f: len(_node_path(graph, f["node"]) or ""))
    for t in deepest["terms"]:
        if _ref_sensor_id(t["ref"]) == sensor["id"]:
            return float(t["coefficient"])
    return 1.0


def total_weights(graph):
    """sensor_id -> weight in the `total` series."""
    return {s["id"]: total_weight(graph, s) for s in graph["sensors"]}


def is_derived(graph, formula):
    """True when the declared output resource differs from a referenced sensor's."""
    for t in formula["terms"]:
        sid = _ref_sensor_id(t["ref"])
        if sid is None:
            continue
        s = _sensor(graph, sid)
        if s is not None and s["resource"] != formula["resource"]:
            return True
    return False


def _expand(graph, formula, term):
    sid = _ref_sensor_id(term["ref"])
    if sid is not None:
        return [(sid, float(term["coefficient"]))]
    path = _node_path(graph, term["ref"])
    if path is None:
        return []
    out = []
    for s in graph["sensors"]:
        if s["resource"] != formula["resource"]:
            continue
        if not s["path"].startswith(path + SEP):
            continue
        c = float(term["coefficient"]) * total_weight(graph, s)
        if c != 0.0:
            out.append((s["id"], c))
    return out


def flatten(graph):
    """Weight rows for every declared claim. `total` formulas are weight
    overrides consumed by total_weight(), not claims, so they are not emitted."""
    rows = []
    for f in graph["formulas"]:
        if f["purpose"] == "total":
            continue
        declaring = _node_path(graph, f["node"])
        if declaring is None:
            continue
        derived = is_derived(graph, f)
        for t in f["terms"]:
            for sensor_id, coefficient in _expand(graph, f, t):
                rows.append({
                    "declaring_node": declaring,
                    "resource": f["resource"],
                    "purpose": f["purpose"],
                    "sensor_id": sensor_id,
                    "coefficient": coefficient,
                    "derived": derived,
                })
    return rows


# ── cross-account load ──

def reader_resource(role_arn, region):
    """DynamoDB resource in the hierarchy account, via HierarchyReaderRole."""
    import boto3
    c = boto3.client("sts").assume_role(
        RoleArn=role_arn, RoleSessionName="measurements-aggregate")["Credentials"]
    return boto3.resource(
        "dynamodb", region_name=region,
        aws_access_key_id=c["AccessKeyId"],
        aws_secret_access_key=c["SecretAccessKey"],
        aws_session_token=c["SessionToken"])


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


def load_company(table, company_id):
    """Build the graph for HN2#<company_id> in two GSI queries."""
    formula_items = _query_gsi(table, "F#HN2#%d" % company_id)
    sensor_items = _query_gsi(table, "S#HN2#%d" % company_id)

    sensors, nodes = [], {}
    for it in sensor_items:
        path = it["gsi1sk"]
        sensors.append({"id": int(it["pk"].split("#")[1]),
                        "path": path,
                        "resource": it.get("resource", "")})
        # Every node on a sensor's path is a node we may need to resolve.
        segs = path.split(SEP)
        for i in range(1, len(segs)):
            if segs[i - 1].startswith("HN"):
                nodes[segs[i - 1]] = SEP.join(segs[:i])

    formulas = []
    for it in formula_items:
        node_path = it["gsi1sk"].rsplit("#", 2)[0]
        nodes[it["pk"]] = node_path
        formulas.append({
            "node": it["pk"],
            "resource": it["resource"],
            "purpose": it["purpose"],
            "terms": [{"ref": t["ref"], "coefficient": float(t["coefficient"])}
                      for t in it.get("terms", [])],
        })

    return {"nodes": [{"id": k, "path": v} for k, v in nodes.items()],
            "sensors": sensors,
            "formulas": formulas}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd infra/daq/data_pipeline/glue && python -m pytest tests/test_hierarchy_matrix.py -q 2>&1 | tail -10`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add infra/daq/data_pipeline/glue/hierarchy_matrix.py \
        infra/daq/data_pipeline/glue/tests/test_hierarchy_matrix.py \
        docs/fixtures/
git commit -m "feat(glue): weight-matrix loader mirroring model::logic::formulas"
```

---

### Task 12: Rust-side parity assertion

**Files:**
- Create: `crates/model/tests/formula_parity.rs`

**Interfaces:**
- Consumes: `flatten`, `total_weight`, `CompanyGraph` (Task 3); the fixtures from Task 11

This is the drift control for spec §8.4: both implementations assert against the same expected file, so either one diverging fails its own suite.

- [ ] **Step 1: Write the failing test**

Create `crates/model/tests/formula_parity.rs`:

```rust
//! The Glue roll-up re-implements `logic::formulas` in PySpark (spec §6, D6).
//! Both sides assert against the same golden fixture, so either drifting fails.

use std::collections::BTreeMap;

use model::domain::ids::{NodeId, SensorId};
use model::domain::node::Node;
use model::domain::node_formula::{NodeFormula, Reference, Term};
use model::domain::sensor::Sensor;
use model::domain::values::{MeterType, Purpose, Resource};
use model::logic::formulas::{flatten, total_weight, CompanyGraph};

fn fixture(name: &str) -> serde_json::Value {
    let p = format!("{}/../../docs/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&p).expect(&p)).unwrap()
}

fn graph() -> CompanyGraph {
    let g = fixture("node-formula-parity.json");
    CompanyGraph {
        nodes: g["nodes"].as_array().unwrap().iter().map(|n| {
            Node::builder()
                .id(NodeId::parse(n["id"].as_str().unwrap()).unwrap())
                .name(String::new())
                .path(n["path"].as_str().unwrap().to_string())
                .build()
        }).collect(),
        sensors: g["sensors"].as_array().unwrap().iter().map(|s| {
            Sensor::builder()
                .id(SensorId::make(s["id"].as_u64().unwrap() as u32))
                .daq_id(String::new())
                .path(s["path"].as_str().unwrap().to_string())
                .resource(s["resource"].as_str().unwrap().parse::<Resource>().unwrap())
                .meter_type(MeterType::Counter)
                .build()
        }).collect(),
        formulas: g["formulas"].as_array().unwrap().iter().map(|f| NodeFormula {
            node: NodeId::parse(f["node"].as_str().unwrap()).unwrap(),
            resource: f["resource"].as_str().unwrap().parse::<Resource>().unwrap(),
            purpose: f["purpose"].as_str().unwrap().parse::<Purpose>().unwrap(),
            terms: f["terms"].as_array().unwrap().iter().map(|t| Term {
                reference: Reference::parse(t["ref"].as_str().unwrap()).unwrap(),
                coefficient: t["coefficient"].as_f64().unwrap(),
            }).collect(),
            note: None,
        }).collect(),
    }
}

#[test]
fn total_weights_match_the_golden_fixture() {
    let g = graph();
    let expected = fixture("node-formula-parity-expected.json");
    let want: BTreeMap<String, f64> = expected["total_weights"]
        .as_object().unwrap().iter()
        .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
        .collect();
    let got: BTreeMap<String, f64> = g.sensors.iter()
        .map(|s| (s.id.id().to_string(), total_weight(&g, s)))
        .collect();
    assert_eq!(got, want);
}

#[test]
fn flatten_matches_the_golden_fixture() {
    let g = graph();
    let expected = fixture("node-formula-parity-expected.json");

    let key = |node: &str, res: &str, pur: &str, sid: u32| format!("{node}|{res}|{pur}|{sid}");
    let want: BTreeMap<String, (f64, bool)> = expected["rows"].as_array().unwrap().iter()
        .map(|r| (
            key(r["declaring_node"].as_str().unwrap(),
                r["resource"].as_str().unwrap(),
                r["purpose"].as_str().unwrap(),
                r["sensor_id"].as_u64().unwrap() as u32),
            (r["coefficient"].as_f64().unwrap(), r["derived"].as_bool().unwrap()),
        ))
        .collect();
    let got: BTreeMap<String, (f64, bool)> = flatten(&g).into_iter()
        .map(|r| (
            key(&r.declaring_node, r.resource.as_str(), r.purpose.as_str(), r.sensor.id()),
            (r.coefficient, r.derived),
        ))
        .collect();
    assert_eq!(got, want);
}
```

- [ ] **Step 2: Run to verify it fails or passes meaningfully**

Run: `cargo test -p model --test formula_parity 2>&1 | tail -20`
Expected: PASS if Task 3 implemented the rules correctly. If it FAILS, the Rust side is wrong (the PySpark side already passes the same fixture) — fix `logic/formulas.rs`, not the fixture.

- [ ] **Step 3: Commit**

```bash
git add crates/model/tests/formula_parity.rs
git commit -m "test(model): pin flatten to the Rust/PySpark golden parity fixture"
```

---

### Task 13: Weighted roll-up

**Files:**
- Modify: `infra/daq/data_pipeline/glue/measurements_aggregate.py` (`build_sk`, `build_gsi1sk`, `build_rollups`, `read_counters`, `main`)
- Modify: `infra/daq/data_pipeline/glue/tests/test_rollups.py`

**Interfaces:**
- Consumes: `hierarchy_matrix.flatten`, `hierarchy_matrix.total_weights` (Task 11)
- Produces:
  - `build_sk(node_path, resource, purpose, gran, bucket) -> str`
  - `build_gsi1pk(hn2, dimension, purpose) -> str`
  - `build_rollups(df, matrix, run_at_iso) -> DataFrame` where `matrix` is `{"rows": [...], "total_weights": {...}}` keyed per company

- [ ] **Step 1: Write the failing tests**

Rewrite `infra/daq/data_pipeline/glue/tests/test_rollups.py`'s expectations and add these cases (keep the existing idempotency test, updating its sort keys):

```python
def _matrix():
    """Sensor 10009 claimed as lighting; 10010 nested inside 10009 (weight 0)."""
    return {
        "total_weights": {10009: 1.0, 10010: 0.0},
        "rows": [
            {"declaring_node": "HN2#2|HN3#9|HN4#456", "resource": "electricity",
             "purpose": "lighting", "sensor_id": 10009, "coefficient": 1.0, "derived": False},
        ],
    }


def test_sort_key_carries_the_purpose_segment(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    assert "HN2#2#electricity#total#h#2026-06-07T08" in out
    assert "HN2#2|HN3#9|HN4#456#electricity#lighting#h#2026-06-07T08" in out


def test_total_honours_weight_overrides(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    # 10009 contributes 4+6 = 10; 10010's 5 is weight 0 (nested meter).
    assert out["HN2#2#electricity#total#h#2026-06-07T08"]["sum"] == 10.0


def test_claims_roll_up_to_every_ancestor(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    for path in ["HN2#2", "HN2#2|HN3#9", "HN2#2|HN3#9|HN4#456"]:
        assert out["%s#electricity#lighting#h#2026-06-07T08" % path]["sum"] == 10.0


def test_unallocated_is_total_minus_claims(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2#electricity#unallocated#h#2026-06-07T08"]["sum"] == 0.0


def test_derived_rows_are_excluded_from_unallocated(spark):
    matrix = _matrix()
    matrix["rows"].append({
        "declaring_node": "HN2#2|HN3#9|HN4#456", "resource": "district_cooling",
        "purpose": "cooling", "sensor_id": 10009, "coefficient": 3.2, "derived": True})
    out = _by_sk(m.build_rollups(_input(spark), matrix, run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2#district_cooling#cooling#h#2026-06-07T08"]["sum"] == 32.0
    # No physical district_cooling meters, so its total and unallocated are both 0.
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

Also update `_input` so its column is named `resource` (not `purpose`) and its values are the lower-case token `"electricity"`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd infra/daq/data_pipeline/glue && python -m pytest tests/test_rollups.py -q 2>&1 | tail -15`
Expected: FAIL — `build_rollups() takes 2 positional arguments but 3 were given`.

- [ ] **Step 3: Rewrite the key builders**

In `infra/daq/data_pipeline/glue/measurements_aggregate.py`:

```python
def build_sk(node_path: str, resource: str, purpose: str, gran: str, bucket: str) -> str:
    """sk = '<node_path>#<resource>#<purpose>#<gran>#<bucket>'. The bucket stays
    LAST so a fixed (resource, purpose) is a pure BETWEEN key-range. The '#'
    after node_path keeps a node's own rows sorting before its descendants'
    ('|' > '#')."""
    return "%s#%s#%s#%s#%s" % (node_path, resource, purpose, gran, bucket)


def build_gsi1pk(hn2: int, dimension: str, purpose: str) -> str:
    """The dimension partition is per-purpose, so a cross-resource 'all energy'
    query can never sum `total` together with its own purpose breakdown."""
    return "HN2#%d#%s#%s" % (hn2, dimension, purpose)
```

`build_gsi1sk` is unchanged.

- [ ] **Step 4: Rewrite `build_rollups`**

Replace `build_rollups` in `infra/daq/data_pipeline/glue/measurements_aggregate.py`:

```python
def build_rollups(df: DataFrame, matrix: dict, run_at_iso: str) -> DataFrame:
    """Aggregate counter rows into per-node/resource/purpose/gran/bucket items.

    Three series come out:
      total        Σ of every descendant sensor of that resource, each at its
                   total weight (nearest declared override, 0 if claimed as an
                   outflow, else 1).
      <purpose>    Σ of the declared claims, rolled up to every ancestor of the
                   declaring node.
      unallocated  total − Σ(claims), excluding derived and outflow purposes.

    `matrix` is {"total_weights": {sensor_id: float}, "rows": [WeightRow…]} as
    produced by hierarchy_matrix.flatten/total_weights.
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

    # ── total ──
    weights = spark.createDataFrame(
        [(int(k), float(v)) for k, v in matrix["total_weights"].items()],
        T.StructType([T.StructField("w_logical_id", T.IntegerType()),
                      T.StructField("weight", T.DoubleType())]))
    weighted = (with_buckets
        .join(F.broadcast(weights),
              with_buckets.logical_id == weights.w_logical_id, "left")
        .withColumn("weight", F.coalesce(F.col("weight"), F.lit(1.0)))
        .withColumn("node_path", F.explode(_ancestor_keys_udf(
            *[F.col("hn%d" % i) for i in range(2, 10)], F.col("logical_id"))))
        .withColumn("contrib", F.col("resample_value") * F.col("weight")))

    totals = (weighted
        .groupBy("hn2", "node_path", "resource", "gran", "bucket")
        .agg(F.sum("contrib").alias("sum"),
             F.count("contrib").alias("count"),
             F.max("unit").alias("unit"))
        .withColumn("purpose", F.lit("total"))
        .withColumn("derived", F.lit(False)))

    # ── declared purposes ──
    rows = matrix["rows"]
    if rows:
        claims = spark.createDataFrame(
            [(r["declaring_node"], r["resource"], r["purpose"],
              int(r["sensor_id"]), float(r["coefficient"]), bool(r["derived"]))
             for r in rows],
            T.StructType([
                T.StructField("declaring_node", T.StringType()),
                T.StructField("c_resource", T.StringType()),
                T.StructField("purpose", T.StringType()),
                T.StructField("c_logical_id", T.IntegerType()),
                T.StructField("coefficient", T.DoubleType()),
                T.StructField("derived", T.BooleanType()),
            ]))
        claimed = (with_buckets
            .join(F.broadcast(claims),
                  with_buckets.logical_id == claims.c_logical_id, "inner")
            # A claim declared on a node counts for that node AND every ancestor.
            .withColumn("node_path", F.explode(_path_ancestors_udf(F.col("declaring_node"))))
            .withColumn("contrib", F.col("resample_value") * F.col("coefficient"))
            .groupBy("hn2", "node_path", "c_resource", "purpose", "gran", "bucket", "derived")
            .agg(F.sum("contrib").alias("sum"),
                 F.count("contrib").alias("count"),
                 F.max("unit").alias("unit"))
            .withColumnRenamed("c_resource", "resource"))
    else:
        claimed = totals.limit(0)

    # ── unallocated = total − Σ(physical, non-outflow claims) ──
    physical = (claimed
        .filter((~F.col("derived")) & (F.col("purpose") != F.lit("generation")))
        .groupBy("hn2", "node_path", "resource", "gran", "bucket")
        .agg(F.sum("sum").alias("claimed_sum")))
    unallocated = (totals
        .join(physical, ["hn2", "node_path", "resource", "gran", "bucket"], "left")
        .withColumn("sum", F.col("sum") - F.coalesce(F.col("claimed_sum"), F.lit(0.0)))
        .drop("claimed_sum")
        .withColumn("purpose", F.lit("unallocated")))

    grouped = totals.unionByName(claimed).unionByName(unallocated)

    return grouped.select(
        F.concat(F.lit("HN2#"), F.col("hn2").cast("string")).alias("pk"),
        _SK_UDF("node_path", "resource", "purpose", "gran", "bucket").alias("sk"),
        _GSI1PK_UDF("hn2", _DIM_UDF("unit"), "purpose").alias("gsi1pk"),
        _GSI1SK_UDF("node_path", "gran", "bucket").alias("gsi1sk"),
        "resource", "purpose", "unit", "sum", "count",
        F.lit(run_at_iso).alias("updated_at"),
        _TTL_UDF("gran", "bucket").alias("ttl"),
    )
```

Add the two new UDFs next to the existing ones:

```python
def path_ancestors(node_path: str):
    """'A|B|C' -> ['A', 'A|B', 'A|B|C'] — the claim-propagation chain."""
    segs = node_path.split("|")
    return ["|".join(segs[: i + 1]) for i in range(len(segs))]


_PATH_ANCESTORS_UDF = F.udf(path_ancestors, T.ArrayType(T.StringType()))
_GSI1PK_UDF = F.udf(build_gsi1pk, T.StringType())
```

(`_SK_UDF` now takes five columns; update its definition to `F.udf(build_sk, T.StringType())` — unchanged — and its call sites.) Rename the `purpose` column to `resource` in `read_counters`'s `SELECT` and its projection.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd infra/daq/data_pipeline/glue && python -m pytest tests/ -q 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 6: Wire the matrix into `main`**

In `main`, after `read_counters`, load the matrix per company and merge:

```python
    reader_role = args.get("hierarchy_reader_role_arn")
    counters = read_counters(spark, window_start)
    companies = [r["hn2"] for r in counters.select("hn2").distinct().collect()]
    ddb = hierarchy_matrix.reader_resource(reader_role, region)
    table = ddb.Table("hierarchy_new")
    matrix = {"total_weights": {}, "rows": []}
    for cid in companies:
        graph = hierarchy_matrix.load_company(table, int(cid))
        matrix["total_weights"].update(hierarchy_matrix.total_weights(graph))
        matrix["rows"].extend(hierarchy_matrix.flatten(graph))
    rollups = build_rollups(counters, matrix, now.strftime("%Y-%m-%dT%H:%M:%S+00:00"))
```

Add `hierarchy_reader_role_arn` to the `required` args list, and pass it plus `hierarchy_matrix.py` (as an extra `--extra-py-files` asset) from `infra/daq/data_pipeline/measurements_aggregate_stack.go`. Grant the Glue role `sts:AssumeRole` on `arn:aws:iam::339712745226:role/HierarchyReaderRole`.

- [ ] **Step 7: Diff and deploy**

```bash
cd infra/daq/data_pipeline && unset GOROOT && \
export AWS_PROFILE=daq_dev && \
eval "$(aws configure export-credentials --profile daq_dev --format env)" && \
npx cdk diff MeasurementsAggregateStack \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
```

Expected: Glue script asset `[~]`, IAM policy `[~]`, new job argument. **Stop and report** if the `measurements_aggregate` table shows replacement. Then `npx cdk deploy MeasurementsAggregateStack --require-approval never` with the same `-c` flags.

- [ ] **Step 8: Wipe and rebuild the view**

The sort key shape changed, so old rows are unreadable garbage. Delete them, then run the job over a wide window:

```bash
aws dynamodb scan --profile daq_dev --table-name measurements_aggregate \
  --projection-expression 'pk,sk' --query 'Items[*]' --output json \
  > /tmp/claude-1000/-home-sla-projects-ems-rust/dc6f41e2-8599-4c7a-8bce-3e403680bd17/scratchpad/old-rollups.json
# delete in batches of 25 with batch-write-item, then:
aws glue start-job-run --profile daq_dev --job-name measurements-aggregate \
  --arguments '{"--lookback_days":"30"}'
aws glue get-job-runs --profile daq_dev --job-name measurements-aggregate \
  --max-results 1 --query 'JobRuns[0].{State:JobRunState,Error:ErrorMessage}'
```

Expected: `State=SUCCEEDED`.

- [ ] **Step 9: Commit**

```bash
git add infra/daq/data_pipeline/
git commit -m "feat(glue): weighted roll-up with total/purpose/unallocated series"
```

---

# Phase 3 — Read side + frontend

---

### Task 14: Aggregations reads the purpose axis

**Files:**
- Modify: `crates/services/aggregations/src/main.rs` — `parse_sk`, `Row` (line 145), `to_rows` (157), `to_rows_dimension` (187), `QueryParams` (236), `query_node` (253), `query_one_resource` (268), `query_dimension` (301), `handle_aggregations` (441), `fetch_node_rows` (550), `tariff_dkk_per_unit` (609), `emission_kg_per_unit` (623), `scale_rows` (654)

**Interfaces:**
- Consumes: the roll-up rows from Task 13
- Produces:
  - `Row { level_id, resource, purpose, unit, resolution, timestamp, value, contributor_count }`
  - `QueryParams { …, resource, purpose }`
  - `tariff_dkk_per_unit(resource: &str, purpose: &str, unit: &str) -> f64`
  - `emission_kg_per_unit(resource: &str, purpose: &str, unit: &str) -> f64`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/services/aggregations/src/main.rs`:

```rust
    #[test]
    fn parse_sk_reads_the_purpose_segment() {
        let (path, resource, purpose, gran, bucket) =
            parse_sk("HN2#2|HN3#9#electricity#lighting#h#2026-06-07T08");
        assert_eq!(path, "HN2#2|HN3#9");
        assert_eq!(resource, "electricity");
        assert_eq!(purpose, "lighting");
        assert_eq!(gran, "h");
        assert_eq!(bucket, "2026-06-07T08");
    }

    /// The default purpose is `total`, so existing widgets keep their meaning
    /// at their current cost — one pure BETWEEN range per resource.
    #[test]
    fn query_prefix_defaults_to_total() {
        assert_eq!(
            sk_prefix("HN2#2", "electricity", "", Gran::Hour),
            "HN2#2#electricity#total#h#"
        );
        assert_eq!(
            sk_prefix("HN2#2", "electricity", "lighting", Gran::Hour),
            "HN2#2#electricity#lighting#h#"
        );
    }

    /// Exported energy must not be billed or charged CO₂ — this is what
    /// replaces the demo's per-sensor emission factor.
    #[test]
    fn generation_has_no_tariff_or_emissions() {
        assert_eq!(emission_kg_per_unit("electricity", "generation", "kWh"), 0.0);
        assert_eq!(tariff_dkk_per_unit("electricity", "generation", "kWh"), 0.0);
    }

    /// Everything else falls back to the resource-level factor.
    #[test]
    fn other_purposes_fall_back_to_the_resource_factor() {
        assert_eq!(emission_kg_per_unit("electricity", "lighting", "kWh"), 0.12);
        assert_eq!(emission_kg_per_unit("electricity", "total", "kWh"), 0.12);
        assert_eq!(tariff_dkk_per_unit("water", "total", "m3"), 50.0);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aggregations parse_sk_reads 2>&1 | tail -20`
Expected: FAIL — `parse_sk` returns a 4-tuple.

- [ ] **Step 3: Implement the read-side changes**

1. `parse_sk` returns `(path, resource, purpose, gran, bucket)` — the sk now has five `#`-separated segments after the path.
2. Extract the prefix builder so it is testable:

```rust
/// `<node_path>#<resource>#<purpose>#<gran>#` — the bucket is appended by the
/// caller to form a pure `BETWEEN` key-range. An empty `purpose` means `total`.
fn sk_prefix(sk_path: &str, resource: &str, purpose: &str, gran: Gran) -> String {
    let purpose = if purpose.is_empty() { "total" } else { purpose };
    format!("{sk_path}#{resource}#{purpose}#{}#", gran.code())
}
```

   and use it in `query_one_resource`.
3. Add `purpose: &'a str` to `QueryParams`; thread it from `handle_aggregations` (`qs.get("purpose")`, default `""`) and from `fetch_node_rows` (always `"total"`).
4. `query_dimension`'s `gsi1pk` becomes `format!("{}#{}#{}", p.pk, dimension, if p.purpose.is_empty() { "total" } else { p.purpose })`.
5. `Row` gains `resource: String` and `purpose` now holds the real purpose; `to_rows`/`to_rows_dimension` populate both.
6. The factor tables take a purpose:

```rust
/// Representative unit price (DKK). Outflow purposes are not billed.
fn tariff_dkk_per_unit(resource: &str, purpose: &str, unit: &str) -> f64 {
    if purpose == "generation" {
        return 0.0;
    }
    let (per_kwh, per_m3) = match resource {
        "electricity" => (2.50, 0.0),
        "district_heating" | "heat" => (0.90, 0.0),
        "district_cooling" => (0.50, 0.0),
        "gas" => (0.0, 8.0),
        "water" => (0.0, 50.0),
        _ => (0.0, 0.0),
    };
    per_unit(per_kwh, per_m3, unit)
}

/// Representative CO₂e factor (kg CO₂e). Exported energy emits nothing here —
/// it is an outflow, which is why no per-sensor emission factor is needed.
fn emission_kg_per_unit(resource: &str, purpose: &str, unit: &str) -> f64 {
    if purpose == "generation" {
        return 0.0;
    }
    let (per_kwh, per_m3) = match resource {
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

7. `scale_rows`'s `factor` closure becomes `Fn(&str, &str, &str) -> f64`, called as `factor(&r.resource, &r.purpose, &r.unit)`.
8. `handle_alarms` and `handle_benchmark` pass `purpose = "total"` explicitly — a median-spike test and an area-normalised peer comparison are meaningless on a subtractive series.
9. Update the `#[utoipa::path]` params on `get_aggregations` with the new `purpose` query parameter.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p aggregations 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/services/aggregations/
git commit -m "feat(aggregations): read the purpose axis; (resource, purpose) tariff and CO2 factors"
```

---

### Task 15: `get_purpose_split` query action

**Files:**
- Modify: `crates/services/aggregations/src/main.rs` (route table at line 387-403; new handler)

**Interfaces:**
- Consumes: `fetch_node_rows`, `sk_prefix` (Task 14)
- Produces: `GET /meterdata/query/get_purpose_split?level_id=&resource=&start=&end=&resolution=&format=` returning one `Row` per purpose

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/services/aggregations/src/main.rs`:

```rust
    /// The split fans out over the declared purposes, keeps `total` separate,
    /// and never folds derived rows into the physical breakdown.
    #[test]
    fn purpose_split_rows_are_grouped_by_purpose() {
        let items = vec![
            agg_item("HN2#2#electricity#total#d#2026-06-07", 100.0),
            agg_item("HN2#2#electricity#lighting#d#2026-06-07", 22.0),
            agg_item("HN2#2#electricity#cooling#d#2026-06-07", 18.0),
            agg_item("HN2#2#electricity#unallocated#d#2026-06-07", 60.0),
        ];
        let rows = to_rows(items, "HN2#2", "daily", Gran::Day);
        let by_purpose: std::collections::BTreeMap<_, _> =
            rows.iter().map(|r| (r.purpose.as_str(), r.value)).collect();
        assert_eq!(by_purpose["total"], 100.0);
        assert_eq!(by_purpose["lighting"], 22.0);
        assert_eq!(by_purpose["unallocated"], 60.0);
        assert_eq!(
            by_purpose["lighting"] + by_purpose["cooling"] + by_purpose["unallocated"],
            by_purpose["total"]
        );
    }
```

Add the `agg_item(sk, sum)` test helper if the module doesn't already have one, building an `AggItem` with `unit = "kWh"` and `count = 1`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p aggregations purpose_split 2>&1 | tail -20`
Expected: FAIL until `to_rows` populates the new `purpose` field correctly.

- [ ] **Step 3: Add the route and handler**

```rust
            "get_purpose_split" => {
                api::finish(handle_purpose_split(client, table, &qs).await, Cors::None)
            }
```

```rust
/// `GET /meterdata/query/get_purpose_split` — a node's end-use breakdown for one
/// resource: every declared purpose plus `total` and `unallocated`.
#[utoipa::path(
    get,
    path = "/meterdata/query/get_purpose_split",
    tag = "aggregations",
    params(
        ("level_id" = String, Query, description = "Hierarchy node path"),
        ("resource" = String, Query, description = "Resource to break down (electricity, district_heating, …)"),
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
    let resource = qs.get("resource").cloned().unwrap_or_default();
    if resource.is_empty() {
        return Err(ApiError::bad_request("resource is required"));
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
    // fan-out over the six resources, so one extra round-trip's latency at most.
    let queries = Purpose::all().map(|p| {
        let params = QueryParams {
            table, pk: &pk, sk_path: &sk_path, gran,
            start_bucket: &start_bucket, end_bucket: &end_bucket,
            resource: &resource, purpose: p.as_str(),
        };
        query_one_resource(client, &params, &resource)
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

Import `model::domain::values::Purpose` alongside the existing `Resource` import, and add the path to `openapi_response()`.

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
curl -s "$(aws cloudformation describe-stacks --profile daq_dev \
  --stack-name MeasurementsAggregateStack \
  --query 'Stacks[0].Outputs[?OutputKey==`AggregationsApiUrl`].OutputValue' \
  --output text)/meterdata/query/get_purpose_split?level_id=HN2%23997&resource=electricity&start=2026-07-01T00:00:00Z&end=2026-07-28T00:00:00Z"
```

Expected: JSON rows including `total` and `unallocated`.

- [ ] **Step 6: Commit**

```bash
git add crates/services/aggregations/ infra/daq/data_pipeline/
git commit -m "feat(aggregations): get_purpose_split end-use breakdown"
```

---

### Task 16: End-use breakdown in the node dashboard

**Files:**
- Create: `frontend/src/components/PurposeSplit.astro`
- Modify: the node Data-tab dashboard that already hosts the aggregation widgets (find it with `grep -rn "get_aggregations" frontend/src`)

**Interfaces:**
- Consumes: `GET /meterdata/query/get_purpose_split` (Task 15) via `PUBLIC_AGG_API_BASE_URL`

- [ ] **Step 1: Build the widget**

Create `frontend/src/components/PurposeSplit.astro` — an HTMX fragment loader, no client-side JSON rendering:

```astro
---
const { levelId, resource = "electricity", start, end } = Astro.props;
const base = import.meta.env.PUBLIC_AGG_API_BASE_URL;
const url = `${base}/meterdata/query/get_purpose_split` +
  `?level_id=${encodeURIComponent(levelId)}&resource=${resource}` +
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

- [ ] **Step 2: Make the page soft-nav safe**

Confirm the widget re-fires on soft navigation: the tab content must call `htmx.process` on `astro:page-load` if it isn't already inside `#node-data-panel` (which the Layout processes). Follow whatever the neighbouring widgets in the same file do.

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

Open `https://d24beiqs2cj89y.cloudfront.net`, navigate to a company node's Data tab, and confirm the Formålsopdeling table renders rows including `total` and `unallocated`.

- [ ] **Step 5: Commit**

```bash
git add frontend/
git commit -m "feat(frontend): end-use breakdown widget on the node Data tab"
```

---

### Task 17: End-to-end acceptance against the presentation

**Files:**
- Modify: none (verification only)

The presentation `docs/hierarchy-presentation.html` is a runnable model of the arithmetic. Reproducing its numbers through the real stack is the acceptance test for the whole plan.

- [ ] **Step 1: Author the fixture company through the UI**

On a scratch company, create the node/sensor shape from the presentation and declare, via the Formler tab:

| Node | Formula |
|---|---|
| Chiller | `electricity/total` = phases × 0 |
| Chiller | `electricity/cooling` = accumulator × 1 |
| Chiller | `district_cooling/cooling` = accumulator × 3.2 |
| Area A1b | `electricity/generation` = export × 1 |
| Building A1 | `district_heating/total` = DHW submeter × 0 |
| Building A1 | `district_heating/dhw` = DHW submeter × 1 |
| Building A1 | `district_heating/space_heating` = main × 1 + DHW × −1 |
| Building A2 | `district_heating/dhw` = main × 0.28 |
| Building A2 | `district_heating/space_heating` = main × 0.72 |
| Building B2 | `heat/space_heating` = gas × 10.45 |

- [ ] **Step 2: Run the roll-up**

```bash
aws glue start-job-run --profile daq_dev --job-name measurements-aggregate \
  --arguments '{"--lookback_days":"2"}'
```

- [ ] **Step 3: Assert the four invariants**

Query `get_purpose_split` for each fixture node and check:

1. **No double counting** — the chiller's `electricity/total` equals its `electricity/cooling` (the phases are weight 0, not summed on top of the accumulator).
2. **Exact partition** — Building A1's `district_heating`: `space_heating + dhw == total` and `unallocated == 0`. Same for A2 with the 0.28/0.72 split.
3. **Generation is out of total** — Area A1b's `electricity/total` equals the import meter alone; `generation` reports the export separately; `get_emissions` on `total` charges nothing for the export.
4. **Derived rows float free** — `district_cooling/cooling` and `heat/space_heating` are non-zero while their `total` and `unallocated` are both 0.

- [ ] **Step 4: Record the result**

Append an "Acceptance" section to `docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md` with the four measured values, so the spec records what was actually verified rather than what was intended.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md
git commit -m "docs: record node-formula roll-up acceptance results"
```

---

## Self-Review

**Spec coverage.** §2 D1–D7 → Tasks 1, 2, 3, 4, 11, 13. §3.1 deletions → Task 2. §3.2 rename → Tasks 2, 9, 10, 13, 14. §3.3 types → Tasks 1, 2. §3.4 flatten → Task 3. §3.5 subtree rule → Task 4. §3.6 derived → Tasks 3, 13. §3.7 claim propagation → Tasks 3, 13. §3.8 total semantics → Tasks 1, 3, 4, 13. §4 storage → Task 5. §5 hierarchy service → Tasks 6, 7. §6 bridge + reader role → Task 9. §7 Flink/Iceberg → Task 10. §8.1 keys → Task 13. §8.2 stages → Tasks 11, 13. §8.3 dropped attributes → Task 13. §8.4 parity → Tasks 11, 12. §9 read side → Tasks 14, 15. §11 testing → distributed across every task. §12 deploy order → Tasks 8, 9, 10, 13, 15, 16. §13 out of scope → not planned, correctly.

**Gap found and closed:** §5.2's "Formler tab" needed the descendant-only reference picker to be *tested*, not just described — added `node_formulas_reference_picker_lists_only_descendants` in Task 7.

**Gap found and closed:** the spec's §12 says wipe `measurements_aggregate` and re-run, but never said the old rows become unreadable when the sort key gains a segment — made explicit in Task 13 Step 8.

**Type consistency.** `Reference`/`Term`/`NodeFormula` (Task 2) are used unchanged in Tasks 3–7. `CompanyGraph`/`WeightRow`/`flatten`/`total_weight`/`is_derived` (Task 3) are consumed with identical signatures in Tasks 4, 6, 12. The PySpark mirror (Task 11) uses `sensor_id` where Rust uses `sensor` — deliberate, and the parity test in Task 12 maps between them explicitly. `build_sk` gains its `purpose` parameter in Task 13 and every call site in that task passes five arguments. `tariff_dkk_per_unit`/`emission_kg_per_unit` gain `purpose` in Task 14 and `scale_rows`'s closure signature is updated in the same task.
