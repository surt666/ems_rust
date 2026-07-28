# Node-Formula Roll-up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce sensors to one kind that carries only what it measures, and move all combination logic onto hierarchy nodes as weighted linear formulas — while splitting the mis-named `purpose` axis into `energy_type` and `purpose`.

**Architecture:** A node's value is Σ of its **children's values** plus its own sensors, unless the node declares a formula overriding some of those weights. Evaluation is recursive, so a node that corrects itself is right at every ancestor automatically. Because every term is linear, `crates/model` flattens the whole recursion into a per-`(node, energy_type, purpose, sensor)` coefficient matrix, which the hierarchy service materialises into `hierarchy_new`. The Glue job reads that matrix cross-account and becomes one join plus one grouped sum, with no formula semantics in PySpark at all.

**Tech Stack:** Rust (`model`, `api`, `services/hierarchy`, `services/aggregations`), maud + HTMX server-rendered HTML, DynamoDB (`hierarchy_new`, `measurements_aggregate`), Scala/Flink on MSF, Iceberg S3 Tables, PySpark on Glue, Go CDK, Astro frontend.

**Spec:** `docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md` — read it first. `docs/hierarchy-presentation.html` is a runnable model of the arithmetic; **its numbers are the acceptance values** used throughout this plan, and it is worth opening before starting Task 3.

## Global Constraints

- Work directly on `main`. No feature branches. Do **not** `git push` — commit locally only.
- **Dev system**: rename outright, no read-fallbacks, no dual-writes, no back-compat shims. Data loss is acceptable when chosen deliberately.
- Accounts, both `eu-central-1`: hierarchy/frontend `339712745226` (profile `stel-sb`), DAQ/pipeline `891377204778` (profile `daq_dev`).
- `unset GOROOT` before **every** `cdk` command. Always `cdk diff` before `cdk deploy`.
- Never lead a chained shell command with `pkill`/`pgrep` or anything whose non-zero exit is normal — it aborts the rest of the chain.
- Rust: `cargo test` from the repo root must pass; `cargo clippy` warning-free.
- Wire tokens are lower-case `snake_case`, keyed verbatim into DynamoDB sort keys. `Display` is the storage contract.
- UI is HTML-over-the-wire (HTMX); never client-side JSON rendering. CSS uses **grid**, never flexbox.
- **Vocabulary:** the inputs are **sensors** (one per device channel/register). A *meter* is a physical device and exists in the hierarchy only as a **node type**. State every rule over sensors.
- Renames in this change: `Resource` → `EnergyType`, `MeterType` → `ReadingKind`, `logical_meter_data` → `logical_data`, `meter-identity` → `sensor-identity`, `MeterMapping` → `SensorMapping`. The `/meterdata/` route prefix is deliberately **not** renamed.
- Roll-up sort key: `<node_path>#<energy_type>#<purpose>#<gran>#<bucket>`; GSI `gsi1pk = HN2#<id>#<dimension>#<purpose>`, `gsi1sk = <node_path>#<gran>#<bucket>`.

---

# Phase 1 — Domain + hierarchy service

Formulas can be authored, validated, flattened and materialised; nothing downstream reads them yet.

**This phase does *not* ship independently, despite reading as if it does.** Task 1's attribute
rename requires a migration of the stored `hierarchy_new` sensor rows (added as
`migrations/2026-07-28-sensor-attr-rename.sh` during execution — the original plan omitted it),
and that migration's `UpdateItem` calls fire the table's DynamoDB stream. The cross-account
bridge reads the old attribute names off that stream, so it breaks the moment the migration runs
— which is what happened: `KeyError: 'meter_type'`, four batches to the DLQ, and every subsequent
sensor change silently failing to reach `meter-identity`. Commit `22d2144` fixed it by having the
bridge translate between the renamed hierarchy vocabulary and the not-yet-renamed pipeline one.
**Any future rename on either side of that stream must ship with the bridge, not before it.**

---

### Task 1: `EnergyType`, `ReadingKind`, `Purpose`

**Files:**
- Modify: `crates/model/src/domain/values.rs` (`Resource` block ~226-298, `MeterType` block ~211-222, and `mod tests`)
- Modify: every `Resource` / `MeterType` reference the compiler flags

**Interfaces:**
- Produces: `EnergyType` (was `Resource`; same six variants, tokens and `dimension()`), `ReadingKind` (was `MeterType`; same `counter`/`gauge` tokens), and `Purpose` with `as_str()`, `all()`, `declarable()`, `is_outflow()`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/model/src/domain/values.rs`:

```rust
    /// Wire tokens are the `measurements_aggregate` sort-key contract.
    #[test]
    fn purpose_wire_tokens_round_trip() {
        for p in Purpose::iter() {
            assert_eq!(p.to_string().parse::<Purpose>().unwrap(), p);
        }
        assert_eq!(Purpose::SpaceHeating.to_string(), "space_heating");
        assert_eq!(Purpose::Dhw.to_string(), "dhw");
        assert_eq!(Purpose::Total.to_string(), "total");
    }

    /// `Total` IS the node's own formula, so it must be declarable. Only
    /// `Unallocated` is job-derived.
    #[test]
    fn total_is_declarable_unallocated_is_not() {
        assert!(Purpose::Total.declarable());
        assert!(!Purpose::Unallocated.declarable());
        assert!(Purpose::Generation.declarable());
    }

    /// Generation reports exported energy; it never reduces Unallocated.
    #[test]
    fn purpose_outflow() {
        assert!(Purpose::Generation.is_outflow());
        assert!(!Purpose::Cooling.is_outflow());
        assert!(!Purpose::Total.is_outflow());
    }

    /// Both renames keep their wire contracts exactly.
    #[test]
    fn renames_keep_their_wire_contracts() {
        for e in EnergyType::iter() {
            assert_eq!(e.to_string().parse::<EnergyType>().unwrap(), e);
        }
        assert_eq!(EnergyType::DistrictHeating.to_string(), "district_heating");
        assert_eq!(EnergyType::Water.dimension(), Dimension::Volume);
        assert_eq!(ReadingKind::Counter.to_string(), "counter");
        assert_eq!("gauge".parse::<ReadingKind>().unwrap(), ReadingKind::Gauge);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model purpose_ 2>&1 | tail -20`
Expected: FAIL — `cannot find type Purpose in this scope`.

- [ ] **Step 3: Rename both types**

Rename `Resource` → `EnergyType` and `MeterType` → `ReadingKind` in `values.rs`, then let `cargo build` point at every call site. `Sensor.meter_type` becomes `Sensor.reading_kind`; the DynamoDB attribute follows. **Do not blind-`sed`** — `resource` appears in `RepositoryError`, CDK `Resources:` and `boto3.resource`.

- [ ] **Step 4: Implement `Purpose`**

Insert after the `impl EnergyType` block:

```rust
/// The **formål** — what the energy is spent on. Independent of [`EnergyType`]:
/// electricity serves lighting, cooling and ventilation alike, and space heating
/// can arrive as district heating, gas or a heat pump. Taxonomy follows
/// Energihåndbogen 2019's chapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq,
         strum::Display, strum::EnumString, EnumIter, strum::IntoStaticStr)]
#[strum(ascii_case_insensitive)]
pub enum Purpose {
    #[strum(serialize = "space_heating")] SpaceHeating,
    #[strum(serialize = "dhw")] Dhw,
    #[strum(serialize = "ventilation")] Ventilation,
    #[strum(serialize = "cooling")] Cooling,
    #[strum(serialize = "lighting")] Lighting,
    #[strum(serialize = "plug_loads")] PlugLoads,
    #[strum(serialize = "ev_charging")] EvCharging,
    #[strum(serialize = "process")] Process,
    #[strum(serialize = "common")] Common,
    /// Egenproduktion (PV export). Reported, but never reduces `Unallocated` —
    /// exported energy is not a slice of consumption.
    #[strum(serialize = "generation")] Generation,
    /// The node's own value. Declarable: a formula with this head **is** the
    /// node's formula. Defaults to Σ children + own sensors.
    #[strum(serialize = "total")] Total,
    /// `Total − Σ(allocating purposes)`. Derived by the roll-up; never declarable.
    #[strum(serialize = "unallocated")] Unallocated,
}

impl Purpose {
    pub fn as_str(self) -> &'static str { self.into() }
    pub fn all() -> impl Iterator<Item = Purpose> { Purpose::iter() }

    /// Whether a node formula may declare this purpose as its output.
    pub const fn declarable(self) -> bool { !matches!(self, Purpose::Unallocated) }

    /// Whether claims of this purpose leave the site rather than being consumed
    /// on it — such claims never reduce `Unallocated`.
    pub const fn is_outflow(self) -> bool { matches!(self, Purpose::Generation) }
}
```

- [ ] **Step 5: Run the full suite**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A crates/
git commit -m "feat(model)!: Resource -> EnergyType, MeterType -> ReadingKind; add the Purpose axis"
```

---

### Task 2: Node-formula types; the sensor becomes bare

**Files:**
- Create: `crates/model/src/domain/node_formula.rs`
- Delete: `crates/model/src/domain/formula.rs`
- Modify: `crates/model/src/domain/mod.rs`, `crates/model/src/domain/sensor.rs`
- Modify: `crates/model/src/repository/dynamodb/codec.rs`, `crates/model/src/logic/sensors.rs`

**Interfaces:**
- Produces:
  - `Reference { Sensor(SensorId), Node(NodeId) }` with `parse(&str)` and `Display`
  - `Term { reference: Reference, coefficient: f64 }`
  - `NodeFormula { node, energy_type, purpose, terms, note }` with `sk() -> "formula#<energy_type>#<purpose>"`
  - `sensor::parent_path(&Sensor) -> &str`
  - `Sensor` with **no** `formula` — only `id, created, daq_id, path, energy_type, reading_kind, unit, resample_minutes`

- [ ] **Step 1: Write the failing tests**

Create `crates/model/src/domain/node_formula.rs` with only its test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::Level;

    #[test]
    fn reference_parses_sensors_and_nodes() {
        assert_eq!(Reference::parse("S#20001").unwrap(),
                   Reference::Sensor(SensorId::make(20001)));
        assert_eq!(Reference::parse("HN5#10042").unwrap(),
                   Reference::Node(NodeId::make(Level::Hn5, 10042)));
        assert!(Reference::parse("nonsense").is_err());
    }

    #[test]
    fn reference_display_round_trips() {
        for s in ["S#20001", "HN5#10042", "HN2#997"] {
            assert_eq!(Reference::parse(s).unwrap().to_string(), s);
        }
    }

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

    /// A node's own formula is a `Total` formula — the same shape, not a special case.
    #[test]
    fn a_total_formula_is_an_ordinary_formula() {
        let f = NodeFormula {
            node: NodeId::make(Level::Hn5, 5),
            energy_type: EnergyType::Electricity,
            purpose: Purpose::Total,
            terms: vec![Term {
                reference: Reference::Sensor(SensorId::make(2)),
                coefficient: 0.0,
            }],
            note: Some("faserne er allerede med i akkumulatoren".to_string()),
        };
        assert_eq!(f.sk(), "formula#electricity#total");
        assert_eq!(f.terms[0].coefficient, 0.0);
    }
}
```

Add to `mod tests` in `crates/model/src/domain/sensor.rs`:

```rust
    /// The node a sensor hangs off — its path minus the trailing `|S#<id>`.
    #[test]
    fn parent_path_strips_the_sensor_segment() {
        assert_eq!(parent_path(&make_sample()), "HN0#root|HN5#10042");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model node_formula 2>&1 | tail -20`
Expected: FAIL — module not declared, then `cannot find type Reference`.

- [ ] **Step 3: Implement the types**

Prepend to `crates/model/src/domain/node_formula.rs`:

```rust
//! Node formulas. There is one kind of sensor, carrying only what it measures;
//! everything about how readings combine lives here, on the node.
//!
//! A node's value is Σ everything below it, unless the node says otherwise. A
//! formula lists only the terms whose weight differs from the default 1.

use std::fmt;

use crate::domain::ids::{NodeId, SensorId};
use crate::domain::values::{EnergyType, Purpose};

/// What a term points at. Sensors may be anywhere in the company; nodes must be
/// direct children of the declaring node (see `logic::formulas`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    Sensor(SensorId),
    Node(NodeId),
}

impl Reference {
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

/// One weighted term. The coefficient covers every case the model supports:
/// exclude 0, include 1, subtract −1, apportion 0.28, COP 3.2,
/// brændværdi × virkningsgrad 10.45.
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub reference: Reference,
    pub coefficient: f64,
}

/// A node's declared output for one `(energy_type, purpose)`. `purpose = Total`
/// is the node's own value; anything else is a purpose claim.
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

Replace `pub mod formula;` with `pub mod node_formula;` in `domain/mod.rs`.

- [ ] **Step 4: Strip the sensor**

In `crates/model/src/domain/sensor.rs`, delete the `Formula` import and the two `formula` struct lines, then add next to `parent_id`:

