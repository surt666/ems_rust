//! Pure request types — same parse-don't-validate shapes as the DataFusion arm
//! (duplicated rather than shared via a lib: this is a self-contained spike).

use chrono::{DateTime, Utc};

/// Sensor id, parsed so it is safe to bind/interpolate. Even though DuckDB queries
/// use bound params, we still reject junk early for a clean 400.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaqId(String);

impl DaqId {
    pub fn parse(s: &str) -> Result<DaqId, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("daqid is empty".into());
        }
        let ok = s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '#'));
        if !ok {
            return Err(format!("daqid has illegal characters: {s:?}"));
        }
        Ok(DaqId(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawQuery {
    pub daq_id: DaqId,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl RawQuery {
    pub fn parse(daqid: &str, from: &str, to: &str) -> Result<RawQuery, String> {
        let daq_id = DaqId::parse(daqid)?;
        let from = parse_time("fromtime", from)?;
        let to = parse_time("totime", to)?;
        if from > to {
            return Err(format!("fromtime {from} is after totime {to}"));
        }
        Ok(RawQuery { daq_id, from, to })
    }
}

fn parse_time(field: &str, s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s.trim())
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| format!("{field} is not RFC3339 ({e}): {s:?}"))
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RawRow {
    pub timestamp: String,
    pub value: Option<f64>,
    pub unit: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daqid_and_window_validation() {
        assert!(DaqId::parse("daq:std_json_v1:klepierre:60098154:energy").is_ok());
        assert!(DaqId::parse("1'; DROP TABLE x").is_err());
        assert!(RawQuery::parse("10011", "2026-01-02T00:00:00Z", "2026-01-01T00:00:00Z").is_err());
        assert!(RawQuery::parse("10011", "2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z").is_ok());
    }
}
