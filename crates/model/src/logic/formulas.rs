//! Recursive evaluation of node formulas, and its flattening into a coefficient
//! matrix.
//!
//! A node's value is Σ of its CHILDREN'S VALUES plus its own sensors, unless the
//! node's formula overrides some of those weights. Because every term is linear,
//! the whole recursion collapses to `value = Σ_sensor coefficient × reading`,
//! which is what the roll-up job consumes — so these rules live here and only here.

use std::collections::BTreeMap;

use crate::domain::ids::{NodeId, SensorId};
use crate::domain::node::{is_at_or_under, Node};
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

/// A sparse coefficient vector over sensors.
pub type Coeffs = BTreeMap<SensorId, f64>;

impl CompanyGraph {
    pub fn node_path(&self, id: &NodeId) -> Option<&str> {
        self.nodes.iter().find(|n| &n.id == id).map(|n| n.path.as_str())
    }

    pub fn children<'a>(&'a self, id: &'a NodeId) -> impl Iterator<Item = &'a Node> + 'a {
        self.nodes.iter().filter(move |n| n.parent.as_ref() == Some(id))
    }

    /// Sensors attached directly to this node (not to a descendant).
    pub fn own_sensors(&self, id: &NodeId) -> Vec<&Sensor> {
        let Some(path) = self.node_path(id) else {
            return vec![];
        };
        self.sensors.iter().filter(|s| parent_path(s) == path).collect()
    }

    pub fn sensor(&self, id: SensorId) -> Option<&Sensor> {
        self.sensors.iter().find(|s| s.id == id)
    }

    fn formula(&self, node: &NodeId, et: EnergyType, p: Purpose) -> Option<&NodeFormula> {
        self.formulas
            .iter()
            .find(|f| &f.node == node && f.energy_type == et && f.purpose == p)
    }
}

fn add(into: &mut Coeffs, sensor: SensorId, c: f64) {
    if c == 0.0 {
        return;
    }
    let e = into.entry(sensor).or_insert(0.0);
    *e += c;
    if *e == 0.0 {
        into.remove(&sensor);
    }
}

/// The coefficient vector of `value(node, et, purpose)` over sensors.
///
/// Two defaults, and the asymmetry is deliberate: a sensor is part of what the
/// node **consumed** automatically, but part of a **purpose** only when a
/// formula says so — otherwise attaching a meter would silently claim it as
/// lighting.
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
    let weight_of = |r: &Reference| {
        f.and_then(|f| {
            f.terms
                .iter()
                .find(|t| &t.reference == r)
                .map(|t| t.coefficient)
        })
    };

    let mut out = Coeffs::new();

    // Children default to 1, for Total and for a named purpose alike.
    for child in g.children(node) {
        let w = weight_of(&Reference::Node(child.id.clone())).unwrap_or(1.0);
        if w == 0.0 {
            continue;
        }
        for (s, c) in coeffs(g, &child.id, et, purpose) {
            add(&mut out, s, w * c);
        }
    }

    // Own sensors default to 1 for Total ONLY.
    if purpose == Purpose::Total {
        for s in g.own_sensors(node).into_iter().filter(|s| s.energy_type == et) {
            add(&mut out, s.id, weight_of(&Reference::Sensor(s.id)).unwrap_or(1.0));
        }
    }

    // Named sensors. For Total the node's own ones were just handled with their
    // override applied; what remains are sensors elsewhere in the company — the
    // main-in-one-building / sub-in-another case. For a purpose, every named
    // sensor counts.
    if let Some(f) = f {
        let own: Vec<SensorId> = g.own_sensors(node).iter().map(|s| s.id).collect();
        for t in &f.terms {
            if let Reference::Sensor(id) = t.reference {
                if purpose == Purpose::Total && own.contains(&id) {
                    continue;
                }
                add(&mut out, id, t.coefficient);
            }
        }
    }

    out
}

/// A formula is derived when its output energy type differs from that of a
/// sensor it names directly. Node references resolve to the formula's own energy
/// type and never make it derived.
pub fn is_derived(g: &CompanyGraph, f: &NodeFormula) -> bool {
    f.terms.iter().any(|t| match &t.reference {
        Reference::Sensor(id) => g.sensor(*id).is_some_and(|s| s.energy_type != f.energy_type),
        Reference::Node(_) => false,
    })
}

