use chrono::{DateTime, Utc};
use typed_builder::TypedBuilder;

use crate::domain::formula::Formula;
use crate::domain::ids::{NodeId, SensorId};
use crate::domain::values::MeterType;

// ---------------------------------------------------------------------------
// Sensor
// ---------------------------------------------------------------------------

/// A sensor attached to a hierarchy node.
///
/// Ported 1:1 from `sensor.ml`.
///
/// `path` is a pipe-separated list of ancestor ids from root down to and
/// including self (the `gsi1sk` attribute in DynamoDB).
#[derive(Clone, Debug, PartialEq, TypedBuilder)]
pub struct Sensor {
    pub id: SensorId,
    pub created: DateTime<Utc>,
    pub daq_id: String,
    pub path: String,
    pub purpose: String,
    pub meter_type: MeterType,
    #[builder(default)]
    pub unit: Option<String>,
    #[builder(default = Formula::Identity)]
    pub formula: Formula,
    #[builder(default)]
    pub resample_minutes: Option<i32>,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Path separator (same as for nodes).
const PATH_SEP: &str = "|";

/// Build the path of a sensor given the parent node's full path and the
/// sensor id as a string.  Matches OCaml `Sensor.child_path`.
pub fn child_path(parent_path: &str, sensor_id_str: &str) -> String {
    format!("{}{}{}", parent_path, PATH_SEP, sensor_id_str)
}

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

impl Sensor {
    /// Walk the path from the back and return the last `HN<n>#<id>` segment
    /// as a `NodeId`.  Sensors must always have a node parent.
    ///
    /// Panics if the path contains no valid node-id segment.
    /// Matches OCaml `Sensor.parent_id`.
    pub fn parent_id(&self) -> NodeId {
        let parts: Vec<&str> = self
            .path
            .split('|')
            .filter(|s| !s.is_empty())
            .collect();

        // Walk forward, keeping the last successfully parsed NodeId.
        let mut last: Option<NodeId> = None;
        for seg in &parts {
            if let Ok(nid) = NodeId::parse(seg) {
                last = Some(nid);
            }
        }
        last.unwrap_or_else(|| {
            panic!("Sensor.parent_id: no node segment in path {:?}", self.path)
        })
    }
}

// ---------------------------------------------------------------------------
// Tests (port of test_domain_sensor.ml)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::Level;
    use chrono::DateTime;

    fn ptime_of(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn make_sample() -> Sensor {
        let parent = NodeId::make(Level::Hn5, 10042);
        let id = SensorId::make(20001);
        let parent_path = format!(
            "{}|{}",
            NodeId::root().to_string(),
            parent.to_string()
        );
        let path = child_path(&parent_path, &id.to_string());
        Sensor {
            id,
            created: ptime_of("2026-04-18T10:00:00Z"),
            daq_id: "daq:adeunis_pu_v1:123:0018b210000191c7:counter_a".to_owned(),
            path,
            purpose: "Electricity".to_owned(),
            meter_type: MeterType::Counter,
            unit: Some("kWh".to_owned()),
            formula: Formula::Identity,
            resample_minutes: Some(15),
        }
    }

    /// Port of `fields_preserved`.
    #[test]
    fn fields_preserved() {
        let s = make_sample();
        assert_eq!(s.purpose, "Electricity");
        assert_eq!(s.daq_id, "daq:adeunis_pu_v1:123:0018b210000191c7:counter_a");
        assert!(matches!(s.meter_type, MeterType::Counter));
        assert_eq!(s.unit, Some("kWh".to_owned()));
    }

    /// Port of `meter_type_to_string`.
    #[test]
    fn meter_type_to_string() {
        assert_eq!(MeterType::Counter.to_str(), "counter");
        assert_eq!(MeterType::Gauge.to_str(), "gauge");
    }

    /// Port of `meter_type_of_string`.
    #[test]
    fn meter_type_of_string() {
        assert_eq!(MeterType::parse("counter"), Ok(MeterType::Counter));
        assert_eq!(MeterType::parse("gauge"), Ok(MeterType::Gauge));
        assert!(MeterType::parse("wat").is_err());
    }

    /// Port of `parent_id_extracts_last_node_segment`.
    #[test]
    fn parent_id_extracts_last_node_segment() {
        let s = make_sample();
        let expected = NodeId::make(Level::Hn5, 10042);
        assert_eq!(
            s.parent_id().to_string(),
            expected.to_string(),
            "parent id from path"
        );
    }

    /// child_path joins with `|`.
    #[test]
    fn child_path_join() {
        let p = child_path("HN0#root|HN5#10042", "S#20001");
        assert_eq!(p, "HN0#root|HN5#10042|S#20001");
    }

    /// parent_id panics for a path with no node segments.
    #[test]
    #[should_panic(expected = "Sensor.parent_id")]
    fn parent_id_panics_no_node_segment() {
        let s = Sensor {
            id: SensorId::make(1),
            created: ptime_of("2026-01-01T00:00:00Z"),
            daq_id: "x".to_owned(),
            path: "S#1".to_owned(), // no HN segment
            purpose: "x".to_owned(),
            meter_type: MeterType::Counter,
            unit: None,
            formula: Formula::Identity,
            resample_minutes: None,
        };
        let _ = s.parent_id();
    }
}
