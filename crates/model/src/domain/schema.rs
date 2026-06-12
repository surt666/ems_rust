#![allow(clippy::type_complexity)]

use typed_builder::TypedBuilder;

use crate::domain::values::FieldType;

// ---------------------------------------------------------------------------
// EdgeSpec
// ---------------------------------------------------------------------------

/// Cardinality for a directed type edge. The child type name is the key in
/// `Schema::edges`; v1's separate `label` is gone — the child type IS the label.
#[derive(Clone, Debug, PartialEq, TypedBuilder)]
pub struct EdgeSpec {
    #[builder(default)]
    pub min: Option<i32>,
    #[builder(default)]
    pub max: Option<i32>,
}

// ---------------------------------------------------------------------------
// FieldSpec
// ---------------------------------------------------------------------------

/// A metadata field specification: its type + whether it is required.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldSpec {
    pub typ: FieldType,
    pub required: bool,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// Reserved type of the hn2 node itself — root of the type graph.
pub const COMPANY_TYPE: &str = "company";
/// Reserved type of hn1 nodes — outside the schema.
pub const PARTNER_TYPE: &str = "partner";

/// A hierarchy schema (v2): a DAG of node types rooted at "company".
/// Levels (hnN) are NOT a schema concept — a node's level is its depth.
#[derive(Clone, Debug, PartialEq, TypedBuilder)]
pub struct Schema {
    pub version: u32,
    /// parent type → (child type → cardinality)
    pub edges: Vec<(String, Vec<(String, EdgeSpec)>)>,
    /// type → (field name → field spec)
    pub metadata: Vec<(String, Vec<(String, FieldSpec)>)>,
    /// types at which sensors may attach
    pub sensors: Vec<String>,
}

impl Schema {
    /// Allowed child types (with cardinality) under `parent` type.
    pub fn allowed_children(&self, parent: &str) -> &[(String, EdgeSpec)] {
        self.edges
            .iter()
            .find(|(t, _)| t == parent)
            .map(|(_, children)| children.as_slice())
            .unwrap_or(&[])
    }

    /// Cardinality of the `parent` → `child` type edge, if allowed.
    pub fn edge_between(&self, parent: &str, child: &str) -> Option<&EdgeSpec> {
        self.allowed_children(parent)
            .iter()
            .find(|(t, _)| t == child)
            .map(|(_, spec)| spec)
    }

    /// Metadata field specs for a type.
    pub fn metadata_for(&self, typ: &str) -> &[(String, FieldSpec)] {
        self.metadata
            .iter()
            .find(|(t, _)| t == typ)
            .map(|(_, fields)| fields.as_slice())
            .unwrap_or(&[])
    }

    /// Whether sensors may attach to nodes of this type.
    pub fn allows_sensors(&self, typ: &str) -> bool {
        self.sensors.iter().any(|t| t == typ)
    }