/// Purposes declared for `et` that reduce `Unallocated`: neither derived nor
/// outflow.
fn allocating_purposes(g: &CompanyGraph, et: EnergyType) -> Vec<Purpose> {
    let mut ps: Vec<Purpose> = g
        .formulas
        .iter()
        .filter(|f| {
            f.energy_type == et
                && f.purpose != Purpose::Total
                && !f.purpose.is_outflow()
                && !is_derived(g, f)
        })
        .map(|f| f.purpose)
        .collect();
    ps.sort_by_key(|p| p.as_str());
    ps.dedup();
    ps
}

/// Every series the roll-up needs — `Total`, each declared purpose, and
/// `Unallocated` — for every node and energy type in the company.
///
/// `Unallocated` rows are pure arithmetic on the rows already computed, so the
/// roll-up job never has to subtract anything itself.
pub fn flatten(g: &CompanyGraph) -> Vec<MatrixRow> {
    let mut out = Vec::new();
    for n in &g.nodes {
        for et in EnergyType::all() {
            let mut purposes = vec![Purpose::Total, Purpose::Unallocated];
            purposes.extend(
                g.formulas
                    .iter()
                    .filter(|f| f.energy_type == et && f.purpose != Purpose::Total)
                    .map(|f| f.purpose),
            );
            purposes.sort_by_key(|p| p.as_str());
            purposes.dedup();

            for purpose in purposes {
                let allocates = purpose != Purpose::Unallocated
                    && !purpose.is_outflow()
                    && !g.formulas.iter().any(|f| {
                        f.energy_type == et && f.purpose == purpose && is_derived(g, f)
                    });
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

/// Validate a formula against its company graph. Returns a human-readable
/// message suitable for a 400 body.
pub fn validate(g: &CompanyGraph, f: &NodeFormula) -> Result<(), String> {
    if !f.purpose.declarable() {
        return Err(format!(
            "purpose {} is derived by the roll-up job and cannot be declared",
            f.purpose
        ));
    }
    let Some(node_path) = g.node_path(&f.node) else {
        return Err(format!("node {} not found in this company", f.node));
    };
    let own: Vec<SensorId> = g.own_sensors(&f.node).iter().map(|s| s.id).collect();
    let children: Vec<NodeId> = g.children(&f.node).map(|n| n.id.clone()).collect();

    for t in &f.terms {
        if !t.coefficient.is_finite() {
            return Err(format!("coefficient for {} must be finite", t.reference));
        }
        match &t.reference {
            Reference::Node(id) => {
                if !children.contains(id) {
                    return Err(format!(
                        "{id} is not a direct child of {} — a deeper node is already counted \
                         through the child chain; override the child instead",
                        f.node
                    ));
                }
            }
            Reference::Sensor(id) => {
                let Some(s) = g.sensor(*id) else {
                    return Err(format!("sensor {id} not found in this company"));
                };
                // For Total, a sensor deeper in this node's own subtree already
                // arrives via the child chain.
                // A sensor's path always ends in `|S#<n>`, so it is never equal
                // to a node path — `is_at_or_under` is exactly "in my subtree".
                let deeper = !own.contains(id) && is_at_or_under(&s.path, node_path);
                if f.purpose == Purpose::Total && deeper {
                    return Err(format!(
                        "sensor {id} is already counted through {}'s children — override the \
                         child node instead",
                        f.node
                    ));
                }
            }
        }
    }

    // Derived-ness must be uniform per (energy_type, purpose) in the company: a
    // series mixing the two would make Unallocated ambiguous.
    if f.purpose != Purpose::Total {
        let mine = is_derived(g, f);
        if let Some(other) = g.formulas.iter().find(|o| {
            o.energy_type == f.energy_type
                && o.purpose == f.purpose
                && o.node != f.node
                && is_derived(g, o) != mine
        }) {
            return Err(format!(
                "{}/{} is already {} on node {} — a series cannot mix derived and metered \
                 contributions",
                f.energy_type,
                f.purpose,
                if mine { "metered" } else { "derived" },
                other.node
            ));
        }
    }

    // A sensor cannot be allocated more than it measured: the signed sum of its
    // coefficients across all ALLOCATING claims for one energy type is ≤ 1. This
    // permits a heat-pump split (0.7 + 0.3), the bimåler pattern (+1, −1) and
    // shared plant across siblings (0.6 + 0.4).
    if f.purpose != Purpose::Total && !f.purpose.is_outflow() && !is_derived(g, f) {
        for t in &f.terms {
            let Reference::Sensor(id) = &t.reference else {
                continue;
            };
            let others: f64 = g
                .formulas
                .iter()
                .filter(|o| {
                    o.energy_type == f.energy_type
                        && o.purpose != Purpose::Total
                        && !o.purpose.is_outflow()
                        && !is_derived(g, o)
                        && !(o.node == f.node && o.purpose == f.purpose)
                })
                .flat_map(|o| &o.terms)
                .filter(|ot| ot.reference == t.reference)
                .map(|ot| ot.coefficient)
                .sum();
            let total = others + t.coefficient;
            if total > 1.0 + f64::EPSILON {
                return Err(format!(
                    "sensor {id} would be allocated {total:.2}× its {} reading — coefficients \
                     across all purposes must sum to at most 1",
                    f.energy_type
                ));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::Level;
    use crate::domain::node_formula::Term;
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

    fn f(
        node: (Level, u32),
        et: EnergyType,
        pur: Purpose,
        terms: &[(&str, f64)],
    ) -> NodeFormula {
        NodeFormula {
            node: NodeId::make(node.0, node.1),
            energy_type: et,
            purpose: pur,
            terms: terms
                .iter()
                .map(|(r, c)| Term {
                    reference: Reference::parse(r).unwrap(),
                    coefficient: *c,
                })
                .collect(),
            note: None,
        }
    }

    /// Evaluate the flattened coefficients against a reading set.
    fn value(
        g: &CompanyGraph,
        n: (Level, u32),
        et: EnergyType,
        pur: Purpose,
        readings: &[(u32, f64)],
    ) -> f64 {
        coeffs(g, &NodeId::make(n.0, n.1), et, pur)
            .iter()
            .map(|(s, c)| {
                c * readings
                    .iter()
                    .find(|(id, _)| *id == s.id())
                    .map_or(0.0, |(_, v)| *v)
            })
            .sum()
    }

    /// The chiller: one device exposing an accumulating channel (S#1) and three
    /// phase channels, all on ONE node. The formula zeroes the phases.
    fn chiller() -> (CompanyGraph, Vec<(u32, f64)>) {
        let chill = format!("{CO}|HN5#5");
        let g = CompanyGraph {
            nodes: vec![
                node(Level::Hn2, 997, None, CO),
                node(Level::Hn5, 5, Some((Level::Hn2, 997)), &chill),
            ],
            sensors: (1..=4)
                .map(|i| sensor(i, &chill, EnergyType::Electricity))
                .collect(),
            formulas: vec![
                f(
                    (Level::Hn5, 5),
                    EnergyType::Electricity,
                    Purpose::Total,
                    &[("S#2", 0.0), ("S#3", 0.0), ("S#4", 0.0)],
                ),
                f(
                    (Level::Hn5, 5),
                    EnergyType::Electricity,
                    Purpose::Cooling,
                    &[("S#1", 1.0)],
                ),
            ],
        };
        (g, vec![(1, 40.0), (2, 13.0), (3, 14.0), (4, 13.0)])
    }

    #[test]
    fn total_defaults_to_the_sum_of_own_sensors() {
        let (mut g, r) = chiller();
        g.formulas.clear();
        assert_eq!(
            value(&g, (Level::Hn5, 5), EnergyType::Electricity, Purpose::Total, &r),
            80.0,
            "no formula: everything below counts"
        );
    }

    #[test]
    fn a_total_formula_overrides_the_default() {
        let (g, r) = chiller();
        assert_eq!(
            value(&g, (Level::Hn5, 5), EnergyType::Electricity, Purpose::Total, &r),
            40.0,
            "phases zeroed, accumulating channel only"
        );
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
            sensors: (1..=4)
                .map(|i| sensor(i, &chill, EnergyType::Electricity))
                .collect(),
            formulas: vec![f(
                (Level::Hn5, 5),
                EnergyType::Electricity,
                Purpose::Total,
                &[("S#2", 0.0), ("S#3", 0.0), ("S#4", 0.0)],
            )],
        };
        let el = EnergyType::Electricity;
        assert_eq!(
            value(&g, (Level::Hn4, 4), el, Purpose::Total, &r),
            40.0,
            "the building sees 40, not 80"
        );
        assert_eq!(
            value(&g, (Level::Hn2, 997), el, Purpose::Total, &r),
            40.0,
            "and so does the company"
        );
    }

    /// A sensor belongs to `Total` automatically but to a purpose only if named.
    #[test]
    fn a_purpose_takes_no_sensor_unless_named() {
        let (g, r) = chiller();
        let n = (Level::Hn5, 5);
        let el = EnergyType::Electricity;
        assert_eq!(value(&g, n, el, Purpose::Cooling, &r), 40.0, "named");
        assert_eq!(value(&g, n, el, Purpose::Lighting, &r), 0.0, "not named");
    }

    /// Sideways sensor reference: main in C1, sub in C2.
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
            sensors: vec![
                sensor(10, &c1, EnergyType::Electricity),
                sensor(11, &c2, EnergyType::Electricity),
            ],
            formulas: vec![f(
                (Level::Hn4, 1),
                EnergyType::Electricity,
                Purpose::Total,
                &[("S#11", -1.0)],
            )],
        };
        (g, vec![(10, 100.0), (11, 30.0)])
    }

    #[test]
    fn main_and_sub_in_different_buildings() {
        let (g, r) = split_metering();
        let el = EnergyType::Electricity;
        assert_eq!(
            value(&g, (Level::Hn4, 1), el, Purpose::Total, &r),
            70.0,
            "C1 = main − sub"
        );
        assert_eq!(value(&g, (Level::Hn4, 2), el, Purpose::Total, &r), 30.0, "C2 = sub");
        assert_eq!(
            value(&g, (Level::Hn3, 3), el, Purpose::Total, &r),
            100.0,
            "the property is the main, counted exactly once"
        );
    }

    /// Shared plant apportioned across siblings — impossible under a
    /// descendants-only rule, natural with company-wide sensor references.
    #[test]
    fn shared_plant_splits_across_siblings() {
        let (mut g, mut r) = split_metering();
        r.push((12, 50.0));
        g.sensors
            .push(sensor(12, &format!("{CO}|HN3#3"), EnergyType::Electricity));
        g.formulas.push(f(
            (Level::Hn4, 1),
            EnergyType::Electricity,
            Purpose::Cooling,
            &[("S#12", 0.6)],
        ));
        g.formulas.push(f(
            (Level::Hn4, 2),
            EnergyType::Electricity,
            Purpose::Cooling,
            &[("S#12", 0.4)],
        ));
        assert_eq!(
            value(&g, (Level::Hn3, 3), EnergyType::Electricity, Purpose::Cooling, &r),
            50.0,
            "0.6 + 0.4 = one chiller"
        );
    }

    /// Cross-type output marks a claim derived; derived and outflow claims never
    /// reduce Unallocated.
    #[test]
    fn derived_and_outflow_do_not_allocate() {
        let (mut g, _) = chiller();
        g.formulas.push(f(
            (Level::Hn5, 5),
            EnergyType::DistrictCooling,
            Purpose::Cooling,
            &[("S#1", 3.2)],
        ));
        g.formulas.push(f(
            (Level::Hn5, 5),
            EnergyType::Electricity,
            Purpose::Generation,
            &[("S#1", 1.0)],
        ));
        let rows = flatten(&g);
        let allocates = |et: EnergyType, p: Purpose| {
            rows.iter()
                .find(|r| r.energy_type == et && r.purpose == p)
                .unwrap()
                .allocates
        };
        assert!(allocates(EnergyType::Electricity, Purpose::Cooling));
        assert!(
            !allocates(EnergyType::DistrictCooling, Purpose::Cooling),
            "derived"
        );
        assert!(
            !allocates(EnergyType::Electricity, Purpose::Generation),
            "outflow"
        );
    }

    /// A fully-claimed node has nothing unallocated, and the coefficients cancel
    /// to nothing — so no rows are written at all. Absence means zero, the same
    /// way it does everywhere else in the sparse roll-up.
    #[test]
    fn unallocated_is_total_minus_allocating() {
        let (g, r) = chiller();
        let el = EnergyType::Electricity;
        assert_eq!(
            value(&g, (Level::Hn5, 5), el, Purpose::Unallocated, &r),
            0.0,
            "40 total − 40 cooling"
        );
        assert!(
            !flatten(&g).iter().any(|w| w.purpose == Purpose::Unallocated),
            "nothing unallocated ⇒ no rows to write"
        );
    }

    /// A partly-claimed node reports the gap rather than hiding it — and does so
    /// as ordinary matrix rows, so the roll-up job subtracts nothing itself.
    #[test]
    fn unallocated_reports_the_gap() {
        let (mut g, r) = chiller();
        g.formulas.retain(|x| x.purpose != Purpose::Cooling);
        let el = EnergyType::Electricity;
        assert_eq!(
            value(&g, (Level::Hn5, 5), el, Purpose::Unallocated, &r),
            40.0
        );
        assert!(
            flatten(&g).iter().any(|w| {
                w.purpose == Purpose::Unallocated && w.energy_type == el && !w.allocates
            }),
            "materialised, not computed downstream"
        );
    }

    // -----------------------------------------------------------------------
    // The whole presentation fixture
    //
    // `docs/hierarchy-presentation.html` is the reference model; these are its
    // independently verified numbers. If this test and the demo disagree, one of
    // them is wrong and neither should be trusted until they don't.
    // -----------------------------------------------------------------------

    #[allow(clippy::type_complexity)]
    fn presentation() -> (CompanyGraph, Vec<(u32, f64)>) {
        use EnergyType::{DistrictCooling, DistrictHeating, Electricity, Gas, Heat};
        use Purpose::*;
        let pa = format!("{CO}|HN3#1");
        let (a1, a2) = (format!("{pa}|HN4#1"), format!("{pa}|HN4#2"));
        let a1a = format!("{a1}|HN5#1");
        let (msb, chill) = (format!("{a1a}|HN6#1"), format!("{a1a}|HN6#2"));
        let a1b = format!("{a1}|HN5#2");
        let pb = format!("{CO}|HN3#2");
        let (b1, b2) = (format!("{pb}|HN4#3"), format!("{pb}|HN4#4"));
        let pc = format!("{CO}|HN3#3");
        let (c1, c2) = (format!("{pc}|HN4#5"), format!("{pc}|HN4#6"));

        let g = CompanyGraph {
            nodes: vec![
                node(Level::Hn2, 997, None, CO),
                node(Level::Hn3, 1, Some((Level::Hn2, 997)), &pa),
                node(Level::Hn4, 1, Some((Level::Hn3, 1)), &a1),
                node(Level::Hn5, 1, Some((Level::Hn4, 1)), &a1a),
                node(Level::Hn6, 1, Some((Level::Hn5, 1)), &msb),
                node(Level::Hn6, 2, Some((Level::Hn5, 1)), &chill),
                node(Level::Hn5, 2, Some((Level::Hn4, 1)), &a1b),
                node(Level::Hn4, 2, Some((Level::Hn3, 1)), &a2),
                node(Level::Hn3, 2, Some((Level::Hn2, 997)), &pb),
                node(Level::Hn4, 3, Some((Level::Hn3, 2)), &b1),
                node(Level::Hn4, 4, Some((Level::Hn3, 2)), &b2),
                node(Level::Hn3, 3, Some((Level::Hn2, 997)), &pc),
                node(Level::Hn4, 5, Some((Level::Hn3, 3)), &c1),
                node(Level::Hn4, 6, Some((Level::Hn3, 3)), &c2),
            ],
            sensors: vec![
                sensor(1, &msb, Electricity), sensor(2, &msb, Electricity),
                sensor(3, &msb, Electricity),
                sensor(4, &chill, Electricity), sensor(5, &chill, Electricity),
                sensor(6, &chill, Electricity), sensor(7, &chill, Electricity),
                sensor(8, &a1b, Electricity), sensor(9, &a1b, Electricity),
                sensor(10, &a1b, Electricity),
                sensor(11, &b1, Electricity), sensor(12, &b1, Electricity),
                sensor(13, &b1, Electricity),
                sensor(14, &b2, Electricity), sensor(20, &b2, Gas),
                sensor(15, &c1, Electricity), sensor(16, &c2, Electricity),
                sensor(17, &a1, DistrictHeating), sensor(18, &a1, DistrictHeating),
                sensor(19, &a2, DistrictHeating),
            ],
            formulas: vec![
                // Chiller: one device, phase channels already in the accumulator.
                f((Level::Hn6, 2), Electricity, Total, &[("S#5", 0.0), ("S#6", 0.0), ("S#7", 0.0)]),
                f((Level::Hn6, 2), Electricity, Cooling, &[("S#4", 1.0)]),
                f((Level::Hn6, 2), DistrictCooling, Cooling, &[("S#4", 3.2)]),
                // PV: export leaves the site.
                f((Level::Hn5, 2), Electricity, Total, &[("S#10", -1.0)]),
                f((Level::Hn5, 2), Electricity, Generation, &[("S#10", 1.0)]),
                // Bimåler: the hot-water submeter is inside the main heat meter.
                f((Level::Hn4, 1), DistrictHeating, Total, &[("S#18", 0.0)]),
                f((Level::Hn4, 1), DistrictHeating, Dhw, &[("S#18", 1.0)]),
                f((Level::Hn4, 1), DistrictHeating, SpaceHeating, &[("S#17", 1.0), ("S#18", -1.0)]),
                // GUF/GAF where there is no submeter.
                f((Level::Hn4, 2), DistrictHeating, Dhw, &[("S#19", 0.28)]),
                f((Level::Hn4, 2), DistrictHeating, SpaceHeating, &[("S#19", 0.72)]),
                // One sensor, three purposes, all on this node. 0.7 + 0.3 = 1.
                f((Level::Hn4, 3), Electricity, Lighting, &[("S#11", 1.0)]),
                f((Level::Hn4, 3), Electricity, Ventilation, &[("S#12", 1.0)]),
                f((Level::Hn4, 3), Electricity, SpaceHeating, &[("S#13", 0.7)]),
                f((Level::Hn4, 3), Electricity, Dhw, &[("S#13", 0.3)]),
                f((Level::Hn4, 3), Heat, SpaceHeating, &[("S#13", 3.5)]),
                f((Level::Hn4, 4), Electricity, PlugLoads, &[("S#14", 1.0)]),
                f((Level::Hn4, 4), Heat, SpaceHeating, &[("S#20", 10.45)]),
                // Main in C1, sub in C2 — the sideways reference.
                f((Level::Hn4, 5), Electricity, Total, &[("S#16", -1.0)]),
            ],
        };
        let r = vec![
            (1, 30.0), (2, 28.0), (3, 26.0),
            (4, 40.0), (5, 13.0), (6, 14.0), (7, 13.0),
            (8, 50.0), (9, 100.0), (10, 30.0),
            (11, 22.0), (12, 18.0), (13, 24.0),
            (14, 15.0), (20, 4.0),
            (15, 100.0), (16, 30.0),
            (17, 120.0), (18, 35.0), (19, 90.0),
        ];
        (g, r)
    }

    #[test]
    fn presentation_fixture_reproduces_the_demo() {
        use EnergyType::{DistrictCooling, DistrictHeating, Electricity, Gas, Heat};
        let (g, r) = presentation();
        // Coefficients are floats, so 24 × 0.7 is 16.799999999999997. The demo
        // rounds for display; here we compare within a tolerance far tighter than
        // any meter's precision.
        let v = |n: (Level, u32), et, p| (value(&g, n, et, p, &r) * 1e6).round() / 1e6;
        let acme = (Level::Hn2, 997);

        // Recursion: the chiller corrects itself and every ancestor follows.
        assert_eq!(v((Level::Hn6, 2), Electricity, Purpose::Total), 40.0, "chiller");
        assert_eq!(v((Level::Hn6, 1), Electricity, Purpose::Total), 84.0, "switchboard");
        assert_eq!(v((Level::Hn5, 1), Electricity, Purpose::Total), 124.0, "area A1a");

        // Direction: import + production − export.
        assert_eq!(v((Level::Hn5, 2), Electricity, Purpose::Total), 120.0, "PV area");
        assert_eq!(v((Level::Hn5, 2), Electricity, Purpose::Generation), 30.0);

        // Sideways reference: main in C1, sub in C2, property counts it once.
        assert_eq!(v((Level::Hn4, 5), Electricity, Purpose::Total), 70.0, "C1");
        assert_eq!(v((Level::Hn4, 6), Electricity, Purpose::Total), 30.0, "C2");
        assert_eq!(v((Level::Hn3, 3), Electricity, Purpose::Total), 100.0, "property C");

        // Bimåler and GUF/GAF partition their heat exactly.
        assert_eq!(v((Level::Hn4, 1), DistrictHeating, Purpose::Total), 120.0);
        assert_eq!(v((Level::Hn4, 1), DistrictHeating, Purpose::Dhw), 35.0);
        assert_eq!(v((Level::Hn4, 1), DistrictHeating, Purpose::SpaceHeating), 85.0);
        assert_eq!(v((Level::Hn4, 1), DistrictHeating, Purpose::Unallocated), 0.0);
        assert_eq!(v((Level::Hn4, 2), DistrictHeating, Purpose::Unallocated), 0.0);

        // One sensor, many purposes.
        assert_eq!(v((Level::Hn4, 3), Electricity, Purpose::Total), 64.0);
        assert_eq!(v((Level::Hn4, 3), Electricity, Purpose::SpaceHeating), 16.8);
        assert_eq!(v((Level::Hn4, 3), Electricity, Purpose::Dhw), 7.2);
        assert_eq!(v((Level::Hn4, 3), Electricity, Purpose::Unallocated), 0.0);

        // Company totals.
        assert_eq!(v(acme, Electricity, Purpose::Total), 423.0);
        assert_eq!(v(acme, Electricity, Purpose::Unallocated), 304.0);
        assert_eq!(v(acme, Electricity, Purpose::Cooling), 40.0);
        assert_eq!(v(acme, DistrictHeating, Purpose::Total), 210.0);
        assert_eq!(v(acme, DistrictHeating, Purpose::Unallocated), 0.0);
        assert_eq!(v(acme, Gas, Purpose::Total), 4.0);
        assert_eq!(v(acme, Gas, Purpose::Unallocated), 4.0);

        // Derived series float free of every total.
        assert_eq!(v(acme, DistrictCooling, Purpose::Cooling), 128.0);
        assert_eq!(v(acme, DistrictCooling, Purpose::Total), 0.0);
        assert_eq!(v(acme, DistrictCooling, Purpose::Unallocated), 0.0);
        assert_eq!(v(acme, Heat, Purpose::SpaceHeating), 125.8);
        assert_eq!(v(acme, Heat, Purpose::Unallocated), 0.0);
    }

    // -----------------------------------------------------------------------
    // validate
    // -----------------------------------------------------------------------

    fn ok() -> NodeFormula {
        f(
            (Level::Hn5, 5),
            EnergyType::Electricity,
            Purpose::Cooling,
            &[("S#1", 1.0)],
        )
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
        let e = validate(&g, &NodeFormula { purpose: Purpose::Unallocated, ..ok() })
            .unwrap_err();
        assert!(e.contains("roll-up job"), "got: {e}");
    }

    /// Zero is legal — it is how a node excludes a reading.
    #[test]
    fn validate_accepts_zero_and_rejects_non_finite() {
        let (g, _) = chiller();
        assert!(validate(
            &g,
            &NodeFormula {
                purpose: Purpose::Total,
                terms: vec![Term {
                    reference: Reference::parse("S#2").unwrap(),
                    coefficient: 0.0,
                }],
                ..ok()
            }
        )
        .is_ok());
        let e = validate(
            &g,
            &NodeFormula {
                terms: vec![Term {
                    reference: Reference::parse("S#1").unwrap(),
                    coefficient: f64::NAN,
                }],
                ..ok()
            },
        )
        .unwrap_err();
        assert!(e.contains("finite"), "got: {e}");
    }

    /// Node references must be DIRECT CHILDREN — a deeper node already arrives
    /// through the chain, so referencing it would double count.
    #[test]
    fn validate_rejects_a_node_reference_that_is_not_a_direct_child() {
        let (g, _) = split_metering();
        let bad = f(
            (Level::Hn2, 997),
            EnergyType::Electricity,
            Purpose::Total,
            &[("HN4#1", 0.5)],
        );
        assert!(validate(&g, &bad).unwrap_err().contains("direct child"));
    }

    #[test]
    fn validate_accepts_a_direct_child_reference() {
        let (g, _) = split_metering();
        let good = f(
            (Level::Hn3, 3),
            EnergyType::Electricity,
            Purpose::Total,
            &[("HN4#1", 0.5)],
        );
        assert!(validate(&g, &good).is_ok());
    }

    /// Sideways SENSOR references are the whole point — accept them.
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
        let bad = f(
            (Level::Hn3, 3),
            EnergyType::Electricity,
            Purpose::Total,
            &[("S#10", 0.0)], // S#10 hangs off HN4#1, a child
        );
        assert!(validate(&g, &bad).unwrap_err().contains("already counted"));
    }

    #[test]
    fn validate_rejects_a_sensor_outside_the_company() {
        let (g, _) = chiller();
        let bad = f(
            (Level::Hn5, 5),
            EnergyType::Electricity,
            Purpose::Cooling,
            &[("S#999", 1.0)],
        );
        assert!(validate(&g, &bad).unwrap_err().contains("not found"));
    }

    /// The ≤ 1 rule allows every legitimate split and catches over-allocation.
    #[test]
    fn validate_allows_a_heat_pump_split_summing_to_one() {
        let (mut g, _) = chiller();
        g.formulas.retain(|x| x.purpose != Purpose::Cooling);
        g.formulas.push(f(
            (Level::Hn5, 5),
            EnergyType::Electricity,
            Purpose::SpaceHeating,
            &[("S#1", 0.7)],
        ));
        let dhw = f(
            (Level::Hn5, 5),
            EnergyType::Electricity,
            Purpose::Dhw,
            &[("S#1", 0.3)],
        );
        assert!(validate(&g, &dhw).is_ok(), "0.7 + 0.3 = 1.0");
    }

    #[test]
    fn validate_rejects_over_allocation() {
        let (mut g, _) = chiller();
        g.formulas.retain(|x| x.purpose != Purpose::Cooling);
        g.formulas.push(f(
            (Level::Hn5, 5),
            EnergyType::Electricity,
            Purpose::SpaceHeating,
            &[("S#1", 0.8)],
        ));
        let dhw = f(
            (Level::Hn5, 5),
            EnergyType::Electricity,
            Purpose::Dhw,
            &[("S#1", 0.5)],
        );
        assert!(validate(&g, &dhw).unwrap_err().contains("sum"));
    }

    /// The bimåler pattern nets to 0 for the submeter and 1 for the main.
    #[test]
    fn validate_accepts_the_bimaaler_pattern() {
        let b = format!("{CO}|HN4#9");
        let mut g = CompanyGraph {
            nodes: vec![
                node(Level::Hn2, 997, None, CO),
                node(Level::Hn4, 9, Some((Level::Hn2, 997)), &b),
            ],
            sensors: vec![
                sensor(30, &b, EnergyType::DistrictHeating),
                sensor(31, &b, EnergyType::DistrictHeating),
            ],
            formulas: vec![],
        };
        let dh = EnergyType::DistrictHeating;
        let dhw = f((Level::Hn4, 9), dh, Purpose::Dhw, &[("S#31", 1.0)]);
        assert!(validate(&g, &dhw).is_ok());
        g.formulas.push(dhw);
        let sh = f(
            (Level::Hn4, 9),
            dh,
            Purpose::SpaceHeating,
            &[("S#30", 1.0), ("S#31", -1.0)],
        );
        assert!(validate(&g, &sh).is_ok(), "S#31 nets to 0, S#30 to 1");
    }

    /// A series mixing derived and metered contributions would make Unallocated
    /// ambiguous.
    #[test]
    fn validate_rejects_mixed_derived_ness_for_one_series() {
        let (mut g, _) = chiller();
        g.sensors
            .push(sensor(9, &format!("{CO}|HN5#5"), EnergyType::Gas));
        let mixed = f(
            (Level::Hn2, 997),
            EnergyType::Electricity,
            Purpose::Cooling,
            &[("S#9", 3.0)], // gas sensor, electricity output → derived
        );
        assert!(validate(&g, &mixed).unwrap_err().contains("derived"));
    }

    /// flatten() must agree with the recursion it flattens.
    #[test]
    fn flatten_agrees_with_the_recursion() {
        let (g, r) = chiller();
        for purpose in [Purpose::Total, Purpose::Cooling, Purpose::Unallocated] {
            let direct = value(&g, (Level::Hn5, 5), EnergyType::Electricity, purpose, &r);
            let flat: f64 = flatten(&g)
                .iter()
                .filter(|w| {
                    w.node_path == format!("{CO}|HN5#5")
                        && w.energy_type == EnergyType::Electricity
                        && w.purpose == purpose
                })
                .map(|w| {
                    w.coefficient
                        * r.iter()
                            .find(|(id, _)| *id == w.sensor.id())
                            .map_or(0.0, |(_, v)| *v)
                })
                .sum();
            assert_eq!(direct, flat, "{purpose}");
        }
    }
}