```rust
/// The path of the node a sensor hangs off — its own path minus the trailing
/// `|S#<id>` segment.
pub fn parent_path(s: &Sensor) -> &str {
    match s.path.rfind(PATH_SEP) {
        Some(i) => &s.path[..i],
        None => &s.path,
    }
}
```

Then: `rm crates/model/src/domain/formula.rs`; delete `walk_refs_sync`, `has_cycle`, `set_formula` and `evaluate` from `logic/sensors.rs` along with `attach`'s formula parameter and its post-allocation cycle check and rollback; drop the `formula` attribute from the sensor codec; and fix the fallout in `crates/services/hierarchy` (Task 7 rewrites the UI properly — here just delete the plumbing).

- [ ] **Step 5: Verify**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS. Delete any test still referencing `Formula` rather than adapting it.

- [ ] **Step 6: Commit**

```bash
git add -A crates/
git commit -m "feat(model)!: node-formula types; the sensor carries only what it measures"
```

---

### Task 3: Recursive evaluation and flattening

**Files:**
- Create: `crates/model/src/logic/formulas.rs`
- Modify: `crates/model/src/logic/mod.rs`

**Interfaces:**
- Produces:
  - `CompanyGraph { nodes, sensors, formulas }` with `children`, `own_sensors`, `node_path`, `sensor`
  - `MatrixRow { node_path, energy_type, purpose, sensor, coefficient, allocates }`
  - `coeffs(&CompanyGraph, &NodeId, EnergyType, Purpose) -> BTreeMap<SensorId, f64>`
  - `is_derived(&CompanyGraph, &NodeFormula) -> bool`
  - `flatten(&CompanyGraph) -> Vec<MatrixRow>`

**The two defaults (spec §3.5) — get these exactly right:**
- `Total`: every **child node** and every **own sensor of that energy type** default to weight 1.
- A named **purpose**: every **child node** defaults to 1; **sensors count only if named**.
- `Unallocated` = `Total − Σ(allocating purposes)`, where a purpose allocates when it is neither derived nor an outflow.

- [ ] **Step 1: Write the failing tests**

Create `crates/model/src/logic/formulas.rs` with only its test module. The fixtures come from `docs/hierarchy-presentation.html`, so the expected numbers are the demo's verified ones:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::Level;
    use crate::domain::node::Node;
    use crate::domain::node_formula::Term;
    use crate::domain::sensor::Sensor;
    use crate::domain::values::ReadingKind;

    const CO: &str = "HN0#root|HN2#997";

    fn node(level: Level, id: u32, parent: Option<(Level, u32)>, path: &str) -> Node {
        Node::builder()
            .id(NodeId::make(level, id))
            .name(format!("n{id}"))
            .parent(parent.map(|(l, i)| NodeId::make(l, i)))
            .path(path.to_string())
            .build()
    }

    fn sensor(id: u32, node_path: &str, et: EnergyType) -> Sensor {
        Sensor::builder()
            .id(SensorId::make(id))
            .daq_id(format!("daq{id}"))
            .path(format!("{node_path}|S#{id}"))
            .energy_type(et)
            .reading_kind(ReadingKind::Counter)
            .build()
    }

    fn f(node: (Level, u32), et: EnergyType, pur: Purpose,
         terms: &[(&str, f64)]) -> NodeFormula {
        NodeFormula {
            node: NodeId::make(node.0, node.1),
            energy_type: et,
            purpose: pur,
            terms: terms.iter()
                .map(|(r, c)| Term {
                    reference: Reference::parse(r).unwrap(), coefficient: *c })
                .collect(),
            note: None,
        }
    }

    /// Evaluate the flattened coefficients against a reading set.
    fn value(g: &CompanyGraph, n: (Level, u32), et: EnergyType, pur: Purpose,
             readings: &[(u32, f64)]) -> f64 {
        coeffs(g, &NodeId::make(n.0, n.1), et, pur).iter()
            .map(|(s, c)| c * readings.iter()
                .find(|(id, _)| *id == s.id()).map_or(0.0, |(_, v)| *v))
            .sum()
    }

    /// The chiller: one device, an accumulating channel (S#1) and three phase
    /// channels, all on one node. The formula zeroes the phases.
    fn chiller() -> (CompanyGraph, Vec<(u32, f64)>) {
        let chill = format!("{CO}|HN5#5");
        let g = CompanyGraph {
            nodes: vec![
                node(Level::Hn2, 997, None, CO),
                node(Level::Hn5, 5, Some((Level::Hn2, 997)), &chill),
            ],
            sensors: (1..=4).map(|i| sensor(i, &chill, EnergyType::Electricity)).collect(),
            formulas: vec![
                f((Level::Hn5, 5), EnergyType::Electricity, Purpose::Total,
                  &[("S#2", 0.0), ("S#3", 0.0), ("S#4", 0.0)]),
                f((Level::Hn5, 5), EnergyType::Electricity, Purpose::Cooling, &[("S#1", 1.0)]),
            ],
        };
        (g, vec![(1, 40.0), (2, 13.0), (3, 14.0), (4, 13.0)])
    }

    #[test]
    fn total_defaults_to_the_sum_of_own_sensors() {
        let (mut g, r) = chiller();
        g.formulas.clear();
        assert_eq!(value(&g, (Level::Hn5, 5), EnergyType::Electricity, Purpose::Total, &r),
                   80.0, "no formula: everything counts");
    }

    #[test]
    fn a_total_formula_overrides_the_default() {
        let (g, r) = chiller();
        assert_eq!(value(&g, (Level::Hn5, 5), EnergyType::Electricity, Purpose::Total, &r),
                   40.0, "phases zeroed, accumulator only");
    }

    /// Recursion: a parent sums its CHILDREN'S VALUES, so a node that corrects
    /// itself is right at every ancestor with nothing to restate upward.
    #[test]
    fn ancestors_inherit_a_childs_correction() {
        let (_, r) = chiller();
        let bld = format!("{CO}|HN4#4");
        let chill = format!("{bld}|HN5#5");
        let g = CompanyGraph {
            nodes: vec![
                node(Level::Hn2, 997, None, CO),
                node(Level::Hn4, 4, Some((Level::Hn2, 997)), &bld),
                node(Level::Hn5, 5, Some((Level::Hn4, 4)), &chill),
            ],
            sensors: (1..=4).map(|i| sensor(i, &chill, EnergyType::Electricity)).collect(),
            formulas: vec![f((Level::Hn5, 5), EnergyType::Electricity, Purpose::Total,
                             &[("S#2", 0.0), ("S#3", 0.0), ("S#4", 0.0)])],
        };
        let el = EnergyType::Electricity;
        assert_eq!(value(&g, (Level::Hn4, 4), el, Purpose::Total, &r), 40.0,
                   "the building sees 40, not 80");
        assert_eq!(value(&g, (Level::Hn2, 997), el, Purpose::Total, &r), 40.0,
                   "and so does the company");
    }

    /// A sensor belongs to `Total` automatically but to a purpose only if named —
    /// otherwise attaching a meter would silently claim it as lighting.
    #[test]
    fn a_purpose_takes_no_sensor_unless_named() {
        let (g, r) = chiller();
        let n = (Level::Hn5, 5);
        let el = EnergyType::Electricity;
        assert_eq!(value(&g, n, el, Purpose::Cooling, &r), 40.0, "named");
        assert_eq!(value(&g, n, el, Purpose::Lighting, &r), 0.0, "not named");
    }

    /// Sideways sensor reference: main in C1, sub in C2 (spec §3.6).
    fn split_metering() -> (CompanyGraph, Vec<(u32, f64)>) {
        let pc = format!("{CO}|HN3#3");
        let c1 = format!("{pc}|HN4#1");
        let c2 = format!("{pc}|HN4#2");
        let g = CompanyGraph {
            nodes: vec![
                node(Level::Hn2, 997, None, CO),
                node(Level::Hn3, 3, Some((Level::Hn2, 997)), &pc),
                node(Level::Hn4, 1, Some((Level::Hn3, 3)), &c1),
                node(Level::Hn4, 2, Some((Level::Hn3, 3)), &c2),
            ],
            sensors: vec![sensor(10, &c1, EnergyType::Electricity),
                          sensor(11, &c2, EnergyType::Electricity)],
            formulas: vec![f((Level::Hn4, 1), EnergyType::Electricity, Purpose::Total,
                             &[("S#11", -1.0)])],
        };
        (g, vec![(10, 100.0), (11, 30.0)])
    }

    #[test]
    fn main_and_sub_in_different_buildings() {
        let (g, r) = split_metering();
        let el = EnergyType::Electricity;
        assert_eq!(value(&g, (Level::Hn4, 1), el, Purpose::Total, &r), 70.0, "C1 = main − sub");
        assert_eq!(value(&g, (Level::Hn4, 2), el, Purpose::Total, &r), 30.0, "C2 = sub");
        assert_eq!(value(&g, (Level::Hn3, 3), el, Purpose::Total, &r), 100.0,
                   "the property is the main, counted exactly once");
    }

    /// Shared plant apportioned across siblings — impossible under a
    /// descendants-only rule, natural with company-wide sensor references.
    #[test]
    fn shared_plant_splits_across_siblings() {
        let (mut g, mut r) = split_metering();
        r.push((12, 50.0));
        g.sensors.push(sensor(12, &format!("{CO}|HN3#3"), EnergyType::Electricity));
        g.formulas.push(f((Level::Hn4, 1), EnergyType::Electricity, Purpose::Cooling,
                          &[("S#12", 0.6)]));
        g.formulas.push(f((Level::Hn4, 2), EnergyType::Electricity, Purpose::Cooling,
                          &[("S#12", 0.4)]));
        assert_eq!(value(&g, (Level::Hn3, 3), EnergyType::Electricity, Purpose::Cooling, &r),
                   50.0, "0.6 + 0.4 = one chiller");
    }

    /// Cross-type output marks a claim derived; derived and outflow claims never
    /// reduce Unallocated.
    #[test]
    fn derived_and_outflow_do_not_allocate() {
        let (mut g, _) = chiller();
        g.formulas.push(f((Level::Hn5, 5), EnergyType::DistrictCooling, Purpose::Cooling,
                          &[("S#1", 3.2)]));
        g.formulas.push(f((Level::Hn5, 5), EnergyType::Electricity, Purpose::Generation,
                          &[("S#1", 1.0)]));
        let rows = flatten(&g);
        let allocates = |et: EnergyType, p: Purpose| rows.iter()
            .find(|r| r.energy_type == et && r.purpose == p).unwrap().allocates;
        assert!(allocates(EnergyType::Electricity, Purpose::Cooling));
        assert!(!allocates(EnergyType::DistrictCooling, Purpose::Cooling), "derived");
        assert!(!allocates(EnergyType::Electricity, Purpose::Generation), "outflow");
    }

    /// Unallocated arrives as ordinary matrix rows, so the roll-up job subtracts
    /// nothing itself.
    #[test]
    fn unallocated_is_total_minus_allocating() {
        let (g, r) = chiller();
        let el = EnergyType::Electricity;
        assert_eq!(value(&g, (Level::Hn5, 5), el, Purpose::Unallocated, &r), 0.0,
                   "40 total − 40 cooling");
        assert!(flatten(&g).iter().any(|w| w.purpose == Purpose::Unallocated),
                "and it is materialised, not computed downstream");
    }

    /// A partly-claimed node reports the gap rather than hiding it.
    #[test]
    fn unallocated_reports_the_gap() {
        let (mut g, r) = chiller();
        g.formulas.retain(|x| x.purpose != Purpose::Cooling);
        assert_eq!(value(&g, (Level::Hn5, 5), EnergyType::Electricity,
                         Purpose::Unallocated, &r), 40.0);
    }

    /// flatten() must agree with the recursion it flattens.
    #[test]
    fn flatten_agrees_with_the_recursion() {
        let (g, r) = chiller();
        for purpose in [Purpose::Total, Purpose::Cooling, Purpose::Unallocated] {
            let direct = value(&g, (Level::Hn5, 5), EnergyType::Electricity, purpose, &r);
            let flat: f64 = flatten(&g).iter()
                .filter(|w| w.node_path == format!("{CO}|HN5#5")
                            && w.energy_type == EnergyType::Electricity
                            && w.purpose == purpose)
                .map(|w| w.coefficient * r.iter()
                    .find(|(id, _)| *id == w.sensor.id()).map_or(0.0, |(_, v)| *v))
                .sum();
            assert_eq!(direct, flat, "{purpose}");
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model formulas:: 2>&1 | tail -20`
Expected: FAIL — `cannot find type CompanyGraph`.

- [ ] **Step 3: Implement**

Prepend to `crates/model/src/logic/formulas.rs`:

