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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    /// A node's own formula is a `Total` formula — the same shape, not a
    /// special case.
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
