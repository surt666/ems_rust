use std::fmt;
use serde::{Deserialize, Serialize};
use strum::EnumIter;

// ---------------------------------------------------------------------------
// SensorId
// ---------------------------------------------------------------------------

/// A sensor identifier: a newtype over u32, printed/parsed as `"S#<n>"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SensorId(pub u32);

impl SensorId {
    /// Construct a `SensorId` from a raw integer.
    pub fn make(n: u32) -> SensorId {
        SensorId(n)
    }

    /// Return the raw integer.
    pub fn id(&self) -> u32 {
        self.0
    }

    /// Parse `"S#<n>"` → `Ok(SensorId(n))`, or `Err(message)`.
    pub fn parse(s: &str) -> Result<SensorId, String> {
        if !s.starts_with("S#") {
            return Err(format!("missing S# prefix in {:?}", s));
        }
        let rest = &s[2..];
        match rest.parse::<u32>() {
            Ok(n) => Ok(SensorId(n)),
            Err(_) => Err(format!("bad id in {:?}", s)),
        }
    }
}

impl fmt::Display for SensorId {
    /// Renders as `"S#<n>"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "S#{}", self.0)
    }
}

impl Serialize for SensorId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SensorId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        SensorId::parse(&raw).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// Level
// ---------------------------------------------------------------------------

/// Hierarchy level, Hn0 (root) through Hn9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter, Serialize, Deserialize,
         strum::Display, strum::EnumString)]
pub enum Level {
    #[strum(serialize = "hn0")]
    Hn0,
    #[strum(serialize = "hn1")]
    Hn1,
    #[strum(serialize = "hn2")]
    Hn2,
    #[strum(serialize = "hn3")]
    Hn3,
    #[strum(serialize = "hn4")]
    Hn4,
    #[strum(serialize = "hn5")]
    Hn5,
    #[strum(serialize = "hn6")]
    Hn6,
    #[strum(serialize = "hn7")]
    Hn7,
    #[strum(serialize = "hn8")]
    Hn8,
    #[strum(serialize = "hn9")]
    Hn9,
}

impl Level {
    /// Numeric depth: Hn0 → 0, …, Hn9 → 9.
    pub fn depth(&self) -> u8 {
        match self {
            Level::Hn0 => 0,
            Level::Hn1 => 1,
            Level::Hn2 => 2,
            Level::Hn3 => 3,
            Level::Hn4 => 4,
            Level::Hn5 => 5,
            Level::Hn6 => 6,
            Level::Hn7 => 7,
            Level::Hn8 => 8,
            Level::Hn9 => 9,
        }
    }

    /// Construct from a depth value; returns `None` for depth > 9.
    pub fn of_depth(d: u8) -> Option<Level> {
        match d {
            0 => Some(Level::Hn0),
            1 => Some(Level::Hn1),
            2 => Some(Level::Hn2),
            3 => Some(Level::Hn3),
            4 => Some(Level::Hn4),
            5 => Some(Level::Hn5),
            6 => Some(Level::Hn6),
            7 => Some(Level::Hn7),
            8 => Some(Level::Hn8),
            9 => Some(Level::Hn9),
            _ => None,
        }
    }

}

// ---------------------------------------------------------------------------
// NodeId
// ---------------------------------------------------------------------------

/// A hierarchy node identifier: either the root or a level+integer pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeId {
    Root,
    Node { level: Level, id: u32 },
}

impl NodeId {
    /// Construct a non-root node.
    pub fn make(level: Level, id: u32) -> NodeId {
        NodeId::Node { level, id }
    }

    /// The singleton root node.
    pub fn root() -> NodeId {
        NodeId::Root
    }

    pub fn is_root(&self) -> bool {
        matches!(self, NodeId::Root)
    }

    /// Level of this node; Root returns `Level::Hn0`.
    pub fn level(&self) -> Level {
        match self {
            NodeId::Root => Level::Hn0,
            NodeId::Node { level, .. } => *level,
        }
    }