```rust
//! Recursive evaluation of node formulas, and its flattening into a coefficient
//! matrix.
//!
//! A node's value is Σ of its CHILDREN'S VALUES plus its own sensors, unless the
//! node's formula overrides some of those weights. Because every term is linear,
//! the whole recursion collapses to `value = Σ_sensor coefficient × reading`,
//! which is what the roll-up job consumes — so these rules live here and only here.

use std::collections::BTreeMap;

use crate::domain::ids::{NodeId, SensorId};
use crate::domain::node::Node;
use crate::domain::node_formula::{NodeFormula, Reference};
use crate::domain::sensor::{parent_path, Sensor};
use crate::domain::values::{EnergyType, Purpose};

/// Everything under one company (HN2) needed to evaluate its formulas.
#[derive(Clone, Debug, Default)]
pub struct CompanyGraph {
    pub nodes: Vec<Node>,
    pub sensors: Vec<Sensor>,
    pub formulas: Vec<NodeFormula>,
}

/// One `(node, energy_type, purpose, sensor)` coefficient.
/// `value(node, et, purpose) = Σ_sensor coefficient × reading(sensor)`.
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixRow {
    pub node_path: String,
    pub energy_type: EnergyType,
    pub purpose: Purpose,
    pub sensor: SensorId,
    pub coefficient: f64,
    /// Whether this purpose reduces `Unallocated` — false for derived rows
    /// (delivered energy, not metered consumption) and for outflow purposes.
    pub allocates: bool,
}

pub type Coeffs = BTreeMap<SensorId, f64>;

impl CompanyGraph {
    pub fn node_path(&self, id: &NodeId) -> Option<&str> {
        self.nodes.iter().find(|n| &n.id == id).map(|n| n.path.as_str())
    }
    pub fn children(&self, id: &NodeId) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(move |n| n.parent.as_ref() == Some(id))
    }
    pub fn own_sensors(&self, id: &NodeId) -> Vec<&Sensor> {
        let Some(path) = self.node_path(id) else { return vec![] };
        self.sensors.iter().filter(|s| parent_path(s) == path).collect()
    }
    pub fn sensor(&self, id: SensorId) -> Option<&Sensor> {
        self.sensors.iter().find(|s| s.id == id)
    }
    fn formula(&self, node: &NodeId, et: EnergyType, p: Purpose) -> Option<&NodeFormula> {
        self.formulas.iter()
            .find(|f| &f.node == node && f.energy_type == et && f.purpose == p)
    }
}

fn add(into: &mut Coeffs, sensor: SensorId, c: f64) {
    if c == 0.0 { return; }
    let e = into.entry(sensor).or_insert(0.0);
    *e += c;
    if *e == 0.0 { into.remove(&sensor); }
}

/// The coefficient vector of `value(node, et, purpose)` over sensors (spec §3.5).
pub fn coeffs(g: &CompanyGraph, node: &NodeId, et: EnergyType, purpose: Purpose) -> Coeffs {
    if purpose == Purpose::Unallocated {
        let mut out = coeffs(g, node, et, Purpose::Total);
        for p in allocating_purposes(g, et) {
            for (s, c) in coeffs(g, node, et, p) {
                add(&mut out, s, -c);
            }
        }
        return out;
    }

    let f = g.formula(node, et, purpose);
    let weight_of = |r: &Reference| f.and_then(|f| {
        f.terms.iter().find(|t| &t.reference == r).map(|t| t.coefficient)
    });

    let mut out = Coeffs::new();

    // Children default to 1, for Total and for a named purpose alike.
    for child in g.children(node) {
        let w = weight_of(&Reference::Node(child.id.clone())).unwrap_or(1.0);
        if w == 0.0 { continue; }
        for (s, c) in coeffs(g, &child.id, et, purpose) {
            add(&mut out, s, w * c);
        }
    }

    // Own sensors default to 1 for Total ONLY. A sensor does not belong to a
    // purpose unless the formula names it.
    if purpose == Purpose::Total {
        for s in g.own_sensors(node).into_iter().filter(|s| s.energy_type == et) {
            add(&mut out, s.id, weight_of(&Reference::Sensor(s.id)).unwrap_or(1.0));
        }
    }

    // Named sensors. For Total the node's own ones were just handled with their
    // override applied; what remains are sensors elsewhere in the company
    // (spec §3.6). For a purpose, every named sensor counts.
    if let Some(f) = f {
        let own: Vec<SensorId> = g.own_sensors(node).iter().map(|s| s.id).collect();
        for t in &f.terms {
            if let Reference::Sensor(id) = t.reference {
                if purpose == Purpose::Total && own.contains(&id) { continue; }
                add(&mut out, id, t.coefficient);
            }
        }
    }

    out
}

/// A formula is derived when its output energy type differs from that of a
/// sensor it names directly. Node references resolve to the formula's own
/// energy type and never make it derived.
pub fn is_derived(g: &CompanyGraph, f: &NodeFormula) -> bool {
    f.terms.iter().any(|t| match &t.reference {
        Reference::Sensor(id) => g.sensor(*id).is_some_and(|s| s.energy_type != f.energy_type),
        Reference::Node(_) => false,
    })
}

/// Purposes declared for `et` that reduce `Unallocated`: neither derived nor outflow.
fn allocating_purposes(g: &CompanyGraph, et: EnergyType) -> Vec<Purpose> {
    let mut ps: Vec<Purpose> = g.formulas.iter()
        .filter(|f| f.energy_type == et
                 && f.purpose != Purpose::Total
                 && !f.purpose.is_outflow()
                 && !is_derived(g, f))
        .map(|f| f.purpose)
        .collect();
    ps.sort_by_key(|p| p.as_str());
    ps.dedup();
    ps
}

/// Every series the roll-up needs — `Total`, each declared purpose, and
/// `Unallocated` — for every node and energy type in the company.
pub fn flatten(g: &CompanyGraph) -> Vec<MatrixRow> {
    let mut out = Vec::new();
    for n in &g.nodes {
        for et in EnergyType::all() {
            let mut purposes = vec![Purpose::Total, Purpose::Unallocated];
            purposes.extend(g.formulas.iter()
                .filter(|f| f.energy_type == et && f.purpose != Purpose::Total)
                .map(|f| f.purpose));
            purposes.sort_by_key(|p| p.as_str());
            purposes.dedup();

            for purpose in purposes {
                let allocates = purpose != Purpose::Unallocated
                    && !purpose.is_outflow()
                    && !g.formulas.iter().any(|f| f.energy_type == et
                                                && f.purpose == purpose
                                                && is_derived(g, f));
                for (sensor, coefficient) in coeffs(g, &n.id, et, purpose) {
                    out.push(MatrixRow {
                        node_path: n.path.clone(),
                        energy_type: et,
                        purpose,
                        sensor,
                        coefficient,
                        allocates,
                    });
                }
            }
        }
    }
    out
}
```

Add `pub mod formulas;` to `crates/model/src/logic/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p model formulas:: 2>&1 | tail -20`
Expected: PASS (10 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/model/src/logic/
git commit -m "feat(model): recursive node-formula evaluation flattened to a coefficient matrix"
```

---

### Task 4: Validation

**Files:**
- Modify: `crates/model/src/logic/formulas.rs` (append `validate` + tests)

**Interfaces:**
- Produces: `validate(&CompanyGraph, &NodeFormula) -> Result<(), String>`

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests`:

```rust
    fn ok() -> NodeFormula {
        f((Level::Hn5, 5), EnergyType::Electricity, Purpose::Cooling, &[("S#1", 1.0)])
    }

    #[test]
    fn validate_accepts_a_well_formed_formula() {
        assert!(validate(&chiller().0, &ok()).is_ok());
    }

    /// `Total` is the node's own formula and must be declarable; only
    /// `Unallocated` is job-derived.
    #[test]
    fn validate_accepts_total_and_rejects_unallocated() {
        let (g, _) = chiller();
        assert!(validate(&g, &NodeFormula { purpose: Purpose::Total, ..ok() }).is_ok());
        let e = validate(&g, &NodeFormula { purpose: Purpose::Unallocated, ..ok() }).unwrap_err();
        assert!(e.contains("roll-up job"), "got: {e}");
    }

    /// Zero is legal — it is how a node excludes a reading.
    #[test]
    fn validate_accepts_zero_and_rejects_non_finite() {
        let (g, _) = chiller();
        assert!(validate(&g, &NodeFormula {
            purpose: Purpose::Total,
            terms: vec![Term { reference: Reference::parse("S#2").unwrap(), coefficient: 0.0 }],
            ..ok()
        }).is_ok());
        let e = validate(&g, &NodeFormula {
            terms: vec![Term { reference: Reference::parse("S#1").unwrap(),
                               coefficient: f64::NAN }],
            ..ok()
        }).unwrap_err();
        assert!(e.contains("finite"));
    }

    /// Node references must be DIRECT CHILDREN — a deeper node already arrives
    /// through the chain, so referencing it would double count.
    #[test]
    fn validate_rejects_a_node_reference_that_is_not_a_direct_child() {
        let (g, _) = split_metering();
        let bad = f((Level::Hn2, 997), EnergyType::Electricity, Purpose::Total,
                    &[("HN4#1", 0.5)]);
        assert!(validate(&g, &bad).unwrap_err().contains("direct child"));
    }

    #[test]
    fn validate_accepts_a_direct_child_reference() {
        let (g, _) = split_metering();
        let good = f((Level::Hn3, 3), EnergyType::Electricity, Purpose::Total,
                     &[("HN4#1", 0.5)]);
        assert!(validate(&g, &good).is_ok());
    }

    /// Sideways SENSOR references are the point of D7 — accept them.
    #[test]
    fn validate_accepts_a_sensor_from_another_branch() {
        let (g, _) = split_metering();
        assert!(validate(&g, &g.formulas[0].clone()).is_ok());
    }

    /// But for `Total`, a sensor deeper in this node's own subtree already
    /// arrives via the child chain.
    #[test]
    fn validate_rejects_a_total_term_naming_a_deeper_descendant_sensor() {
        let (g, _) = split_metering();
        let bad = f((Level::Hn3, 3), EnergyType::Electricity, Purpose::Total,
                    &[("S#10", 0.0)]);   // S#10 hangs off HN4#1, a child
        assert!(validate(&g, &bad).unwrap_err().contains("already counted"));
    }

    #[test]
    fn validate_rejects_a_sensor_outside_the_company() {
        let (g, _) = chiller();
        let bad = f((Level::Hn5, 5), EnergyType::Electricity, Purpose::Cooling,
                    &[("S#999", 1.0)]);
        assert!(validate(&g, &bad).unwrap_err().contains("not found"));
    }

    /// The ≤ 1 rule allows every legitimate split and catches over-allocation.
    #[test]
    fn validate_allows_a_heat_pump_split_summing_to_one() {
        let (mut g, _) = chiller();
        g.formulas.retain(|x| x.purpose != Purpose::Cooling);
        g.formulas.push(f((Level::Hn5, 5), EnergyType::Electricity, Purpose::SpaceHeating,
                          &[("S#1", 0.7)]));
        let dhw = f((Level::Hn5, 5), EnergyType::Electricity, Purpose::Dhw, &[("S#1", 0.3)]);
        assert!(validate(&g, &dhw).is_ok(), "0.7 + 0.3 = 1.0");
    }

    #[test]
    fn validate_rejects_over_allocation() {
        let (mut g, _) = chiller();
        g.formulas.retain(|x| x.purpose != Purpose::Cooling);
        g.formulas.push(f((Level::Hn5, 5), EnergyType::Electricity, Purpose::SpaceHeating,
                          &[("S#1", 0.8)]));
        let dhw = f((Level::Hn5, 5), EnergyType::Electricity, Purpose::Dhw, &[("S#1", 0.5)]);
        assert!(validate(&g, &dhw).unwrap_err().contains("sum"));
    }

    /// The bimåler pattern nets to 0 for the submeter and 1 for the main.
    #[test]
    fn validate_accepts_the_bimaaler_pattern() {
        let b = format!("{CO}|HN4#9");
        let mut g = CompanyGraph {
            nodes: vec![node(Level::Hn2, 997, None, CO),
                        node(Level::Hn4, 9, Some((Level::Hn2, 997)), &b)],
            sensors: vec![sensor(30, &b, EnergyType::DistrictHeating),
                          sensor(31, &b, EnergyType::DistrictHeating)],
            formulas: vec![],
        };
        let dh = EnergyType::DistrictHeating;
        let dhw = f((Level::Hn4, 9), dh, Purpose::Dhw, &[("S#31", 1.0)]);
        assert!(validate(&g, &dhw).is_ok());
        g.formulas.push(dhw);
        let sh = f((Level::Hn4, 9), dh, Purpose::SpaceHeating,
                   &[("S#30", 1.0), ("S#31", -1.0)]);
        assert!(validate(&g, &sh).is_ok(), "S#31 nets to 0, S#30 to 1");
    }

    /// A series mixing derived and metered contributions would make Unallocated
    /// ambiguous.
    #[test]
    fn validate_rejects_mixed_derived_ness_for_one_series() {
        let (mut g, _) = chiller();
        g.sensors.push(sensor(9, &format!("{CO}|HN5#5"), EnergyType::Gas));
        let mixed = f((Level::Hn2, 997), EnergyType::Electricity, Purpose::Cooling,
                      &[("S#9", 3.0)]);   // gas sensor, electricity output → derived
        assert!(validate(&g, &mixed).unwrap_err().contains("derived"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model validate_ 2>&1 | tail -20`
Expected: FAIL — `cannot find function validate`.

- [ ] **Step 3: Implement**

Append to the non-test part of `formulas.rs`:

```rust
/// Validate a formula against its company graph (spec §5.3). Returns a
/// human-readable message suitable for a 400 body.
pub fn validate(g: &CompanyGraph, f: &NodeFormula) -> Result<(), String> {
    if !f.purpose.declarable() {
        return Err(format!(
            "purpose {} is derived by the roll-up job and cannot be declared", f.purpose));
    }
    let Some(node_path) = g.node_path(&f.node) else {
        return Err(format!("node {} not found in this company", f.node));
    };
    let own: Vec<SensorId> = g.own_sensors(&f.node).iter().map(|s| s.id).collect();
    let children: Vec<NodeId> = g.children(&f.node).map(|n| n.id.clone()).collect();
    let sep = crate::domain::node::PATH_SEP;

    for t in &f.terms {
        if !t.coefficient.is_finite() {
            return Err(format!("coefficient for {} must be finite", t.reference));
        }
        match &t.reference {
            Reference::Node(id) => {
                if !children.contains(id) {
                    return Err(format!(
                        "{id} is not a direct child of {} — a deeper node is already counted \
                         through the child chain; override the child instead", f.node));
                }
            }
            Reference::Sensor(id) => {
                let Some(s) = g.sensor(*id) else {
                    return Err(format!("sensor {id} not found in this company"));
                };
                let deeper = !own.contains(id)
                    && s.path.starts_with(&format!("{node_path}{sep}"));
                if f.purpose == Purpose::Total && deeper {
                    return Err(format!(
                        "sensor {id} is already counted through {}'s children — override the \
                         child node instead", f.node));
                }
            }
        }
    }

    // Derived-ness must be uniform per (energy_type, purpose) in the company.
    if f.purpose != Purpose::Total {
        let mine = is_derived(g, f);
        if let Some(other) = g.formulas.iter().find(|o| {
            o.energy_type == f.energy_type && o.purpose == f.purpose
                && o.node != f.node
                && is_derived(g, o) != mine
        }) {
            return Err(format!(
                "{}/{} is already {} on node {} — a series cannot mix derived and metered \
                 contributions", f.energy_type, f.purpose,
                if mine { "metered" } else { "derived" }, other.node));
        }
    }

    // A sensor cannot be allocated more than it measured: the signed sum of its
    // coefficients across all ALLOCATING claims for one energy type is ≤ 1. This
    // permits a heat-pump split (0.7 + 0.3), the bimåler pattern (+1, −1) and
    // shared plant across siblings (0.6 + 0.4).
    if f.purpose != Purpose::Total && !f.purpose.is_outflow() && !is_derived(g, f) {
        for t in &f.terms {
            let Reference::Sensor(id) = &t.reference else { continue };
            let others: f64 = g.formulas.iter()
                .filter(|o| o.energy_type == f.energy_type
                         && o.purpose != Purpose::Total
                         && !o.purpose.is_outflow()
                         && !is_derived(g, o)
                         && !(o.node == f.node && o.purpose == f.purpose))
                .flat_map(|o| &o.terms)
                .filter(|ot| ot.reference == t.reference)
                .map(|ot| ot.coefficient)
                .sum();
            let total = others + t.coefficient;
            if total > 1.0 + f64::EPSILON {
                return Err(format!(
                    "sensor {id} would be allocated {total:.2}× its {} reading — coefficients \
                     across all purposes must sum to at most 1", f.energy_type));
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
git commit -m "feat(model): validate node formulas (direct children, <=1 allocation, uniform derivedness)"
```

---

### Task 5: Persist formulas and the coefficient matrix

**Files:**
- Create: `crates/model/src/repository/dynamodb/node_formula.rs`, `.../weight.rs`
- Modify: `.../dynamodb/mod.rs`, `.../dynamodb/codec.rs`, `crates/model/src/repository/memory.rs`

**Interfaces:**
- Produces:
  - `codec::{node_formula_to_item, node_formula_of_item, matrix_row_to_item, formula_gsi1pk, weight_gsi1pk}`
  - `node_formula::{put_node_formula, delete_node_formula, list_node_formulas, list_company_formulas}`
  - `weight::replace_company_matrix(client, table, company_path, &[MatrixRow])`

Item shapes are spec §4.1 / §4.2. Matrix sort key:
`weight#<node_path>#<energy_type>#<purpose>#<sensor>`, `gsi1pk = W#HN2#<id>`, attributes
`node_path`, `energy_type`, `purpose`, `sensor_id`, `coefficient`, `allocates`.

- [ ] **Step 1: Write the failing codec tests**

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
        let item = node_formula_to_item(&f, "HN0#root|HN2#997|HN4#30", "HN0#root|HN2#997");
        let s = |k: &str| item.get(k).and_then(|v| v.as_s().ok()).map(String::as_str);
        assert_eq!(s("sk"), Some("formula#district_heating#space_heating"));
        assert_eq!(s("gsi1pk"), Some("F#HN2#997"));
        assert_eq!(node_formula_of_item(&item).unwrap(), f);
    }

    /// A Total formula persists like any other — it is not a special case.
    #[test]
    fn total_formula_item_round_trips() {
        use crate::domain::node_formula::{NodeFormula, Reference, Term};
        use crate::domain::values::Purpose;
        let f = NodeFormula {
            node: NodeId::make(Level::Hn5, 5),
            energy_type: EnergyType::Electricity,
            purpose: Purpose::Total,
            terms: vec![Term { reference: Reference::Sensor(SensorId::make(2)),
                               coefficient: 0.0 }],
            note: None,
        };
        let item = node_formula_to_item(&f, "HN0#root|HN2#997|HN5#5", "HN0#root|HN2#997");
        assert_eq!(item.get("sk").and_then(|v| v.as_s().ok()).map(String::as_str),
                   Some("formula#electricity#total"));
        assert_eq!(node_formula_of_item(&item).unwrap(), f);
    }

    #[test]
    fn matrix_row_keys_by_company_node_and_sensor() {
        use crate::domain::values::Purpose;
        use crate::logic::formulas::MatrixRow;
        let r = MatrixRow {
            node_path: "HN0#root|HN2#997|HN4#30".to_string(),
            energy_type: EnergyType::DistrictHeating,
            purpose: Purpose::Dhw,
            sensor: SensorId::make(21),
            coefficient: 1.0,
            allocates: true,
        };
        let item = matrix_row_to_item(&r, "HN0#root|HN2#997");
        let s = |k: &str| item.get(k).and_then(|v| v.as_s().ok()).map(String::as_str);
        assert_eq!(s("pk"), Some("HN2#997"));
        assert_eq!(s("gsi1pk"), Some("W#HN2#997"));
        assert_eq!(s("sk"),
                   Some("weight#HN0#root|HN2#997|HN4#30#district_heating#dhw#S#21"));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p model _item 2>&1 | tail -20`
Expected: FAIL — `cannot find function node_formula_to_item`.

- [ ] **Step 3: Implement the codec and adapters**

Follow the existing helpers in `codec.rs` (`s(..)`, `Item`, `CodecError`).
`node_formula_to_item` serialises `terms` as `L` of `M{ ref: S, coefficient: N }`;
`matrix_row_to_item` writes the attributes above with `allocates` as `Bool`.

`node_formula.rs`: `put_node_formula` (PutItem), `delete_node_formula` (DeleteItem on
`pk = <NodeId>`, `sk = formula#<et>#<purpose>`), `list_node_formulas` (`pk = <NodeId>` +
`begins_with(sk, "formula#")`), `list_company_formulas` (`gsi1` on `F#HN2#<id>`) — in the
paginator style already used in `sensor.rs`.

