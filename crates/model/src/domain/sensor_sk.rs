use std::fmt;

use chrono::{DateTime, Utc};

// ---------------------------------------------------------------------------
// SensorSk
// ---------------------------------------------------------------------------

/// Sort-key discriminator for sensor DynamoDB items.
///
/// `Active(ts)` → `"active#<rfc3339Z>"`
/// `History(ts)` → `"<rfc3339Z>"`
///
/// The RFC3339 format uses UTC with the `Z` suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SensorSk {
    Active(DateTime<Utc>),
    History(DateTime<Utc>),
}

const ACTIVE_PREFIX: &str = "active#";

/// Format a `DateTime<Utc>` as RFC3339 with `Z` suffix (no fractional seconds
/// for whole-second values; chrono's `to_rfc3339` uses `+00:00` so we replace
/// that).
fn dt_to_rfc3339z(dt: &DateTime<Utc>) -> String {
    // chrono formats as "2026-04-18T10:00:00+00:00"; replace the offset with "Z"
    let s = dt.to_rfc3339();
    if let Some(stripped) = s.strip_suffix("+00:00") {
        format!("{}Z", stripped)
    } else {
        s
    }
}

/// Parse an RFC3339 string.
fn parse_ts(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| format!("bad timestamp {:?}", s))
}

impl SensorSk {
    /// Parse `"active#<ts>"` → `Active(ts)` or plain `"<ts>"` → `History(ts)`.
    pub fn parse(s: &str) -> Result<SensorSk, String> {
        if let Some(rest) = s.strip_prefix(ACTIVE_PREFIX) {
            parse_ts(rest).map(SensorSk::Active)
        } else {
            parse_ts(s).map(SensorSk::History)
        }
    }
}

impl fmt::Display for SensorSk {
    /// `Active(t)` → `"active#<rfc3339Z>"`;  `History(t)` → `"<rfc3339Z>"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SensorSk::Active(t) => write!(f, "{}{}", ACTIVE_PREFIX, dt_to_rfc3339z(t)),
            SensorSk::History(t) => write!(f, "{}", dt_to_rfc3339z(t)),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
        /// Helper: parse the sample timestamp used in tests.
    fn sample_time() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-18T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn active_encodes_with_prefix() {
        let t = sample_time();
        assert_eq!(
            SensorSk::Active(t).to_string(),
            "active#2026-04-18T10:00:00Z"
        );
    }

    #[test]
    fn history_encodes_without_prefix() {
        let t = sample_time();
        assert_eq!(SensorSk::History(t).to_string(), "2026-04-18T10:00:00Z");
    }

    #[test]
    fn parse_active() {
        match SensorSk::parse("active#2026-04-18T10:00:00Z") {
            Ok(SensorSk::Active(t)) => {
                assert_eq!(dt_to_rfc3339z(&t), "2026-04-18T10:00:00Z");
            }
            Ok(other) => panic!("expected Active, got {:?}", other),
            Err(e) => panic!("parse failed: {}", e),
        }
    }

    #[test]
    fn parse_history() {
        match SensorSk::parse("2026-03-01T00:00:00Z") {
            Ok(SensorSk::History(t)) => {
                assert_eq!(dt_to_rfc3339z(&t), "2026-03-01T00:00:00Z");
            }
            Ok(other) => panic!("expected History, got {:?}", other),
            Err(e) => panic!("parse failed: {}", e),
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(SensorSk::parse("garbage").is_err(), "should reject garbage");
    }

    /// Round-trip: Active encode → parse → same variant and time.
    #[test]
    fn active_roundtrip() {
        let t = sample_time();
        let s = SensorSk::Active(t).to_string();
        match SensorSk::parse(&s) {
            Ok(SensorSk::Active(t2)) => assert_eq!(t, t2),
            other => panic!("expected Active after roundtrip, got {:?}", other),
        }
    }

    /// Round-trip: History encode → parse → same variant and time.
    #[test]
    fn history_roundtrip() {
        let t = sample_time();
        let s = SensorSk::History(t).to_string();
        match SensorSk::parse(&s) {
            Ok(SensorSk::History(t2)) => assert_eq!(t, t2),
            other => panic!("expected History after roundtrip, got {:?}", other),
        }
    }

    /// `active#` prefix followed by garbage timestamp is rejected.
    #[test]
    fn rejects_active_prefix_bad_ts() {
        assert!(SensorSk::parse("active#garbage").is_err());
    }

    /// Ensure the UTC offset variant (+00:00) is also accepted by parse.
    #[test]
    fn accepts_utc_offset_form() {
        // chrono accepts both Z and +00:00; our parse should handle either
        assert!(SensorSk::parse("2026-04-18T10:00:00+00:00").is_ok());
    }

}