    /// Parse `"HN0#root"` or `"HN<n>#<id>"`.
    ///
    /// Matches OCaml `Node_id.of_string`:
    /// - `"HN0#root"` is checked first (exact match → Root).
    /// - Otherwise find `#`; prefix must be exactly 3 chars `HN<digit>`; rest must
    ///   parse as a non-negative integer.
    /// - `"HN4#"` (empty rest), `"HN10#42"` (prefix length 4), `"hn4#10"` (lowercase
    ///   prefix), `"HN4#not-an-int"` all return `Err`.
    pub fn parse(s: &str) -> Result<NodeId, String> {
        if s == "HN0#root" {
            return Ok(NodeId::Root);
        }
        let hash_pos = s
            .find('#')
            .ok_or_else(|| format!("no '#' in {:?}", s))?;
        let prefix = &s[..hash_pos];
        let rest = &s[hash_pos + 1..];
        let pbytes = prefix.as_bytes();
        if pbytes.len() != 3 || pbytes[0] != b'H' || pbytes[1] != b'N' {
            return Err(format!("bad prefix in {:?}", s));
        }
        match pbytes[2] {
            c @ b'0'..=b'9' => {
                let d = c - b'0';
                let level = Level::of_depth(d)
                    .ok_or_else(|| format!("bad level in {:?}", s))?;
                let id: u32 = rest
                    .parse()
                    .map_err(|_| format!("bad id in {:?}", s))?;
                Ok(NodeId::Node { level, id })
            }
            _ => Err(format!("bad level in {:?}", s)),
        }
    }
}

impl fmt::Display for NodeId {
    /// Root → `"HN0#root"`;  Node → `"HN<depth>#<id>"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeId::Root => write!(f, "HN0#root"),
            NodeId::Node { level, id } => write!(f, "HN{}#{}", level.depth(), id),
        }
    }
}

impl Serialize for NodeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        NodeId::parse(&raw).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// UserId
// ---------------------------------------------------------------------------

/// A user identifier: a newtype over an email string.
///
/// Printed/parsed as `"U#<email>"`.  Ported from `user_id.ml`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserId(String);

impl UserId {
    /// Construct a `UserId` from an email address (no prefix stored).
    pub fn of_email(email: &str) -> UserId {
        UserId(email.to_owned())
    }

    /// Return the raw email address (without any prefix).
    pub fn email(&self) -> &str {
        &self.0
    }

    /// Parse `"U#<email>"` → `Ok(UserId)`, or `Err(message)`.
    ///
    /// Rejects: missing `U#` prefix, empty email after prefix.
    /// Matches OCaml `User_id.of_string`.
    pub fn parse(s: &str) -> Result<UserId, String> {
        if let Some(email) = s.strip_prefix("U#") {
            if email.is_empty() {
                Err("empty email after U#".to_string())
            } else {
                Ok(UserId(email.to_owned()))
            }
        } else {
            Err("user_id must start with U#".to_string())
        }
    }
}

impl fmt::Display for UserId {
    /// Renders as `"U#<email>"` — matches OCaml `User_id.to_string`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "U#{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// Tests  (ported 1:1 from OCaml test_domain_level.ml / test_domain_node_id.ml)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Level tests --------------------------------------------------------

    /// Port of `test_domain_level.ml :: roundtrip`
    /// Checks of_string / depth / to_string for hn0..hn9.
    #[test]
    fn level_of_string_roundtrip() {
        for i in 0u8..=9 {
            let s = format!("hn{}", i);
            let lvl = s.parse::<Level>().unwrap_or_else(|e| panic!("parse {:?} -> {}", s, e));
            assert_eq!(lvl.depth(), i, "depth mismatch for {}", s);
            assert_eq!(lvl.to_string(), s, "render mismatch for {}", s);
        }
    }

    /// Port of `test_domain_level.ml :: rejects_bad_input`
    #[test]
    fn level_rejects_malformed() {
        for bad in &["", "hn", "hn10", "hn-1", "HN0", "sensor"] {
            assert!(
                bad.parse::<Level>().is_err(),
                "expected Err for {:?} but got Ok",
                bad
            );
        }
    }

    /// Port of the spec requirement: of_depth roundtrip for 0..=9, None for 10.
    #[test]
    fn level_depth_roundtrip() {
        for d in 0..=9u8 {
            assert_eq!(Level::of_depth(d).unwrap().depth(), d);
        }
        assert_eq!(Level::of_depth(10), None);
    }

    // --- NodeId tests -------------------------------------------------------

    /// Port of `test_domain_node_id.ml :: roundtrip`
    #[test]
    fn node_id_roundtrip() {
        let id = NodeId::make(Level::Hn4, 10042);
        let s = id.to_string();
        assert_eq!(s, "HN4#10042");
        let id2 = NodeId::parse(&s).unwrap_or_else(|e| panic!("parse {:?} -> {}", s, e));
        assert_eq!(id.level().depth(), id2.level().depth());
        // Access the id field directly for comparison
        match (&id, &id2) {
            (NodeId::Node { id: a, .. }, NodeId::Node { id: b, .. }) => assert_eq!(a, b),
            _ => panic!("expected Node variant"),
        }
    }