`weight.rs`: `replace_company_matrix` queries `gsi1` on `W#HN2#<id>` for existing keys, then
`batch_write_item` in chunks of 25 — deletes first, then puts. The matrix is derived data, so
wholesale replacement is simpler and safer than diffing.

Mirror all of it in `repository/memory.rs`.

- [ ] **Step 4: Verify and commit**

Run: `cargo test -p model 2>&1 | tail -20` → PASS.

```bash
git add crates/model/src/repository/
git commit -m "feat(model): persist node formulas and the materialised coefficient matrix"
```

---

### Task 6: Commands and matrix recompute

**Files:**
- Modify: `crates/services/hierarchy/src/command.rs`, `dispatch.rs`, `repo_fns.rs`, `json.rs`

**Interfaces:**
- Produces: `Command::{SetNodeFormula, DeleteNodeFormula, RebuildCompanyMatrix}`;
  `dispatch::{handle_set_node_formula, handle_delete_node_formula, handle_rebuild_company_matrix, recompute_matrix}`

Wire format — `terms` accepted as a JSON array or a JSON-encoded string (the HTML form builds it client-side to avoid dynamic field names):

```
action=set_node_formula
node_id=HN5#5
energy_type=electricity
purpose=total
terms=[{"ref":"S#2","coefficient":0},{"ref":"S#3","coefficient":0}]
note=faserne er allerede med i akkumulatoren
```

- [ ] **Step 1: Write the failing parse tests**

```rust
    #[test]
    fn parse_set_node_formula_json() {
        let body = serde_json::json!({
            "action": "set_node_formula", "node_id": "HN5#5",
            "energy_type": "electricity", "purpose": "total",
            "terms": [{"ref": "S#2", "coefficient": 0}],
            "note": "faserne er allerede med i akkumulatoren"
        }).to_string();
        let cmd = parse_command(&body, Some("application/json")).unwrap();
        assert!(matches!(&cmd, Command::SetNodeFormula { purpose, .. } if purpose == "total"));
    }

    #[test]
    fn parse_set_node_formula_form_with_encoded_terms() {
        let form = "action=set_node_formula&node_id=HN4%231&energy_type=electricity\
                    &purpose=total\
                    &terms=%5B%7B%22ref%22%3A%22S%2311%22%2C%22coefficient%22%3A-1%7D%5D";
        let cmd = parse_command(form, Some("application/x-www-form-urlencoded")).unwrap();
        assert!(matches!(&cmd, Command::SetNodeFormula { terms, .. }
                         if terms.is_string() || terms.is_array()));
    }

    #[test]
    fn parse_delete_and_rebuild() {
        let d = parse_command("action=delete_node_formula&node_id=HN4%2330\
                               &energy_type=district_heating&purpose=dhw",
                              Some("application/x-www-form-urlencoded")).unwrap();
        assert!(matches!(&d, Command::DeleteNodeFormula { purpose, .. } if purpose == "dhw"));
        let r = parse_command("action=rebuild_company_matrix&company=HN2%23997",
                              Some("application/x-www-form-urlencoded")).unwrap();
        assert!(matches!(&r, Command::RebuildCompanyMatrix { company } if company == "HN2#997"));
    }
```

- [ ] **Step 2: Run to verify they fail, then add the variants**

Run: `cargo test -p hierarchy node_formula 2>&1 | tail -20` → FAIL.

```rust
    /// `set_node_formula` — upsert a node's `(energy_type, purpose)` formula.
    /// `purpose = total` declares the node's own value; anything else is a claim.
    SetNodeFormula {
        node_id: String,
        energy_type: String,
        purpose: String,
        terms: Value,
        #[serde(default)] note: Option<String>,
    },
    /// `delete_node_formula` — remove one, reverting that pair to the default.
    DeleteNodeFormula { node_id: String, energy_type: String, purpose: String },
    /// `rebuild_company_matrix` — recompute the materialised matrix from scratch.
    RebuildCompanyMatrix { company: String },
```

Run again: PASS.

- [ ] **Step 3: Write the failing handler tests**

```rust
    #[tokio::test]
    async fn set_node_formula_stores_and_rebuilds_the_matrix() {
        let store = memory_store_with_company();
        assert!(store.weight_rows().is_empty());
        let out = set_formula(&store, "HN5#5", "electricity", "total",
                              serde_json::json!([{"ref": "S#2", "coefficient": 0}])).await;
        assert_eq!(out["ok"], serde_json::json!(true));
        assert!(!store.weight_rows().is_empty(), "the matrix is what Glue reads");
    }

    /// Attaching a sensor changes the matrix, so it must recompute too —
    /// recompute-on-formula-write alone leaves it stale.
    #[tokio::test]
    async fn attach_sensor_rebuilds_the_matrix() {
        let store = memory_store_with_company();
        let before = store.weight_rows().len();
        attach_sensor_at(&store, "HN5#5", "S#77", "electricity").await;
        assert!(store.weight_rows().len() > before);
    }

    #[tokio::test]
    async fn set_node_formula_rejects_a_non_child_node_reference() {
        let store = memory_store_with_company();
        let out = set_formula(&store, "HN2#997", "electricity", "total",
                              serde_json::json!([{"ref": "HN5#5", "coefficient": 0.5}])).await;
        assert_eq!(out["error"]["code"], serde_json::json!("Validation"));
    }

    #[tokio::test]
    async fn set_node_formula_rejects_malformed_terms() {
        let store = memory_store_with_company();
        let out = set_formula(&store, "HN5#5", "electricity", "total",
                              serde_json::json!("not-json")).await;
        assert_eq!(out["error"]["code"], serde_json::json!("Bad_request"));
    }
```

Add `memory_store_with_company`, `set_formula` and `attach_sensor_at` alongside the existing
in-memory helpers; the fixture needs `HN2#997` → `HN4#4` → `HN5#5` with a sensor on `HN5#5`.

- [ ] **Step 4: Implement the handlers**

```rust
/// Parse `terms`: a JSON array, or a JSON-encoded string of one.
fn parse_terms(v: &Value) -> Result<Vec<Term>, String> {
    let arr = match v {
        Value::Array(a) => a.clone(),
        Value::String(s) => serde_json::from_str::<Vec<Value>>(s)
            .map_err(|e| format!("terms is not a JSON array: {e}"))?,
        _ => return Err("terms must be a JSON array".to_string()),
    };
    arr.iter().map(|t| {
        let r = t.get("ref").and_then(Value::as_str).ok_or("term missing \"ref\"")?;
        let c = t.get("coefficient").and_then(Value::as_f64)
            .ok_or("term missing numeric \"coefficient\"")?;
        Ok(Term { reference: Reference::parse(r)?, coefficient: c })
    }).collect()
}

/// Load the company graph, flatten it, replace its materialised matrix. Called
/// by every command that can change it.
pub async fn recompute_matrix</* closure generics as in repo_fns */>(
    company_path: String,
    list_company_formulas: FLF,
    list_company_sensors: FLS,
    list_company_nodes: FLN,
    replace_matrix: FRM,
) -> Result<(), RepositoryError> {
    let (formulas, sensors, nodes) = futures::try_join!(
        list_company_formulas(company_path.clone()),
        list_company_sensors(company_path.clone()),
        list_company_nodes(company_path.clone()))?;
    let matrix = formulas_logic::flatten(&CompanyGraph { nodes, sensors, formulas });
    replace_matrix(company_path, matrix).await
}
```