    /// Validate v2 invariants:
    /// 1. names non-empty; "partner" nowhere; "company" never a child; no self-edges;
    ///    per parent no duplicate child; min <= max.
    /// 2. the type graph is a DAG.
    /// 3. every referenced type (parents, metadata, sensors) reachable from "company".
    /// 4. longest path from "company" <= 7 edges (deepest node fits hn9).
    /// 5. metadata field specs valid; no duplicate sensors entries.
    pub fn validate(&self) -> Result<(), String> {
        use std::collections::{HashMap, HashSet};

        // 1: local name/cardinality rules
        for (parent, children) in &self.edges {
            if parent.is_empty() {
                return Err("empty parent type name".to_string());
            }
            if parent == PARTNER_TYPE {
                return Err("\"partner\" is reserved and cannot appear in the schema".to_string());
            }
            let mut seen: HashSet<&str> = HashSet::new();
            for (child, spec) in children {
                if child.is_empty() {
                    return Err(format!("edge from {:?}: empty child type name", parent));
                }
                if child == COMPANY_TYPE || child == PARTNER_TYPE {
                    return Err(format!(
                        "edge {} -> {}: reserved type cannot be a child",
                        parent, child
                    ));
                }
                if child == parent {
                    return Err(format!("self edge {} -> {}", parent, child));
                }
                if !seen.insert(child.as_str()) {
                    return Err(format!("duplicate child {} under {}", child, parent));
                }
                if let (Some(a), Some(b)) = (spec.min, spec.max) {
                    if a > b {
                        return Err(format!("edge {} -> {} has min > max", parent, child));
                    }
                }
            }
        }

        // adjacency map
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for (parent, children) in &self.edges {
            let entry = adj.entry(parent.as_str()).or_default();
            for (child, _) in children {
                entry.push(child.as_str());
            }
        }

        // 2: DAG check — 3-color DFS from every declared parent
        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White,
            Gray,
            Black,
        }
        fn dfs<'a>(
            node: &'a str,
            adj: &HashMap<&'a str, Vec<&'a str>>,
            color: &mut HashMap<&'a str, Color>,
        ) -> Result<(), String> {
            color.insert(node, Color::Gray);
            for next in adj.get(node).map(|v| v.as_slice()).unwrap_or(&[]) {
                match color.get(next).copied().unwrap_or(Color::White) {
                    Color::Gray => return Err(format!("cycle involving type {:?}", next)),
                    Color::White => dfs(next, adj, color)?,
                    Color::Black => {}
                }
            }
            color.insert(node, Color::Black);
            Ok(())
        }
        let mut roots: Vec<&str> = adj.keys().copied().collect();
        roots.sort(); // deterministic error messages
        let mut color: HashMap<&str, Color> = HashMap::new();
        for r in roots {
            if color.get(r).copied().unwrap_or(Color::White) == Color::White {
                dfs(r, &adj, &mut color)?;
            }
        }

        // 3: reachability from "company"
        let mut reach: HashSet<&str> = HashSet::new();
        reach.insert(COMPANY_TYPE);
        let mut queue: Vec<&str> = vec![COMPANY_TYPE];
        while let Some(t) = queue.pop() {
            for c in adj.get(t).map(|v| v.as_slice()).unwrap_or(&[]) {
                if reach.insert(c) {
                    queue.push(c);
                }
            }
        }
        for (parent, _) in &self.edges {
            if !reach.contains(parent.as_str()) {
                return Err(format!("type {:?} not reachable from \"company\"", parent));
            }
        }
        for (typ, _) in &self.metadata {
            if !reach.contains(typ.as_str()) {
                return Err(format!("metadata for unreachable type {:?}", typ));
            }
        }
        let mut seen_sensors: HashSet<&str> = HashSet::new();
        for typ in &self.sensors {
            if !reach.contains(typ.as_str()) {
                return Err(format!("sensors for unreachable type {:?}", typ));
            }
            if !seen_sensors.insert(typ.as_str()) {
                return Err(format!("duplicate sensors entry {:?}", typ));
            }
        }

        // 4: longest path from "company" ≤ 7 (DAG → DFS with memo)
        fn longest<'a>(
            node: &'a str,
            adj: &HashMap<&'a str, Vec<&'a str>>,
            memo: &mut HashMap<&'a str, usize>,
        ) -> usize {
            if let Some(d) = memo.get(node) {
                return *d;
            }
            let d = adj
                .get(node)
                .map(|v| v.iter().map(|c| 1 + longest(c, adj, memo)).max().unwrap_or(0))
                .unwrap_or(0);
            memo.insert(node, d);
            d
        }
        let mut memo: HashMap<&str, usize> = HashMap::new();
        let depth = longest(COMPANY_TYPE, &adj, &mut memo);
        if depth > 7 {
            return Err(format!(
                "longest type chain from \"company\" is {} edges; max 7 (deepest node must fit hn9)",
                depth
            ));
        }

        // 5: metadata field specs
        for (typ, fields) in &self.metadata {
            for (name, spec) in fields {
                validate_spec(spec).map_err(|msg| format!("{}.{}: {}", typ, name, msg))?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Metadata validation
// ---------------------------------------------------------------------------

/// Metadata validation error.
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataError {
    pub path: String,
    pub message: String,
}

/// Validate a single field spec's invariants.
///
/// Only `Enum { one_of = [] }` is currently rejected.
pub fn validate_spec(spec: &FieldSpec) -> Result<(), String> {
    match &spec.typ {
        FieldType::Enum { one_of } if one_of.is_empty() => {
            Err("enum must declare non-empty one_of".to_string())
        }
        _ => Ok(()),
    }
}

/// Validate a single value against a field spec.
///
/// - `Null` with `required=true` → error "required field missing"
/// - `Null` with `required=false` → Ok
/// - Type-specific checks follow.
pub fn validate_one(path: &str, spec: &FieldSpec, v: &serde_json::Value) -> Result<(), MetadataError> {
    use serde_json::Value;

    let err = |message: String| MetadataError {
        path: path.to_string(),
        message,
    };

    // Null handling
    if v.is_null() {
        return if spec.required {
            Err(err("required field missing".to_string()))
        } else {
            Ok(())
        };
    }

    match (&spec.typ, v) {
        (FieldType::String { min_len, max_len }, Value::String(s)) => {
            let len = s.chars().count() as i64;
            if let Some(n) = min_len {
                if len < *n {
                    return Err(err(format!("string too short (min {})", n)));
                }
            }
            if let Some(n) = max_len {
                if len > *n {
                    return Err(err(format!("string too long (max {})", n)));
                }
            }
            Ok(())
        }

        // Number field, JSON floats
        (FieldType::Number { min, max }, Value::Number(n)) if n.is_f64() => {
            let f = n.as_f64().unwrap();
            if let Some(lo) = min {
                if f < *lo {
                    return Err(err(format!("value below minimum {}", lo)));
                }
            }
            if let Some(hi) = max {
                if f > *hi {
                    return Err(err(format!("value above maximum {}", hi)));
                }
            }
            Ok(())
        }

        // Number field, JSON integers treated as floats
        (FieldType::Number { min, max }, Value::Number(n)) if n.is_i64() => {
            let f = n.as_i64().unwrap() as f64;
            if let Some(lo) = min {
                if f < *lo {
                    return Err(err(format!("value below minimum {}", lo)));
                }
            }
            if let Some(hi) = max {
                if f > *hi {
                    return Err(err(format!("value above maximum {}", hi)));
                }
            }
            Ok(())
        }

        // Integer field, JSON integers
        (FieldType::Integer { min, max }, Value::Number(n)) if n.is_i64() => {
            let iv = n.as_i64().unwrap();
            if let Some(lo) = min {
                if iv < *lo {
                    return Err(err("integer below minimum".to_string()));
                }
            }
            if let Some(hi) = max {
                if iv > *hi {
                    return Err(err("integer above maximum".to_string()));
                }
            }
            Ok(())
        }

        (FieldType::Boolean, Value::Bool(_)) => Ok(()),

        (FieldType::Timestamp, Value::String(s)) => {
            // Use chrono for RFC3339 parsing
            if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
                Ok(())
            } else {
                Err(err("invalid RFC3339 timestamp".to_string()))
            }
        }

        (FieldType::Enum { one_of }, Value::String(s)) => {
            if one_of.contains(s) {
                Ok(())
            } else {
                Err(err(format!("value not in enum: {}", s)))
            }
        }

        // Catch-all: type mismatch
        _ => Err(err("type mismatch".to_string())),
    }
}

/// Validate a JSON object against a set of field specs.
///
/// - If `v` is not a JSON object, return a single error with empty path.
/// - For each spec: if key is absent and required → error; if present → `validate_one`.
/// - Unknown keys in the JSON object are silently ignored.
/// - Returns all collected errors.
pub fn validate(
    specs: &[(String, FieldSpec)],
    v: &serde_json::Value,
) -> Result<(), Vec<MetadataError>> {
    use serde_json::Value;

    let kvs = match v {
        Value::Object(map) => map,
        _ => {
            return Err(vec![MetadataError {
                path: String::new(),
                message: "metadata must be a JSON object".to_string(),
            }]);
        }
    };

    let mut errs: Vec<MetadataError> = Vec::new();

    for (name, spec) in specs {
        match kvs.get(name) {
            None if spec.required => {
                errs.push(MetadataError {
                    path: name.clone(),
                    message: "required field missing".to_string(),
                });
            }
            None => {}
            Some(value) => {
                if let Err(e) = validate_one(name, spec, value) {
                    errs.push(e);
                }
            }
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

// ---------------------------------------------------------------------------
// Metadata coercion (form strings -> declared schema types)
// ---------------------------------------------------------------------------

/// Normalize a timestamp string to RFC3339, or return `None` if it is neither
/// RFC3339 nor a bare `YYYY-MM-DD` date.
fn normalize_timestamp(s: &str) -> Option<String> {
    let t = s.trim();
    if chrono::DateTime::parse_from_rfc3339(t).is_ok() {
        return Some(t.to_string());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Some(format!("{}T00:00:00Z", d.format("%Y-%m-%d")));
    }
    None
}

/// Coerce string-encoded form values to their declared schema types, in place.
///
/// Form bodies encode every value as a string; `validate` expects JSON of the
/// declared type. For each declared field present in the object:
/// - empty/whitespace string → the key is removed (blank optional field);
/// - `number`/`integer` string that parses → JSON number;
/// - `boolean` `"true"`/`"false"` → JSON bool;
/// - `timestamp` date-only (`YYYY-MM-DD`) → RFC3339 midnight UTC; RFC3339 kept;
/// - `string`/`enum` and already-typed values are left unchanged.
///
/// Keys are never added; undeclared keys are left untouched.
pub fn coerce_metadata(specs: &[(String, FieldSpec)], v: &mut serde_json::Value) {
    use serde_json::Value;
    let obj = match v.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    for (name, spec) in specs {
        let cur = match obj.get(name) {
            Some(x) => x.clone(),
            None => continue,
        };
        if let Value::String(s) = &cur {
            if s.trim().is_empty() {
                obj.remove(name);
                continue;
            }
        }
        let coerced: Option<Value> = match (&spec.typ, &cur) {
            (FieldType::Number { .. }, Value::String(s)) => s
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number),
            (FieldType::Integer { .. }, Value::String(s)) => {
                s.trim().parse::<i64>().ok().map(|i| Value::Number(i.into()))
            }
            (FieldType::Boolean, Value::String(s)) => match s.trim() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            (FieldType::Timestamp, Value::String(s)) => normalize_timestamp(s).map(Value::String),
            _ => None,
        };
        if let Some(c) = coerced {
            obj.insert(name.clone(), c);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::FieldType;
    use serde_json::json;

    // ---- coerce_metadata ---------------------------------------------------

    #[test]
    fn coerce_number_and_integer_from_string() {
        let specs: Vec<(String, FieldSpec)> = vec![
            ("lat".into(), FieldSpec { typ: FieldType::Number { min: None, max: None }, required: true }),
            ("n".into(),   FieldSpec { typ: FieldType::Integer { min: None, max: None }, required: false }),
        ];
        let mut v = json!({ "lat": "55.5", "n": "7" });
        coerce_metadata(&specs, &mut v);
        assert_eq!(v["lat"], json!(55.5));
        assert_eq!(v["n"], json!(7));
    }

    #[test]
    fn coerce_boolean_from_string() {
        let specs: Vec<(String, FieldSpec)> = vec![
            ("b".into(), FieldSpec { typ: FieldType::Boolean, required: false }),
        ];
        let mut v = json!({ "b": "true" });
        coerce_metadata(&specs, &mut v);
        assert_eq!(v["b"], json!(true));
    }

    #[test]
    fn coerce_date_to_rfc3339() {
        let specs: Vec<(String, FieldSpec)> = vec![
            ("ts".into(), FieldSpec { typ: FieldType::Timestamp, required: false }),
        ];
        let mut v = json!({ "ts": "2026-06-12" });
        coerce_metadata(&specs, &mut v);
        assert_eq!(v["ts"], json!("2026-06-12T00:00:00Z"));
        // a value already in RFC3339 is left intact
        let mut v2 = json!({ "ts": "2026-06-12T08:30:00Z" });
        coerce_metadata(&specs, &mut v2);
        assert_eq!(v2["ts"], json!("2026-06-12T08:30:00Z"));
    }

    #[test]
    fn coerce_empty_string_removes_key() {
        let specs: Vec<(String, FieldSpec)> = vec![
            ("lat".into(), FieldSpec { typ: FieldType::Number { min: None, max: None }, required: false }),
        ];
        let mut v = json!({ "lat": "  " });
        coerce_metadata(&specs, &mut v);
        assert!(v.as_object().unwrap().get("lat").is_none());
    }

    #[test]
    fn coerce_leaves_typed_and_undeclared_values() {
        let specs: Vec<(String, FieldSpec)> = vec![
            ("lat".into(), FieldSpec { typ: FieldType::Number { min: None, max: None }, required: false }),
        ];
        // already a number → untouched; undeclared key → untouched
        let mut v = json!({ "lat": 1.0, "other": "x" });
        coerce_metadata(&specs, &mut v);
        assert_eq!(v["lat"], json!(1.0));
        assert_eq!(v["other"], json!("x"));
    }

    // ---- helpers -----------------------------------------------------------

    fn sample_schema() -> Schema {
        Schema {
            version: 2,
            edges: vec![
                ("company".to_string(), vec![
                    ("group".to_string(), EdgeSpec::builder().build()),
                    ("property".to_string(), EdgeSpec::builder().build()),
                    ("building".to_string(), EdgeSpec::builder().build()),
                ]),
                ("group".to_string(), vec![
                    ("building".to_string(), EdgeSpec::builder().build()),
                ]),
                ("property".to_string(), vec![
                    ("building".to_string(), EdgeSpec::builder().build()),
                ]),
                ("building".to_string(), vec![
                    ("area".to_string(), EdgeSpec::builder().build()),
                ]),
            ],
            metadata: vec![(
                "building".to_string(),
                vec![
                    ("lat".to_string(), FieldSpec {
                        typ: FieldType::Number { min: Some(-90.0), max: Some(90.0) },
                        required: true,
                    }),
                    ("lng".to_string(), FieldSpec {
                        typ: FieldType::Number { min: Some(-180.0), max: Some(180.0) },
                        required: true,
                    }),
                ],
            )],
            sensors: vec!["building".to_string(), "area".to_string()],
        }
    }

    // ---- Schema::validate --------------------------------------------------

    #[test]
    fn self_check_accepts_sample() {
        sample_schema().validate().expect("sample schema must validate");
    }

    #[test]
    fn rejects_cycle() {
        let mut bad = sample_schema();
        // area -> group closes a cycle group -> building -> area -> group
        bad.edges.push((
            "area".to_string(),
            vec![("group".to_string(), EdgeSpec::builder().build())],
        ));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("cycle"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_self_edge() {
        let mut bad = sample_schema();
        bad.edges.push((
            "area".to_string(),
            vec![("area".to_string(), EdgeSpec::builder().build())],
        ));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("self edge"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_unreachable_parent() {
        let mut bad = sample_schema();
        bad.edges.push((
            "warehouse".to_string(),
            vec![("area".to_string(), EdgeSpec::builder().build())],
        ));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("not reachable"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_company_as_child() {
        let mut bad = sample_schema();
        bad.edges[1].1.push(("company".to_string(), EdgeSpec::builder().build()));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("reserved"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_partner_anywhere() {
        let mut bad = sample_schema();
        bad.edges.push((
            "partner".to_string(),
            vec![("building".to_string(), EdgeSpec::builder().build())],
        ));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("reserved"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_chain_deeper_than_hn9() {
        // company -> t1 -> t2 -> ... -> t8 = 8 edges (one too many)
        let mut edges: Vec<(String, Vec<(String, EdgeSpec)>)> = Vec::new();
        let mut parent = "company".to_string();
        for i in 1..=8 {
            let child = format!("t{}", i);
            edges.push((parent.clone(), vec![(child.clone(), EdgeSpec::builder().build())]));
            parent = child;
        }
        let bad = Schema { version: 2, edges, metadata: vec![], sensors: vec![] };
        let err = bad.validate().unwrap_err();
        assert!(err.contains("max 7"), "wrong error: {}", err);

        // exactly 7 edges is fine
        let mut edges: Vec<(String, Vec<(String, EdgeSpec)>)> = Vec::new();
        let mut parent = "company".to_string();
        for i in 1..=7 {
            let child = format!("t{}", i);
            edges.push((parent.clone(), vec![(child.clone(), EdgeSpec::builder().build())]));
            parent = child;
        }
        let ok = Schema { version: 2, edges, metadata: vec![], sensors: vec![] };
        ok.validate().expect("7-edge chain must validate");
    }

    #[test]
    fn rejects_duplicate_child_type() {
        let mut bad = sample_schema();
        bad.edges[1].1.push(("building".to_string(), EdgeSpec::builder().build()));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("duplicate child"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_min_greater_than_max() {
        let mut bad = sample_schema();
        bad.edges[3].1[0].1 = EdgeSpec::builder().min(Some(10)).max(Some(5)).build();
        let err = bad.validate().unwrap_err();
        assert!(err.contains("min > max"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_unreachable_sensor_and_metadata_types() {
        let mut bad = sample_schema();
        bad.sensors.push("warehouse".to_string());
        assert!(bad.validate().unwrap_err().contains("unreachable"));

        let mut bad2 = sample_schema();
        bad2.metadata.push(("warehouse".to_string(), vec![]));
        assert!(bad2.validate().unwrap_err().contains("unreachable"));
    }

    #[test]
    fn rejects_duplicate_sensor_entry() {
        let mut bad = sample_schema();
        bad.sensors.push("building".to_string());
        assert!(bad.validate().unwrap_err().contains("duplicate sensors"));
    }

    #[test]
    fn allowed_children_lookup() {
        let s = sample_schema();
        let kids = s.allowed_children("group");
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].0, "building");
        assert!(s.allowed_children("area").is_empty());
        assert_eq!(s.allowed_children("company").len(), 3);
    }

    #[test]
    fn edge_between_lookup() {
        let s = sample_schema();
        assert!(s.edge_between("company", "building").is_some());
        assert!(s.edge_between("group", "area").is_none());
    }

    #[test]
    fn metadata_for_lookup() {
        let s = sample_schema();
        assert_eq!(s.metadata_for("building").len(), 2);
        assert!(s.metadata_for("area").is_empty());
    }

    #[test]
    fn allows_sensors_lookup() {
        let s = sample_schema();
        assert!(s.allows_sensors("building"));
        assert!(s.allows_sensors("area"));
        assert!(!s.allows_sensors("group"));
    }

    // ---- validate_spec -----------------------------------------------------

    #[test]
    fn spec_rejects_empty_enum() {
        let spec = FieldSpec {
            typ: FieldType::Enum { one_of: vec![] },
            required: false,
        };
        match validate_spec(&spec) {
            Ok(()) => panic!("empty enum must be rejected"),
            Err(_) => {}
        }
    }

    #[test]
    fn spec_accepts_non_empty_enum() {
        let spec = FieldSpec {
            typ: FieldType::Enum {
                one_of: vec!["a".to_string()],
            },
            required: false,
        };
        assert!(validate_spec(&spec).is_ok());
    }

    // ---- validate (metadata against JSON) ----------------------------------

    #[test]
    fn validate_ok() {
        let specs: Vec<(String, FieldSpec)> = vec![
            (
                "lat".to_string(),
                FieldSpec {
                    typ: FieldType::Number {
                        min: Some(-90.0),
                        max: Some(90.0),
                    },
                    required: true,
                },
            ),
            (
                "lng".to_string(),
                FieldSpec {
                    typ: FieldType::Number {
                        min: Some(-180.0),
                        max: Some(180.0),
                    },
                    required: true,
                },
            ),
            (
                "name".to_string(),
                FieldSpec {
                    typ: FieldType::String {
                        min_len: Some(1),
                        max_len: None,
                    },
                    required: true,
                },
            ),
        ];
        let values = json!({
            "lat": 55.68,
            "lng": 12.57,
            "name": "Building A",
            "ignored_unknown": "ok"
        });
        match validate(&specs, &values) {
            Ok(()) => {}
            Err(errs) => panic!("expected Ok, got {} errors", errs.len()),
        }
    }

    #[test]
    fn validate_missing_required() {
        let specs: Vec<(String, FieldSpec)> = vec![(
            "lat".to_string(),
            FieldSpec {
                typ: FieldType::Number {
                    min: None,
                    max: None,
                },
                required: true,
            },
        )];
        let values = json!({});
        match validate(&specs, &values) {
            Ok(()) => panic!("expected Error on missing required"),
            Err(errs) if errs.len() == 1 => {
                assert_eq!(errs[0].path, "lat");
                assert!(
                    errs[0].message.contains("required"),
                    "message should mention required: {}",
                    errs[0].message
                );
            }
            Err(errs) => panic!("expected exactly 1 error, got {}", errs.len()),
        }
    }

    #[test]
    fn validate_number_out_of_range() {
        let specs: Vec<(String, FieldSpec)> = vec![(
            "lat".to_string(),
            FieldSpec {
                typ: FieldType::Number {
                    min: Some(-90.0),
                    max: Some(90.0),
                },
                required: true,
            },
        )];
        let values = json!({ "lat": 200.0 });
        match validate(&specs, &values) {
            Ok(()) => panic!("expected Error on out-of-range"),
            Err(_) => {}
        }
    }

    #[test]
    fn validate_enum() {
        let specs: Vec<(String, FieldSpec)> = vec![(
            "kind".to_string(),
            FieldSpec {
                typ: FieldType::Enum {
                    one_of: vec!["ccs".to_string(), "type2".to_string()],
                },
                required: true,
            },
        )];

        // Bad enum value
        match validate(&specs, &json!({ "kind": "chademo" })) {
            Ok(()) => panic!("expected Error on bad enum"),
            Err(_) => {}
        }

        // Good enum value
        match validate(&specs, &json!({ "kind": "ccs" })) {
            Ok(()) => {}
            Err(_) => panic!("expected Ok on good enum"),
        }
    }

    // ---- validate_one edge cases -------------------------------------------

    /// Null with required=false is Ok.
    #[test]
    fn validate_one_null_optional() {
        let spec = FieldSpec {
            typ: FieldType::Boolean,
            required: false,
        };
        assert!(validate_one("f", &spec, &json!(null)).is_ok());
    }

    /// Null with required=true is Err.
    #[test]
    fn validate_one_null_required() {
        let spec = FieldSpec {
            typ: FieldType::Boolean,
            required: true,
        };
        assert!(validate_one("f", &spec, &json!(null)).is_err());
    }

    /// Integer below minimum.
    #[test]
    fn validate_one_integer_below_min() {
        let spec = FieldSpec {
            typ: FieldType::Integer {
                min: Some(0),
                max: Some(100),
            },
            required: false,
        };
        assert!(validate_one("n", &spec, &json!(-1)).is_err());
    }

    /// Integer above maximum.
    #[test]
    fn validate_one_integer_above_max() {
        let spec = FieldSpec {
            typ: FieldType::Integer {
                min: Some(0),
                max: Some(100),
            },
            required: false,
        };
        assert!(validate_one("n", &spec, &json!(101)).is_err());
    }

    /// Integer in range.
    #[test]
    fn validate_one_integer_in_range() {
        let spec = FieldSpec {
            typ: FieldType::Integer {
                min: Some(0),
                max: Some(100),
            },
            required: false,
        };
        assert!(validate_one("n", &spec, &json!(50)).is_ok());
    }

    /// String too short.
    #[test]
    fn validate_one_string_too_short() {
        let spec = FieldSpec {
            typ: FieldType::String {
                min_len: Some(5),
                max_len: None,
            },
            required: false,
        };
        assert!(validate_one("s", &spec, &json!("hi")).is_err());
    }

    /// String too long.
    #[test]
    fn validate_one_string_too_long() {
        let spec = FieldSpec {
            typ: FieldType::String {
                min_len: None,
                max_len: Some(3),
            },
            required: false,
        };
        assert!(validate_one("s", &spec, &json!("hello")).is_err());
    }

    /// Valid RFC3339 timestamp.
    #[test]
    fn validate_one_timestamp_valid() {
        let spec = FieldSpec {
            typ: FieldType::Timestamp,
            required: false,
        };
        assert!(validate_one("ts", &spec, &json!("2024-01-15T10:30:00Z")).is_ok());
    }

    /// Invalid RFC3339 timestamp.
    #[test]
    fn validate_one_timestamp_invalid() {
        let spec = FieldSpec {
            typ: FieldType::Timestamp,
            required: false,
        };
        assert!(validate_one("ts", &spec, &json!("not-a-timestamp")).is_err());
    }

    /// Type mismatch: boolean spec but string value.
    #[test]
    fn validate_one_type_mismatch() {
        let spec = FieldSpec {
            typ: FieldType::Boolean,
            required: false,
        };
        match validate_one("b", &spec, &json!("true")) {
            Err(e) => assert_eq!(e.message, "type mismatch"),
            Ok(()) => panic!("expected type mismatch error"),
        }
    }

    /// Non-object passed to validate → error with empty path.
    #[test]
    fn validate_non_object() {
        let specs = vec![];
        match validate(&specs, &json!("not an object")) {
            Err(errs) if errs.len() == 1 => {
                assert_eq!(errs[0].path, "");
                assert!(errs[0].message.contains("JSON object"));
            }
            _ => panic!("expected exactly 1 error for non-object input"),
        }
    }

    /// Number field accepts JSON integer values (integer → float coercion).
    #[test]
    fn validate_one_number_accepts_integer_json() {
        let spec = FieldSpec {
            typ: FieldType::Number {
                min: Some(0.0),
                max: Some(100.0),
            },
            required: false,
        };
        // JSON integer within range
        assert!(validate_one("n", &spec, &json!(42)).is_ok());
        // JSON integer out of range
        assert!(validate_one("n", &spec, &json!(200)).is_err());
    }
}