    /// Port of `test_domain_node_id.ml :: rejects_bad`
    #[test]
    fn node_id_rejects_bad() {
        for bad in &["", "HN4", "HN4#", "hn4#10", "HN10#42", "HN4#not-an-int"] {
            assert!(
                NodeId::parse(bad).is_err(),
                "expected Err for {:?} but got Ok",
                bad
            );
        }
    }

    /// Port of `test_domain_node_id.ml :: root_constant`
    #[test]
    fn node_id_root_constant() {
        assert_eq!(NodeId::root().to_string(), "HN0#root");
        assert!(NodeId::root().is_root());
    }

    /// Spec cases from the task description.
    #[test]
    fn node_id_render_parse() {
        assert_eq!(NodeId::make(Level::Hn2, 2).to_string(), "HN2#2");
        assert_eq!(
            NodeId::parse("HN2#2").unwrap(),
            NodeId::make(Level::Hn2, 2)
        );
        assert_eq!(NodeId::Root.to_string(), "HN0#root");
        assert_eq!(NodeId::parse("HN0#root").unwrap(), NodeId::Root);
    }

    /// Additional bad-prefix / no-hash cases.
    #[test]
    fn node_id_rejects_bad_extra() {
        assert!(NodeId::parse("X#2").is_err()); // bad prefix
        assert!(NodeId::parse("HN2").is_err()); // no '#'
        assert!(NodeId::parse("HN2#abc").is_err()); // non-int id
    }

    /// OCaml `equal` semantics: Root==Root, Node by depth+id.
    #[test]
    fn node_id_equality() {
        assert_eq!(NodeId::Root, NodeId::Root);
        assert_eq!(NodeId::make(Level::Hn3, 7), NodeId::make(Level::Hn3, 7));
        assert_ne!(NodeId::make(Level::Hn3, 7), NodeId::make(Level::Hn3, 8));
        assert_ne!(NodeId::make(Level::Hn3, 7), NodeId::make(Level::Hn4, 7));
        assert_ne!(NodeId::Root, NodeId::make(Level::Hn0, 0));
    }

    /// `NodeId::level` returns Hn0 for Root.
    #[test]
    fn node_id_level_of_root() {
        assert_eq!(NodeId::Root.level(), Level::Hn0);
        assert!(!NodeId::make(Level::Hn1, 1).is_root());
    }

    // --- SensorId tests (port of test_domain_sensor_id.ml) ------------------

    /// Port of `round_trip`: make → to_string → parse → equal.
    #[test]
    fn sensor_id_round_trip() {
        let id = SensorId::make(10042);
        let s = id.to_string();
        assert_eq!(s, "S#10042");
        let id2 = SensorId::parse(&s).unwrap_or_else(|e| panic!("parse failed: {}", e));
        assert_eq!(id, id2);
        assert_eq!(id.id(), id2.id());
    }

    /// Port of `rejects_missing_prefix`.
    #[test]
    fn sensor_id_rejects_missing_prefix() {
        assert!(SensorId::parse("10042").is_err(), "should reject missing S#");
    }

    /// Port of `rejects_wrong_prefix`.
    #[test]
    fn sensor_id_rejects_wrong_prefix() {
        assert!(SensorId::parse("HN4#10042").is_err(), "should reject HN4# prefix");
    }

    /// Port of `rejects_bad_id`.
    #[test]
    fn sensor_id_rejects_bad_id() {
        assert!(SensorId::parse("S#not-an-int").is_err(), "should reject bad id");
    }

    // --- UserId tests (port of test_domain_user.ml user_id cases) -----------

    /// Port of `user_id_rt`: of_email → to_string → of_string roundtrip.
    #[test]
    fn user_id_roundtrip() {
        let id = UserId::of_email("alice@example.com");
        assert_eq!(id.to_string(), "U#alice@example.com");
        match UserId::parse("U#alice@example.com") {
            Ok(id2) => assert_eq!(id2.email(), "alice@example.com"),
            Err(e) => panic!("parse failed: {}", e),
        }
    }

    /// Port of `user_id_rejects_bad`: missing U# prefix must fail.
    #[test]
    fn user_id_rejects_missing_prefix() {
        assert!(
            UserId::parse("alice@example.com").is_err(),
            "missing U# prefix should fail"
        );
    }

    /// Extra: empty email after U# must fail.
    #[test]
    fn user_id_rejects_empty_email() {
        assert!(UserId::parse("U#").is_err(), "empty email should fail");
    }
}