`handle_set_node_formula` parses and validates `(node_id, energy_type, purpose, terms)`,
resolves the company path from the node, loads the graph, calls `formulas_logic::validate`,
writes the formula item, then calls `recompute_matrix`. `handle_delete_node_formula` deletes
then recomputes. `handle_rebuild_company_matrix` recomputes alone.

Add the three `run` arms, gate all three behind the same `writes` edge as `AttachSensor`, and
call `recompute_matrix` at the end of the existing `AttachSensor`, `ReplaceSensorDevice`,
`DeleteSensor`, `AddNode` and `DeleteNode` arms. `UpdateNode` must **not** recompute.

Add closure factories to `repo_fns.rs` following the existing pattern: `put_node_formula_fn`,
`delete_node_formula_fn`, `list_node_formulas_fn`, `list_company_formulas_fn`,
`list_company_sensors_fn`, `list_company_nodes_fn`, `replace_matrix_fn`. Add
`formula_to_json` to `json.rs`.

- [ ] **Step 5: Verify and commit**

Run: `cargo test -p hierarchy 2>&1 | tail -20` → PASS.

```bash
git add crates/services/hierarchy/src/
git commit -m "feat(hierarchy): formula commands + materialised matrix recompute"
```

---

### Task 7: Formler tab; remove the sensor-formula UI

**Files:**
- Modify: `crates/services/hierarchy/src/query.rs`, `html/node.rs`, `html/forms.rs`
- Test: `crates/services/hierarchy/tests/node_forms_html.rs`

**Interfaces:**
- Produces: `GET /hierarchy/query/node_formulas?node=<NodeId>`;
  `html::node::render_node_formulas(&Node, &[NodeFormula], &[Sensor], &[Node]) -> Markup`

- [ ] **Step 1: Write the failing tests**

```rust
/// One card per formula, headed by (energy_type, purpose).
#[test]
fn node_formulas_render_one_card_per_formula() {
    let html = render_node_formulas_fixture();
    assert!(html.contains("district_heating") && html.contains("space_heating"));
    assert!(html.contains("bimåler"));
    assert!(html.contains("-1"));
}

/// A pair with no formula shows the default, read-only — "nothing declared" must
/// look different from "declared as Σ".
#[test]
fn node_formulas_show_the_default_when_none_is_declared() {
    let html = render_node_formulas_fixture();
    assert!(html.contains("Σ"));
}

/// The picker offers EVERY sensor in the company (D7) and names the node each one
/// hangs off, so a sideways reference is an informed choice.
#[test]
fn reference_picker_offers_company_wide_sensors_with_their_node() {
    let html = render_node_formulas_fixture();
    assert!(html.contains("S#1"), "own sensor");
    assert!(html.contains("S#77"), "sensor in another branch is offered");
    assert!(html.contains("Building C2"), "and says where it lives");
}

/// Node references are direct children only.
#[test]
fn reference_picker_offers_only_direct_children_as_nodes() {
    let html = render_node_formulas_fixture();
    assert!(html.contains("HN5#5"), "direct child");
    assert!(!html.contains("HN6#6"), "grandchild must not be offered");
}

/// The old per-sensor formula dialog is gone, and the sensor form asks for
/// nothing but what the sensor measures.
#[test]
fn add_sensor_form_has_no_classification_controls() {
    let html = render_add_sensor_form_fixture();
    assert!(!html.contains("formula-dialog"));
    assert!(!html.contains("data.formula.kind"));
    assert!(html.contains("name=\"energy_type\""));
    assert!(html.contains("name=\"reading_kind\""));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p hierarchy --test node_forms_html 2>&1 | tail -20` → FAIL.

- [ ] **Step 3: Delete the old UI**

Remove the `<dialog id="formula-dialog">` block from `html/node.rs` — kind select, expression
input, alias→sensor rows, the three hidden `data.formula.*` inputs and their inline script —
and the company-sensor `<option>` fragment endpoint. In `html/forms.rs`, drop the Formula row
and rename the `purpose` select to `energy_type`, `meter_type` to `reading_kind`.

Run: `cargo test -p hierarchy add_sensor_form_has_no_classification_controls` → PASS.

- [ ] **Step 4: Implement the Formler tab**

```rust
/// The node panel's **Formler** tab. One card per declared formula, an empty card
/// for adding one, and — for every `(energy_type, purpose)` with no formula — a
/// read-only line showing the default, so "nothing declared" is visibly different
/// from "declared as Σ".
pub fn render_node_formulas(
    node: &Node,
    formulas: &[NodeFormula],
    company_sensors: &[Sensor],   // D7: every sensor in the company
    child_nodes: &[Node],         // D7: direct children only
) -> Markup {
    html! {
        div class="formulas" {
            @for f in formulas {
                section class="formula-card" {
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
                                (reference_select(&t.reference, company_sensors, child_nodes))
                                input type="number" step="any" class="coef"
                                      value=(t.coefficient);
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
                        button type="submit" { "Slet — brug standarden" }
                    }
                }
            }
            (default_lines(node, formulas))
            (new_formula_card(node, company_sensors, child_nodes))
        }
    }
}

/// Every reference the node may legally use: its direct children, and every
/// sensor in the company labelled with the node it hangs off — so picking one
/// from another branch is an informed choice, not an accident.
fn reference_select(selected: &Reference, sensors: &[Sensor], children: &[Node]) -> Markup {
    html! {
        select class="ref" {
            @for n in children {
                option value=(n.id) selected[*selected == Reference::Node(n.id.clone())] {
                    (n.name) " (" (n.id) ")"
                }
            }
            @for s in sensors {
                option value=(s.id) selected[*selected == Reference::Sensor(s.id)] {
                    (s.daq_id) " (" (s.energy_type) ") — " (sensor_node_name(s))
                }
            }
        }
    }
}
```

Add `terms_json`, `new_formula_card` (selects from `EnergyType::all()` and
`Purpose::all().filter(|p| p.declarable())`), `default_lines`, `sensor_node_name`, and a small
inline script keeping the hidden `terms` input in sync with the rows on submit.

Wire the query action next to `"sensors"` in `query.rs`; `handle_node_formulas` loads the
node, the company's sensors and the node's direct children, then renders.

- [ ] **Step 5: Add the tab, verify, commit**

Add a **Formler** tab to the node panel's tab strip loading via
`hx-get="/hierarchy/query/node_formulas?node=<id>"`.

Run: `cargo test -p hierarchy 2>&1 | tail -5 && cargo clippy --all-targets 2>&1 | grep -c warning`
Expected: PASS, `0`.

```bash
git add crates/services/hierarchy/
git commit -m "feat(hierarchy): Formler tab; drop the per-sensor formula dialog"
```

---

### Task 8: Deploy Phase 1 and seed the matrices

- [ ] **Step 1: Build and diff**

```bash
cargo lambda build --release --arm64 -p hierarchy
cd infra/hierarchy && unset GOROOT && \
export AWS_PROFILE=stel-sb && \
eval "$(aws configure export-credentials --profile stel-sb --format env)" && \
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1 && \
cdk diff OcamlHierarchyStack
```

Expected: only Lambda `Code` `[~]`. **Stop and report** if the DynamoDB table shows any change.

- [ ] **Step 2: Deploy and verify**

```bash
cdk deploy OcamlHierarchyStack --require-approval never
aws lambda get-function-configuration --profile stel-sb \
  --function-name rust-lambda-hierarchy --query '{State:State,Last:LastUpdateStatus}'
```

Expected: `Active` / `Successful`.

- [ ] **Step 3: Seed every company's matrix**

Existing companies have no weight rows, so nothing would roll up. For each HN2 node, POST
`action=rebuild_company_matrix&company=<id>` to the CloudFront `/command` endpoint, then:

```bash
aws dynamodb query --profile stel-sb --table-name hierarchy_new --index-name gsi1 \
  --key-condition-expression 'gsi1pk = :p' \
  --expression-attribute-values '{":p":{"S":"W#HN2#997"}}' \
  --query 'length(Items)'
```

Expected: non-zero — a company with sensors has `total` rows even with no formulas declared.

- [ ] **Step 4: Commit**

```bash
git commit --allow-empty -m "chore(hierarchy): deploy node-formula authoring (phase 1)"
```

---

# Phase 2 — Pipeline

**Tasks 9–11 must land together** — the contract has no fallback, and Task 9 leaves `sensor-identity` empty until it is repopulated.

---

### Task 9: `sensor-identity`, bridge, cross-account reader role

**Files:**
- Modify (daq account `891377204778`): `data_pipeline_stack.go` (table, IAM, Flink property),
  `main.go` (stack props), `ocaml_bridge_stack.go` (writer role), `late_recomputation_stack.go`
  (props, IAM, backfill ESM, lambda env, Glue args), `measurements_aggregate_stack.go` (pin the
  Glue role name), `lambda/backfill_trigger/handler.py`, `lambda/late_arrival_trigger/handler.py`,
  `glue/late_recomputation.py`, `scripts/backup_restore_ddb.py`
- Modify (hierarchy account `339712745226`): `infra/hierarchy/app.go` — bridge item builder, plus a
  new `HierarchyReaderRole`

> **Revised 2026-07-28 after verifying against the live system.** The original task named five
> files; the real footprint is ten, and three of its assumptions were wrong. Corrections:
>
> - **The bridge is already half-done.** Commit `22d2144` made it read `energy_type`/`reading_kind`
>   and stop copying `formula`, because the Phase 1 migration broke it (`KeyError: 'meter_type'`,
>   4 batches to the DLQ). It currently *translates* to the pipeline's `purpose`/`meter_type`.
>   Step 5 removes the translation.
> - **Flink has no event source mapping.** It reads the Kinesis stream
>   `flink-iceberg-processor-ddb-changes` (attached to the table via `KinesisStreamSpecification`,
>   `data_pipeline_stack.go:60-63`) and learns the table name from the app property
>   `METER_IDENTITY_TABLE`. The rename is a property change plus a restart — not an ESM repoint.
> - **`backfill-trigger` *does* have an ESM** on the `meter-identity` DynamoDB stream, defined in
>   `late_recomputation_stack.go:232-235` — a stack the original task never mentioned. Replacing
>   the table replaces that stream, so the ESM must be replaced with it.
> - **The table is `RemovalPolicy: RETAIN`.** CFN will create `sensor-identity` and *orphan*
>   `meter-identity` rather than delete it. The orphan keeps PITR billing and a live Kinesis
>   streaming destination into the same change stream. Delete it by hand in Step 7.
> - **The Glue role ARN has a CFN-generated random suffix**
>   (`MeasurementsAggregateStack-AggGlueJobRole76EE978D-LYfqTL5960Ta`). Trusting that ARN directly
>   means any future role replacement silently breaks the cross-account read, so Step 1 pins it to
>   a stable name first.

The cross-account role has a strict ordering: the DAQ-side role must exist under its stable name
before the hierarchy-side trust policy can name it, and the `sts:AssumeRole` grant comes last.
Steps 1-3 are that dance; do not reorder them.

- [ ] **Step 1: Pin the Glue role name (daq account)**

In `measurements_aggregate_stack.go`, find the `AggGlueJobRole` and give it an explicit name:

```go
	RoleName: jsii.String("MeasurementsAggregateGlueRole"),
```

```bash
cd infra/daq/data_pipeline
unset GOROOT
export AWS_PROFILE=daq_dev
eval "$(aws configure export-credentials --profile daq_dev --format env)"
npx cdk diff MeasurementsAggregateStack -c TableBucketName=measurements -c LookbackDays=1
```

Expect the IAM role `[-]`/`[+]` (replacement — a role rename always replaces) and the Glue job
`[~]` picking up the new ARN. **Stop and report** if `measurements_aggregate` (DynamoDB) shows
any change. Deploy, then confirm the stable ARN:

```bash
aws iam get-role --profile daq_dev --role-name MeasurementsAggregateGlueRole \
  --query 'Role.Arn' --output text
# arn:aws:iam::891377204778:role/MeasurementsAggregateGlueRole
```

- [ ] **Step 2: Add the reader role (hierarchy account)**

In `infra/hierarchy/app.go`, beside the `hierarchy_new` table:

```go
	// The DAQ account's Glue roll-up assumes this to read the materialised
	// coefficient matrix (spec §4.2, §8). Read-only, hierarchy_new + gsi1.
	readerRole := awsiam.NewRole(stack, jsii.String("HierarchyReaderRole"), &awsiam.RoleProps{
		RoleName: jsii.String("HierarchyReaderRole"),
		AssumedBy: awsiam.NewArnPrincipal(
			jsii.String("arn:aws:iam::891377204778:role/MeasurementsAggregateGlueRole")),
	})
	readerRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Actions:   jsii.Strings("dynamodb:Query"),
		Resources: jsii.Strings(*table.TableArn(), *table.TableArn()+"/index/gsi1"),
	}))
```

```bash
cd infra/hierarchy
unset GOROOT
export AWS_PROFILE=stel-sb
eval "$(aws configure export-credentials --profile stel-sb --format env)"
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1
cdk diff OcamlHierarchyStack
```

Expect exactly one new IAM role and its policy. **Stop and report** if `hierarchy_new` shows any
change. Deploy.

- [ ] **Step 3: Grant the Glue role `sts:AssumeRole` (daq account)**

In `measurements_aggregate_stack.go`, after the role:

```go
	glueRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Actions:   jsii.Strings("sts:AssumeRole"),
		Resources: jsii.Strings("arn:aws:iam::339712745226:role/HierarchyReaderRole"),
	}))
```

Deploy `MeasurementsAggregateStack`, then verify both sides statically:

```bash
aws iam get-role --profile stel-sb --role-name HierarchyReaderRole \
  --query 'Role.AssumeRolePolicyDocument.Statement[].Principal.AWS' --output text
# arn:aws:iam::891377204778:role/MeasurementsAggregateGlueRole
aws iam get-role-policy --profile daq_dev --role-name MeasurementsAggregateGlueRole \
  --policy-name "$(aws iam list-role-policies --profile daq_dev \
    --role-name MeasurementsAggregateGlueRole --query 'PolicyNames[0]' --output text)" \
  --query 'PolicyDocument.Statement[?Action==`sts:AssumeRole`].Resource' --output json
# includes arn:aws:iam::339712745226:role/HierarchyReaderRole
```

**There is deliberately no live probe here.** `HierarchyReaderRole` trusts *only*
`MeasurementsAggregateGlueRole`, which in turn is only assumable by `glue.amazonaws.com` — so
no human credential can exercise this path, and any command that appears to would mean the trust
is too wide. The end-to-end proof is the first `measurements-aggregate` run in Task 11 Step 5,
which assumes the role for real; treat an `AccessDenied` there as this step's failure, not that
one's.

- [ ] **Step 4: Back up the table before replacing it**

```bash
cd infra/daq/data_pipeline/scripts
python backup_restore_ddb.py backup --profile daq_dev
```

The bridge repopulates from `hierarchy_new` stream events, so this backup is insurance, not the
migration path. Keep the file until Step 7 verifies the row count.

- [ ] **Step 5: Rename the table and every reference to it**

`data_pipeline_stack.go`: `TableName` → `sensor-identity`, local `meterIdentity` →
`sensorIdentity`, construct id `MeterIdentity` → `SensorIdentity`, the props field
`MeterIdentityTable` → `SensorIdentityTable`, and the Flink property key `METER_IDENTITY_TABLE`
→ `SENSOR_IDENTITY_TABLE`.

`main.go`: `MeterIdentityArn`/`MeterIdentityName`/`MeterIdentityStreamArn` →
`SensorIdentity*`; `MeterIdentityTableArn` → `SensorIdentityTableArn`.

`ocaml_bridge_stack.go`: the props field and the role description.

`late_recomputation_stack.go`: the three props fields, the `dynamodb:Scan`/`Query` grant, the
Glue argument `--meter_identity_table` → `--sensor_identity_table`, the two lambda env vars
`METER_IDENTITY_TABLE` → `SENSOR_IDENTITY_TABLE`, and the `MeterIdentityRef` table reference
that backs `backfill-trigger`'s ESM.

`lambda/backfill_trigger/handler.py`, `lambda/late_arrival_trigger/handler.py`: the env var and
the Glue argument name.

`glue/late_recomputation.py`: `REQUIRED_ARGS` and `args["meter_identity_table"]` →
`sensor_identity_table`; `load_meter_identity` → `load_sensor_identity`.

`scripts/backup_restore_ddb.py`: `TABLE_NAME`, `BACKUP_FILE`, the docstring and the argparse help.

In `infra/hierarchy/app.go`, drop the translation added in `22d2144` — emit the hierarchy's own
vocabulary now that the pipeline speaks it:

```python
    out = {
        "pk": {"S": _pk(daq)},
        "sk": {"S": daq},
        "logical_id": {"N": str(sid)},
        "reading_kind": {"S": img["reading_kind"]["S"]},
        "hierarchy_path": {"S": _path(img["gsi1sk"]["S"])},
        "energy_type": {"S": img["energy_type"]["S"]},
    }
```

Also rename the function `ocaml-meter-identity-bridge` → `ocaml-sensor-identity-bridge`, its
DLQ, and the `…-dlq-not-empty` alarm. Note this replaces the log group, so the DLQ evidence from
the Phase 1 breakage is lost — check it is empty first:

```bash
aws sqs get-queue-attributes --profile stel-sb \
  --queue-url https://sqs.eu-central-1.amazonaws.com/339712745226/ocaml-meter-identity-bridge-dlq \
  --attribute-names ApproximateNumberOfMessages --query 'Attributes' --output json
```

- [ ] **Step 6: Break the cross-stack export deadlock (do this BEFORE Step 7)**

Renaming the table drops three exports — `…MeterIdentity…Arn`, `…Ref…`, `…StreamArn` — that
`LateRecomputationStack` and `OcamlBridgeWriterRoleStack` still import. CloudFormation refuses:

```
Delete canceled. Cannot delete export DaqPipelineStack:ExportsOutputFnGetAttMeterIdentity…Arn…
as it is in use by LateRecomputationStack and OcamlBridgeWriterRoleStack.
```

The consumers cannot move first either — the `SensorIdentity` exports do not exist until
`DaqPipelineStack` deploys. Deploying the producer alone rolls back to `UPDATE_ROLLBACK_COMPLETE`
(clean, and safe while Flink is stopped — but it is a wasted cycle).

Break it with a transitional deploy. In `data_pipeline_stack.go`, keep the old table resource and
re-declare the legacy export **names** explicitly with unchanged values:

```go
	legacyMeterIdentity := awsdynamodb.NewTable(stack, jsii.String("MeterIdentity"), /* …unchanged props… */)
	awscdk.NewCfnOutput(stack, jsii.String("LegacyMeterIdentityArn"), &awscdk.CfnOutputProps{
		Value:      legacyMeterIdentity.TableArn(),
		ExportName: jsii.String("DaqPipelineStack:ExportsOutputFnGetAttMeterIdentity2F42C403ArnF8DDA39D"),
	})
	// …same for …ExportsOutputRefMeterIdentity2F42C403A37A0AA6 (TableName)
	// and  …ExportsOutputFnGetAttMeterIdentity2F42C403StreamArnD3C8C1B9 (TableStreamArn)
```

Exports are keyed by **name**, so a name present in both the old and new template is neither
deleted nor updated, and the imports keep resolving while the new exports appear alongside.
Take the three export names from the failed diff — they embed CFN logical-id hashes and cannot
be guessed.

Then three deploys, in this order:

1. `cdk deploy DaqPipelineStack` — creates `sensor-identity`, keeps `meter-identity` and its exports.
2. `cdk deploy S3TablesStack` then `cdk deploy LateRecomputationStack OcamlBridgeWriterRoleStack`
   — consumers move onto the `SensorIdentity` exports; the legacy ones fall out of use.
   (`S3TablesStack` first: `LateRecomputationStack`'s Lake Formation grant names `logical_data`.)
3. Delete the transitional block and `cdk deploy DaqPipelineStack` again — legacy exports are
   dropped and `meter-identity` reports `DELETE_SKIPPED` (orphaned by `RETAIN`, as intended).

`LateRecomputationStack` will log `DELETE_FAILED` on `GlueLfLogicalMeterPermissions` — it is
revoking a Lake Formation grant on `logical_meter_data`, which no longer exists. CloudFormation
retries three times over ~6 minutes and then finishes `UPDATE_COMPLETE` with "One or more
resources could not be deleted." That is expected; the replacement grant on `logical_data` is
created regardless.

- [ ] **Step 7: Stop Flink, deploy both accounts, restart**

The Flink app holds `meter-identity` mappings in keyed state and reads the change stream. Stop it
so it cannot write against a half-renamed world:

```bash
aws kinesisanalyticsv2 stop-application --profile daq_dev \
  --application-name flink-iceberg-processor
```

(Graceful stop — no `--force`. The operator `uid`s are unchanged, so it snapshots cleanly.)

```bash
cd infra/daq/data_pipeline
unset GOROOT && export AWS_PROFILE=daq_dev
eval "$(aws configure export-credentials --profile daq_dev --format env)"
npx cdk diff DaqPipelineStack LateRecomputationStack OcamlBridgeWriterRoleStack \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
```

Expect: `sensor-identity` `[+]`, `meter-identity` `[-]` (retained, not deleted), the
`backfill-trigger` ESM replaced, IAM `[~]`, the Flink app `[~]` for the property change.
**Stop and report** if `logical_meter_data`, `raw_data` or `measurements_aggregate` appear at
all — they are not in scope for this task. Deploy the same three stacks, then deploy
`OcamlHierarchyStack` from the hierarchy account (Step 2's recipe) for the bridge.

Restart Flink. The uid and keyed-state descriptors are unchanged, so the snapshot restores:

```bash
aws kinesisanalyticsv2 start-application --profile daq_dev \
  --application-name flink-iceberg-processor \
  --run-configuration '{"ApplicationRestoreConfiguration":{"ApplicationRestoreType":"RESTORE_FROM_LATEST_SNAPSHOT"}}'
```

- [ ] **Step 8: Repopulate, verify, delete the orphan**

The new table is empty. Re-save every sensor through the UI (or touch each row — a same-value
`UpdateItem` writes nothing and emits **no** stream record, so change a field and change it back):

```bash
aws dynamodb scan --profile daq_dev --table-name sensor-identity \
  --query 'Items[].{daq:sk.S,et:energy_type.S,rk:reading_kind.S,f:formula.S}' --output table
```

Expect one row per active sensor (**7** today), `energy_type`/`reading_kind` populated, `formula`
absent. Cross-check the count against `hierarchy_new`'s active sensors — a short count means
Flink silently drops that sensor's readings:

```bash
aws dynamodb query --profile stel-sb --table-name hierarchy_new --index-name gsi1 \
  --key-condition-expression 'gsi1pk = :p' \
  --expression-attribute-values '{":p":{"S":"S#HN2#10003"}}' \
  --query 'length(Items[?type.S==`sensor`])'
```

Only once the counts match, delete the orphaned table (it still streams into
`flink-iceberg-processor-ddb-changes`):

```bash
aws dynamodb delete-table --profile daq_dev --table-name meter-identity
```

- [ ] **Step 9: Commit**

```bash
git add infra/
git commit -m "feat(bridge)!: sensor-identity table; energy_type/reading_kind end to end; reader role"
```

---

### Task 10: Iceberg + Flink rename

**Files:**
- Rename: `.../enrichment/MeterMapping.scala` → `SensorMapping.scala`
- Modify (Scala main): `DdbBootstrapLoader.scala`, `DdbStreamDeserializer.scala`,
  `MeterEnrichmentFunction.scala`, `ResampleFunction.scala`, `HierarchyPathParser.scala`,
  `flink/Main.scala`
- Modify (Scala tests): `DdbBootstrapLoaderSpec`, `DdbStreamDeserializerSpec`,
  `MeterEnrichmentFunctionSpec`, `ResampleFunctionSpec`, `ResampleHarnessSpec`,
  `EnrichmentPipelineSpec`, `EnrichmentMiniClusterSpec`, `ParserMiniClusterSpec`,
  `ScenarioTestHelper`, `ScenarioTestHelperSpec`, `TestEnrichmentMapper`
- Modify (infra): `s3tables_stack.go`, `late_recomputation_stack.go` (Lake Formation grant),
  `measurements_aggregate_stack.go:140` (Lake Formation grant)
- Modify (jobs/scripts): `glue/late_recomputation.py`, `flink_app_scala/scripts/smoke_test/test_smoke.py`,
  `scripts/verify_ingestion.py`, `docs/generate_diagrams.py`

> **Revised 2026-07-28 after verifying against the live system.** The original task would have
> destroyed 311 million rows unrecoverably. Corrections:
>
> - **`raw_data` is not touched.** It has no `purpose` and no `meter_type` column — `purpose`
>   appears exactly once in `s3tables_stack.go` (line 73, inside `logical_meter_data`) and
>   `meter_type` is not an Iceberg column anywhere. The original "rename in both table
>   definitions" had no target in `raw_data`, and clearing it would have destroyed
>   **311,220,885 rows across 71,472 daqs** going back to 2026-05-02.
> - **"~24 h is replayable from Kinesis" is wrong twice over.** `logical_meter_data` holds
>   **25,970 rows spanning 2026-05-01 → now** — about three months — while `DAQ_INPUT_STREAM`
>   retention is 24 h. A Kinesis replay would also duplicate `raw_data` (the gotcha already in
>   CLAUDE.md). The correct rebuild is `late-data-recomputation` over the mapped daqs, which
>   derives logical rows from `raw_data` — and `raw_data` covers the same window.
> - **No two-step delete/recreate is needed.** CLAUDE.md's recipe exists because
>   `AWS::S3Tables::Table` cannot be replaced under an *unchanged* name (create-before-delete
>   → `409 table with an identical name already exists`). We are renaming, so there is no
>   collision: one deploy creates `logical_data` and orphans `logical_meter_data`.
> - **That recipe is stale anyway.** Both tables gained `RemovalPolicy: RETAIN` with
>   `ApplyToUpdateReplacePolicy` on 2026-07-18 (`2d21c6c`), six weeks after the recipe was
>   learned on 2026-06-07. Removing the resource no longer deletes the table, so the documented
>   two-step would silently leave the data in place. Step 6 fixes CLAUDE.md.

- [ ] **Step 1: Rename in Scala**

`MeterMapping` → `SensorMapping`; `.purpose` → `.energyType`; `.meterType` → `.readingKind`.
In `Main.scala`, the property key `METER_IDENTITY_TABLE` → `SENSOR_IDENTITY_TABLE` (matching
Task 9 Step 5), the table schema's `purpose` column → `energy_type`, and the sink target
`all.logical_meter_data` → `all.logical_data`.

Run: `cd infra/daq/data_pipeline/flink_app_scala && sbt test 2>&1 | tail -20` → PASS.

**A green suite does not mean this step is right.** Scala field names and DynamoDB/Iceberg
*wire* names are different things, and a bulk rename conflates them. Doing `purpose` →
`energyType` across the tree rewrote the DynamoDB attribute literal `"purpose"` into
`"energyType"` and the Iceberg sink column to `"energyType"`, while `"meter_type"` was never
renamed at all — and all 118 tests still passed, because the fixtures were rewritten in
lockstep with the code. Only a cross-artifact check catches it.

The wire names now live once, in `enrichment/LogicalDataSchema.scala`, and
`LogicalDataSchemaSpec` reads `s3tables_stack.go` to assert the sink's column list matches the
table's. After renaming, confirm the guard can still fail — revert the column in the Go file,
run `sbt "testOnly *LogicalDataSchemaSpec"`, expect 2 failures, restore. A guard that cannot
fail is worth nothing.

Column names on the wire are snake_case (`reading_kind`, `energy_type`); the Scala fields are
camelCase (`readingKind`, `energyType`). Any camelCase string reaching DynamoDB or Iceberg is
this bug.

- [ ] **Step 2: Rename the Iceberg table and its one column**

In `s3tables_stack.go`, in the `logical_meter_data` definition only:

```go
		TableName:       jsii.String("logical_data"),
...
					field("energy_type", "string", false),
```

Leave `raw_data` alone. The partition spec and sort order reference fields by **index**
(`partition(7, ...)`, `sortField(7)`), and renaming a field in place does not shift indices, so
they need no change — but re-read them after editing to confirm the column order is unchanged.

Rename the Lake Formation grants that name the table, in `late_recomputation_stack.go:125` and
`measurements_aggregate_stack.go:140`:

```go
				DatabaseName: jsii.String("all"), Name: jsii.String("logical_data"),
```

- [ ] **Step 3: Rename the columns in the Glue jobs**

`glue/late_recomputation.py` writes this table and reads the identity columns. Rename
`meter_type` → `reading_kind` and `purpose` → `energy_type` in `_identity_record`, the
`StructType` schema, the two `F.col("meter_type") == …` filters, the final `F.select`, and point
`write_df.writeTo(...)` at `all.logical_data`. (`measurements_aggregate.py` is rewritten
wholesale in Task 11 — leave it.)

Also update the read-only helpers so they do not silently return nothing:
`flink_app_scala/scripts/smoke_test/test_smoke.py`, `scripts/verify_ingestion.py`,
`docs/generate_diagrams.py`.

- [ ] **Step 4: Deploy the table (destructive — confirm first)**

**This orphans `logical_meter_data` (25,970 rows) and creates an empty `logical_data`.** The data
is rebuildable from `raw_data` in Step 5; nothing else is at risk. Stop Flink first so it is not
writing to a table that is about to disappear from under it:

```bash
aws kinesisanalyticsv2 stop-application --profile daq_dev \
  --application-name flink-iceberg-processor
```

```bash
cd infra/daq/data_pipeline
unset GOROOT && export AWS_PROFILE=daq_dev
eval "$(aws configure export-credentials --profile daq_dev --format env)"
npx cdk diff S3TablesStack -c TableBucketName=measurements
```

Expect exactly one `[+]` (`logical_data`) and one `[-]` (`logical_meter_data`, retained).
**Stop and report** if `raw_data` appears in the diff at all. Deploy `S3TablesStack`, then
`LateRecomputationStack` and `DaqPipelineStack` (Task 9 Step 6's recipe, with a fresh
`RUN_NR`) after `sbt clean assembly`.

Restart Flink from its snapshot — the operator `uid`s and keyed-state descriptors are unchanged:

```bash
cd flink_app_scala && sbt clean assembly && cd ..
npx cdk deploy DaqPipelineStack --require-approval never \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
aws kinesisanalyticsv2 start-application --profile daq_dev \
  --application-name flink-iceberg-processor \
  --run-configuration '{"ApplicationRestoreConfiguration":{"ApplicationRestoreType":"RESTORE_FROM_LATEST_SNAPSHOT"}}'
```

- [ ] **Step 5: Rebuild the history from `raw_data`**

Flink only fills `logical_data` going forward. Rebuild the three months from `raw_data` with a
targeted recomputation over the mapped daqs — cheap, because only the 7 sensors in
`sensor-identity` have logical rows at all:

```bash
DAQS=$(aws dynamodb scan --profile daq_dev --table-name sensor-identity \
  --query 'Items[].sk.S' --output text | tr '\t' ',')
aws glue start-job-run --profile daq_dev --job-name late-data-recomputation \
  --arguments "{\"--daq_ids\":\"$DAQS\",\"--lookback_days\":\"90\"}"
```

Verify against the pre-migration numbers (25,970 rows, 2026-05-01 → now):

```bash
aws athena start-query-execution --profile daq_dev --work-group daq-workgroup \
  --query-execution-context 'Catalog=s3tablescatalog/measurements,Database=all' \
  --query-string 'SELECT energy_type, count(*) AS n, min(resample_timestamp) AS oldest,
                         max(resample_timestamp) AS newest
                  FROM "all".logical_data GROUP BY energy_type'
```

Expect rows grouped by `energy_type` (`electricity`, `district_heating`, `water`) and a row count
in the same order of magnitude. An exact match is not expected — recomputation re-derives deltas
and the window boundary differs.

- [ ] **Step 6: Fix the stale CLAUDE.md recipe**

CLAUDE.md's "two-step deploy" for `AWS::S3Tables::Table` predates the `RETAIN` policy added in
`2d21c6c` (2026-07-18) and no longer works: removing the resource retains the table instead of
deleting it, so the second deploy still hits `409 … identical name already exists`. Record that
clearing a table under an unchanged name now needs the removal policy relaxed first, and that a
**rename** avoids the whole problem. Update the table/column names in the same pass.

- [ ] **Step 7: Commit**

```bash
git add infra/daq/data_pipeline/ CLAUDE.md
git commit -m "feat(pipeline)!: sensor vocabulary in Flink and Iceberg (energy_type, logical_data)"
```

---

### Task 11: Roll-up job — one join, one grouped sum

**Files:**
- Create: `infra/daq/data_pipeline/glue/hierarchy_matrix.py`
- Modify: `glue/measurements_aggregate.py`, `glue/tests/test_rollups.py`, `measurements_aggregate_stack.go`

**`ancestor_keys` is deleted.** Ancestry is baked into the matrix. There is no recursion, no default handling, no derived detection and no `Unallocated` subtraction in PySpark — `Unallocated` arrives as ordinary matrix rows.

> **Revised 2026-07-28.** Three pieces of this task moved into the revised Tasks 9 and 10, so do
> not repeat them here: the `HierarchyReaderRole` trust policy and the Glue role's
> `sts:AssumeRole` grant are Task 9 Steps 2-3 (and the role now has the stable ARN
> `arn:aws:iam::891377204778:role/MeasurementsAggregateGlueRole`), and the Lake Formation grant
> naming `logical_meter_data` in `measurements_aggregate_stack.go:140` is renamed in Task 10
> Step 2. What remains here is the job itself, its tests, the matrix reader, and the two job
> arguments.

- [ ] **Step 1: Write the failing tests**

Rewrite `_input` so its column is `energy_type` with the token `"electricity"`, then:

```python
def _matrix():
    """Two nodes. HN2#2 sums both sensors; HN3#9 has only 10009. Lighting claims
    10009 and rolls up. Unallocated arrives pre-computed by crates/model."""
    def row(node, pur, sid, c):
        return {"node_path": node, "energy_type": "electricity", "purpose": pur,
                "sensor_id": sid, "coefficient": c}
    return [
        row("HN2#2", "total", 10009, 1.0), row("HN2#2", "total", 10010, 1.0),
        row("HN2#2|HN3#9", "total", 10009, 1.0),
        row("HN2#2", "lighting", 10009, 1.0),
        row("HN2#2|HN3#9", "lighting", 10009, 1.0),
        row("HN2#2", "unallocated", 10010, 1.0),
    ]


def test_sort_key_carries_the_purpose_segment(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    assert "HN2#2#electricity#total#h#2026-06-07T08" in out
    assert "HN2#2|HN3#9#electricity#lighting#h#2026-06-07T08" in out


def test_value_is_the_weighted_sum_of_the_matrix(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    # 10009 contributes 4+6 = 10, 10010 contributes 5.
    assert out["HN2#2#electricity#total#h#2026-06-07T08"]["sum"] == 15.0
    assert out["HN2#2|HN3#9#electricity#total#h#2026-06-07T08"]["sum"] == 10.0


def test_unallocated_needs_no_arithmetic_in_the_job(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2#electricity#unallocated#h#2026-06-07T08"]["sum"] == 5.0


def test_a_negative_coefficient_subtracts(spark):
    matrix = [r for r in _matrix() if r["purpose"] == "total"]
    matrix.append({"node_path": "HN2#2|HN3#9", "energy_type": "electricity",
                   "purpose": "total", "sensor_id": 10010, "coefficient": -1.0})
    out = _by_sk(m.build_rollups(_input(spark), matrix, run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2|HN3#9#electricity#total#h#2026-06-07T08"]["sum"] == 5.0


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


def test_the_job_holds_no_formula_logic(spark):
    """Guard against semantics creeping back into PySpark."""
    src = open(os.path.join(os.path.dirname(__file__), "..",
                            "measurements_aggregate.py")).read()
    for forbidden in ("ancestor_keys", "derived", "allocates", "generation", "def coeffs"):
        assert forbidden not in src, forbidden
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd infra/daq/data_pipeline/glue && python -m pytest tests/ -q 2>&1 | tail -15` → FAIL.

- [ ] **Step 3: Write the matrix reader**

```python
"""Read a company's materialised coefficient matrix from hierarchy_new.

Holds NO domain rules. The recursion, the defaults, derived detection and the
Unallocated arithmetic all live in crates/model/src/logic/formulas.rs and are
materialised by the hierarchy service. This module is a GSI query.
"""


def reader_table(role_arn, region, table_name="hierarchy_new"):
    import boto3
    c = boto3.client("sts").assume_role(
        RoleArn=role_arn, RoleSessionName="measurements-aggregate")["Credentials"]
    return boto3.resource(
        "dynamodb", region_name=region,
        aws_access_key_id=c["AccessKeyId"],
        aws_secret_access_key=c["SecretAccessKey"],
        aws_session_token=c["SessionToken"]).Table(table_name)


def load_matrix(table, company_id):
    from boto3.dynamodb.conditions import Key
    rows, kwargs = [], {"IndexName": "gsi1",
                        "KeyConditionExpression": Key("gsi1pk").eq("W#HN2#%d" % int(company_id))}
    while True:
        page = table.query(**kwargs)
        rows.extend({"node_path": r["node_path"],
                     "energy_type": r["energy_type"],
                     "purpose": r["purpose"],
                     "sensor_id": int(r["sensor_id"]),
                     "coefficient": float(r["coefficient"])}
                    for r in page.get("Items", []))
        if "LastEvaluatedKey" not in page:
            return rows
        kwargs["ExclusiveStartKey"] = page["LastEvaluatedKey"]
```

The reader deliberately drops `allocates` — the job never needs it, because `Unallocated` is
already a set of matrix rows.

- [ ] **Step 4: Rewrite the job**

```python
def build_sk(node_path, energy_type, purpose, gran, bucket):
    """The bucket stays LAST so a fixed (energy_type, purpose) is a pure BETWEEN
    key-range. '#' after node_path keeps a node's own rows sorting before its
    descendants' ('|' > '#')."""
    return "%s#%s#%s#%s#%s" % (node_path, energy_type, purpose, gran, bucket)


def build_gsi1pk(hn2, dimension, purpose):
    """Per-purpose, so a cross-type 'all energy' query cannot sum total together
    with its own purpose breakdown."""
    return "HN2#%d#%s#%s" % (hn2, dimension, purpose)


def build_rollups(df, matrix, run_at_iso):
    """value(node, energy_type, purpose) = Σ coefficient × reading.

    The matrix already encodes ancestry, the defaults and Unallocated, so this is
    one join and one grouped sum. See spec §8.2.
    """
    spark = df.sparkSession
    with_buckets = df.withColumn("gb", F.explode(F.array(
        F.struct(F.lit("h").alias("gran"),
                 F.date_format(F.col("resample_timestamp"), "yyyy-MM-dd'T'HH").alias("bucket")),
        F.struct(F.lit("d").alias("gran"),
                 F.date_format(F.col("resample_timestamp"), "yyyy-MM-dd").alias("bucket")),
    ))).select("*", F.col("gb.gran").alias("gran"), F.col("gb.bucket").alias("bucket"))

    schema = T.StructType([
        T.StructField("node_path", T.StringType()),
        T.StructField("m_energy_type", T.StringType()),
        T.StructField("purpose", T.StringType()),
        T.StructField("m_logical_id", T.IntegerType()),
        T.StructField("coefficient", T.DoubleType()),
    ])
    m_df = spark.createDataFrame(
        [(r["node_path"], r["energy_type"], r["purpose"],
          int(r["sensor_id"]), float(r["coefficient"])) for r in matrix], schema)

    grouped = (with_buckets
        .join(F.broadcast(m_df),
              (with_buckets.logical_id == m_df.m_logical_id)
              & (with_buckets.energy_type == m_df.m_energy_type), "inner")
        .withColumn("contrib", F.col("resample_value") * F.col("coefficient"))
        .groupBy("hn2", "node_path", "m_energy_type", "purpose", "gran", "bucket")
        .agg(F.sum("contrib").alias("sum"),
             F.count("contrib").alias("count"),
             F.max("unit").alias("unit"))
        .withColumnRenamed("m_energy_type", "energy_type"))

    return grouped.select(
        F.concat(F.lit("HN2#"), F.col("hn2").cast("string")).alias("pk"),
        _SK_UDF("node_path", "energy_type", "purpose", "gran", "bucket").alias("sk"),
        _GSI1PK_UDF("hn2", _DIM_UDF("unit"), "purpose").alias("gsi1pk"),
        _GSI1SK_UDF("node_path", "gran", "bucket").alias("gsi1sk"),
        "energy_type", "purpose", "unit", "sum", "count",
        F.lit(run_at_iso).alias("updated_at"),
        _TTL_UDF("gran", "bucket").alias("ttl"))
```

Delete `ancestor_keys` and `_ancestor_keys_udf`. Rename `purpose` → `energy_type` in
`read_counters` and point it at `all.logical_data`. In `main`, load and merge the matrix per
distinct `hn2`, and add `hierarchy_reader_role_arn` to the required args.

In `measurements_aggregate_stack.go`: pass that argument and add `hierarchy_matrix.py` via
`--extra-py-files`. The `sts:AssumeRole` grant is already in place from Task 9 Step 3 — verify
rather than re-add it:

```bash
aws iam get-role-policy --profile daq_dev --role-name MeasurementsAggregateGlueRole \
  --policy-name $(aws iam list-role-policies --profile daq_dev \
    --role-name MeasurementsAggregateGlueRole --query 'PolicyNames[0]' --output text) \
  --query 'PolicyDocument.Statement[?Action==`sts:AssumeRole`]'
```

- [ ] **Step 5: Verify, deploy, rebuild the view**

Run: `cd infra/daq/data_pipeline/glue && python -m pytest tests/ -q 2>&1 | tail -15` → PASS.

`cdk diff MeasurementsAggregateStack`: expect Glue script asset `[~]`, IAM `[~]`, a new job
argument. **Stop and report** if the DynamoDB table shows replacement. Deploy, delete the old
rollup rows (the sort key gained a segment, so they are unreadable), then:

```bash
aws glue start-job-run --profile daq_dev --job-name measurements-aggregate \
  --arguments '{"--lookback_days":"30"}'
```

- [ ] **Step 6: Update the docs the renames invalidated**

Task 10 Step 6 already corrected CLAUDE.md's table/column names and its stale `AWS::S3Tables::Table`
recipe. What is left here is the roll-up description in the `MeasurementsAggregateStack` section
(it documents the per-node/purpose/hour|day rollup and the `ancestor_keys` behaviour this task
deletes), plus the session memories that still say "SPECCED NOT DONE"
(`sensor-not-meter-vocabulary.md`, `node-formula-rollup-design.md`), and renaming
`meter-identity-change-auto-triggers-late-recompute.md` with its `[[…]]` backlinks.

- [ ] **Step 7: Commit**

```bash
git add infra/daq/data_pipeline/ CLAUDE.md
git commit -m "feat(glue): roll-up is one join and one grouped sum over the coefficient matrix"
```

---

# Phase 3 — Read side + frontend

---

### Task 12: Aggregations reads the purpose axis

**Files:**
- Modify: `crates/services/aggregations/src/main.rs` — `parse_sk`, `Row` (145), `to_rows` (157), `to_rows_dimension` (187), `QueryParams` (236), `query_node` (253), `query_one_resource` (268), `query_dimension` (301), `handle_aggregations` (441), `fetch_node_rows` (550), the factor tables (609/623), `scale_rows` (654)

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn parse_sk_reads_the_purpose_segment() {
        let (path, et, purpose, gran, bucket) =
            parse_sk("HN2#2|HN3#9#electricity#lighting#h#2026-06-07T08");
        assert_eq!((path, et, purpose, gran, bucket),
                   ("HN2#2|HN3#9", "electricity", "lighting", "h", "2026-06-07T08"));
    }

    #[test]
    fn query_prefix_defaults_to_total() {
        assert_eq!(sk_prefix("HN2#2", "electricity", "", Gran::Hour),
                   "HN2#2#electricity#total#h#");
        assert_eq!(sk_prefix("HN2#2", "electricity", "lighting", Gran::Hour),
                   "HN2#2#electricity#lighting#h#");
    }

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

- [ ] **Step 2: Run to verify they fail, then implement**

`parse_sk` returns a 5-tuple. Extract:

```rust
/// `<node_path>#<energy_type>#<purpose>#<gran>#` — the caller appends the bucket
/// to form a pure `BETWEEN` range. Empty `purpose` means `total`.
fn sk_prefix(sk_path: &str, energy_type: &str, purpose: &str, gran: Gran) -> String {
    let purpose = if purpose.is_empty() { "total" } else { purpose };
    format!("{sk_path}#{energy_type}#{purpose}#{}#", gran.code())
}
```

Add `purpose` to `QueryParams`, thread it from `handle_aggregations` (default `""`) and
`fetch_node_rows` (always `"total"`); rename `QueryParams.resource` → `energy_type` and
`query_one_resource` → `query_one_energy_type`. `query_dimension`'s `gsi1pk` gains the
purpose. `Row` gains `energy_type`. Both factor tables take `(energy_type, purpose, unit)` and
return 0 for `generation`; `scale_rows`'s closure becomes `Fn(&str, &str, &str) -> f64`.
`handle_alarms` and `handle_benchmark` pass `purpose = "total"`. Update the utoipa params.

- [ ] **Step 3: Verify and commit**

Run: `cargo test -p aggregations 2>&1 | tail -20` → PASS.

```bash
git add crates/services/aggregations/
git commit -m "feat(aggregations): read the purpose axis; (energy_type, purpose) tariff and CO2 factors"
```

---

### Task 13: `get_purpose_split`

**Files:**
- Modify: `crates/services/aggregations/src/main.rs` (routes 387-403, new handler, `openapi_response`)

- [ ] **Step 1: Write the failing test**

```rust
    /// The physical purposes plus unallocated add up to total.
    #[test]
    fn purpose_split_rows_are_grouped_by_purpose() {
        let items = vec![
            agg_item("HN2#2#electricity#total#d#2026-06-07", 100.0),
            agg_item("HN2#2#electricity#lighting#d#2026-06-07", 22.0),
            agg_item("HN2#2#electricity#cooling#d#2026-06-07", 18.0),
            agg_item("HN2#2#electricity#unallocated#d#2026-06-07", 60.0),
        ];
        let by: std::collections::BTreeMap<_, _> =
            to_rows(items, "HN2#2", "daily", Gran::Day)
                .iter().map(|r| (r.purpose.clone(), r.value)).collect();
        assert_eq!(by["total"], 100.0);
        assert_eq!(by["lighting"] + by["cooling"] + by["unallocated"], by["total"]);
    }
```

- [ ] **Step 2: Add the route and handler**

`GET /meterdata/query/get_purpose_split?level_id=&energy_type=&start=&end=&resolution=&format=`
fans out over `Purpose::all()` concurrently — the same shape as today's fan-out over the six
energy types — and returns one `Row` per purpose. Add the utoipa path and register it in
`openapi_response()`.

- [ ] **Step 3: Verify, deploy, commit**

Run: `cargo test -p aggregations 2>&1 | tail -20` → PASS.

```bash
cargo lambda build --release --arm64 -p aggregations
# cdk diff + deploy MeasurementsAggregateStack, then:
curl -s "$API/meterdata/query/get_purpose_split?level_id=HN2%23997&energy_type=electricity&start=2026-07-01T00:00:00Z&end=2026-07-28T00:00:00Z"
```

Expected: rows including `total` and `unallocated`.

```bash
git add crates/services/aggregations/ infra/daq/data_pipeline/
git commit -m "feat(aggregations): get_purpose_split end-use breakdown"
```

---

### Task 14: End-use breakdown widget

**Files:**
- Create: `frontend/src/components/PurposeSplit.astro`
- Modify: the node Data-tab dashboard (`grep -rn "get_aggregations" frontend/src`)

- [ ] **Step 1: Build it**

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
    <thead><tr><th>Formål</th><th>Tid</th><th>Værdi</th><th>Enhed</th><th>Punkter</th></tr></thead>
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

Grid, not flex. Mount it beside the existing aggregation widgets and confirm it re-fires on
soft navigation (`htmx.process` on `astro:page-load` if it sits outside `#node-data-panel`).

- [ ] **Step 2: Build, deploy, verify, commit**

```bash
cd frontend && npm run build
cd ../infra/frontend && unset GOROOT && \
export AWS_PROFILE=stel-sb && \
eval "$(aws configure export-credentials --profile stel-sb --format env)" && \
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1 && \
cdk deploy OcamlFrontendStack --require-approval never
```

Open the live site, go to a company node's Data tab, confirm the table renders with `total`
and `unallocated`.

```bash
git add frontend/
git commit -m "feat(frontend): end-use breakdown widget on the node Data tab"
```

---

### Task 15: Acceptance against the presentation

`docs/hierarchy-presentation.html` is a runnable model of the arithmetic. Reproducing its numbers through the real stack is the acceptance test for the whole plan.

- [ ] **Step 1: Build the fixture company through the UI**

| Node | Formula |
|---|---|
| Chiller | `electricity/total` = cL1×0, cL2×0, cL3×0 |
| Chiller | `electricity/cooling` = ACC×1 |
| Chiller | `district_cooling/cooling` = ACC×3.2 |
| Area A1b | `electricity/total` = EXP×−1 |
| Area A1b | `electricity/generation` = EXP×1 |
| Building A1 | `district_heating/total` = DHW×0 |
| Building A1 | `district_heating/dhw` = DHW×1 |
| Building A1 | `district_heating/space_heating` = HM1×1 + DHW×−1 |
| Building A2 | `district_heating/dhw` = HM2×0.28; `…/space_heating` = HM2×0.72 |
| Building B1 | `electricity/lighting` = LGT; `…/ventilation` = AHU |
| Building B1 | `electricity/space_heating` = HP×0.7; `…/dhw` = HP×0.3; `heat/space_heating` = HP×3.5 |
| Building B2 | `electricity/plug_loads` = KIT; `heat/space_heating` = GAS×10.45 |
| **Building C1** | **`electricity/total` = SUB×−1** — SUB lives in Building C2 |

- [ ] **Step 2: Run the roll-up**

```bash
aws glue start-job-run --profile daq_dev --job-name measurements-aggregate \
  --arguments '{"--lookback_days":"2"}'
```

- [ ] **Step 3: Assert six invariants via `get_purpose_split`**

1. **Recursion** — Chiller `total` = 40 (not 80); Area A1a = 124. The correction needs no restating upward.
2. **Sideways reference** — C1 = 70, C2 = 30, Property C = 100. The main is counted exactly once.
3. **Direction** — Area A1b `total` = import + production − export = 120; `generation` = 30 separately; `get_emissions` on `total` charges nothing for the export.
4. **Exact partition** — A1 `district_heating`: `space_heating + dhw == total`, `unallocated == 0`. Same for A2's 0.28/0.72.
5. **One sensor, many purposes** — B1 `electricity`: lighting + ventilation + space_heating + dhw == total, `unallocated == 0`.
6. **Derived floats free** — `district_cooling/cooling` and both `heat/space_heating` sources are non-zero while their `total` and `unallocated` are 0.

- [ ] **Step 4: Record and commit**

Append an "Acceptance" section to the spec with the measured values, so it records what was verified rather than what was intended.

```bash
git add docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md
git commit -m "docs: record node-formula roll-up acceptance results"
```

---

## Self-Review

**Spec coverage.** D1–D9 → Tasks 1–4, 9, 11. §3.1 deletions → Task 2. §3.2 renames → Tasks 1, 9, 10, 12. §3.3 bare sensor → Tasks 2, 7. §3.4 types → Tasks 1, 2. §3.5 the two defaults → Task 3. §3.6 sideways references → Tasks 3, 4, 7, 15. §3.7 flatten → Task 3. §3.8 derived/outflow → Tasks 3, 4. §3.9 termination → Task 4 (direct-child rule). §4.1/§4.2 storage → Task 5. §5 service + UI → Tasks 6, 7. §6 bridge + `sensor-identity` → Task 9. §7 Flink/Iceberg → Task 10. §8 roll-up → Task 11. §9 read side → Tasks 12, 13. §11 testing → distributed. §12 deploy order → Tasks 8–11, 13, 14.

**Gap found and closed:** the spec says `Unallocated` arrives as ordinary matrix rows, which only holds if `flatten` emits them. Task 3 tests it explicitly (`unallocated_is_total_minus_allocating`), and Task 11 adds a guard test asserting no formula vocabulary appears in the PySpark source at all, so the semantics cannot creep back.

**Gap found and closed:** recompute-on-formula-write alone leaves the matrix stale whenever a *sensor* changes. Task 6 names every triggering command and tests `attach_sensor` specifically.

**Gap found and closed:** existing companies have no weight rows after Phase 1, so nothing would roll up at all. Task 8 Step 3 seeds them and asserts a non-zero count.

**Type consistency.** `Reference`/`Term`/`NodeFormula` (Task 2) are used unchanged in Tasks 3–7. `CompanyGraph`/`MatrixRow`/`coeffs`/`flatten`/`is_derived` (Task 3) keep their signatures in Tasks 4, 5, 6. The PySpark side (Task 11) reads `node_path, energy_type, purpose, sensor_id, coefficient` — exactly the attribute names Task 5's `matrix_row_to_item` writes. `build_sk` takes five arguments at every call site. The factor functions gain `purpose` in Task 12 and `scale_rows` is updated there; `query_one_energy_type` is renamed in Task 12 and called by that name in Task 13.
