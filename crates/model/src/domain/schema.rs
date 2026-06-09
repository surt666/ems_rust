#![allow(clippy::type_complexity)]

use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::ids::Level;
use crate::domain::values::FieldType;

// ---------------------------------------------------------------------------
// EdgeSpec
// ---------------------------------------------------------------------------

/// Specification for a directed hierarchy edge (label + optional cardinality).
///
/// Faithfully ported from `services/hierarchy/lib/domain/schema.ml`:
/// `type edge_spec = { label : string; min : int option; max : int option }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TypedBuilder)]
pub struct EdgeSpec {
    pub label: String,
    #[builder(default)]
    pub min: Option<i32>,
    #[builder(default)]
    pub max: Option<i32>,
}

// ---------------------------------------------------------------------------
// FieldSpec
// ---------------------------------------------------------------------------

/// A metadata field specification: its type + whether it is required.
///
/// Faithfully ported from `services/hierarchy/lib/domain/metadata.ml`:
/// `type field_spec = { typ : field_type; required : bool }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldSpec {
    pub typ: FieldType,
    pub required: bool,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// A hierarchy schema: versioned edge rules, metadata specs, and sensor levels.
///
/// Faithfully ported from `services/hierarchy/lib/domain/schema.ml`:
/// `type t = { version; edges; metadata; sensors }`.
///
/// The `edges` and `metadata` fields use association-list structure matching OCaml's
/// `(Level.t * (Level.t * edge_spec list) list) list` / `(Level.t * (string * field_spec) list) list`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TypedBuilder)]
pub struct Schema {
    pub version: u32,
    /// Association list: parent level → list of (child level → edge specs).
    #[allow(clippy::type_complexity)]
    pub edges: Vec<(Level, Vec<(Level, Vec<EdgeSpec>)>)>,
    /// Association list: level → list of (field name → field spec).
    pub metadata: Vec<(Level, Vec<(String, FieldSpec)>)>,
    /// Levels at which sensors are allowed.
    pub sensors: Vec<Level>,
}

impl Schema {
    /// Returns the allowed child entries for `parent`, or an empty slice.
    ///
    /// Port of OCaml `allowed_children`:
    /// `List.assoc_opt parent t.edges |> Option.value ~default:[]`.
    pub fn allowed_children(&self, parent: Level) -> &[(Level, Vec<EdgeSpec>)] {
        self.edges
            .iter()
            .find(|(lvl, _)| *lvl == parent)
            .map(|(_, children)| children.as_slice())
            .unwrap_or(&[])
    }

    /// Returns the edge specs between `parent` and `child`, or empty Vec.
    ///
    /// Port of OCaml `edges_between`:
    /// `List.assoc_opt child (allowed_children t parent) |> Option.value ~default:[]`.
    pub fn edges_between(&self, parent: Level, child: Level) -> Vec<EdgeSpec> {
        self.allowed_children(parent)
            .iter()
            .find(|(lvl, _)| *lvl == child)
            .map(|(_, specs)| specs.clone())
            .unwrap_or_default()
    }

    /// Returns the metadata field specs for `level`, or an empty slice.
    ///
    /// Port of OCaml `metadata_for`:
    /// `List.assoc_opt level t.metadata |> Option.value ~default:[]`.
    pub fn metadata_for(&self, level: Level) -> &[(String, FieldSpec)] {
        self.metadata
            .iter()
            .find(|(lvl, _)| *lvl == level)
            .map(|(_, fields)| fields.as_slice())
            .unwrap_or(&[])
    }

    /// Returns whether `level` allows sensors.
    ///
    /// Port of OCaml `allows_sensors`: `List.mem level t.sensors`.
    pub fn allows_sensors(&self, level: Level) -> bool {
        self.sensors.contains(&level)
    }

    /// Validate schema invariants.
    ///
    /// Port of OCaml `validate`:
    /// 1. Each edge `parent → child` must have `depth(parent) < depth(child)`.
    /// 2. Within an edge list, labels must be unique.
    /// 3. If both `min` and `max` are set on a spec, `min <= max`.
    /// 4. Each metadata field spec must pass `validate_spec`.
    /// 5. The `sensors` list must have no duplicate levels.
    pub fn validate(&self) -> Result<(), String> {
        // 1–3: edge ordering, duplicate labels, min <= max
        for (parent, children) in &self.edges {
            for (child, specs) in children {
                if parent.depth() >= child.depth() {
                    return Err(format!(
                        "edge {} -> {} violates depth ordering",
                        parent, child
                    ));
                }
                let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
                for spec in specs {
                    if !seen.insert(spec.label.as_str()) {
                        return Err(format!(
                            "edge {} -> {}: duplicate label {:?}",
                            parent, child, spec.label
                        ));
                    }
                    if let (Some(a), Some(b)) = (spec.min, spec.max) {
                        if a > b {
                            return Err(format!(
                                "edge {} -> {} ({}) has min > max",
                                parent, child, spec.label
                            ));
                        }
                    }
                }
            }
        }

        // 4: metadata field-spec validation
        for (level, fields) in &self.metadata {
            for (name, spec) in fields {
                validate_spec(spec).map_err(|msg| {
                    format!("{}.{}: {}", level, name, msg)
                })?;
            }
        }

        // 5: no duplicate sensor levels
        let mut seen: std::collections::HashSet<Level> = std::collections::HashSet::new();
        for level in &self.sensors {
            if !seen.insert(*level) {
                return Err(format!(
                    "duplicate sensors entry for level {}",
                    level
                ));
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Metadata validation (ported from metadata.ml)
// ---------------------------------------------------------------------------

/// Metadata validation error.
///
/// Port of OCaml `type error = { path : string; message : string }`.
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataError {
    pub path: String,
    pub message: String,
}

/// Validate a single field spec's invariants.
///
/// Port of OCaml `Metadata.validate_spec`:
/// only `Enum { one_of = [] }` is currently rejected.
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
/// Port of OCaml `Metadata.validate_one`.
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

    // Null handling (matches first arm in OCaml)
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

        // OCaml: `Number { min; max }, `Float f` — JSON floats
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

        // OCaml: `Number { min; max }, `Int i` — JSON integers treated as floats
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

        // OCaml: `Integer { min; max }, `Int i`
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
            // Port of OCaml `is_rfc3339`: parse via Ptime.of_rfc3339
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
/// Port of OCaml `Metadata.validate`:
/// - If `v` is not a JSON object, return a single error with empty path.
/// - For each spec: if key is absent and required → error; if present → `validate_one`.
/// - Unknown keys in the JSON object are silently ignored (OCaml: `List.assoc_opt` iterates specs).
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::Level;
    use crate::domain::values::FieldType;
    use serde_json::json;

    // ---- helpers -----------------------------------------------------------

    fn sample_schema() -> Schema {
        Schema {
            version: 1,
            edges: vec![
                (
                    Level::Hn2,
                    vec![(
                        Level::Hn3,
                        vec![EdgeSpec::builder().label("property".to_string()).build()],
                    )],
                ),
                (
                    Level::Hn3,
                    vec![(
                        Level::Hn4,
                        vec![EdgeSpec::builder()
                            .label("building".to_string())
                            .min(Some(1))
                            .build()],
                    )],
                ),
                (
                    Level::Hn4,
                    vec![(
                        Level::Hn5,
                        vec![EdgeSpec::builder().label("area".to_string()).build()],
                    )],
                ),
            ],
            metadata: vec![(
                Level::Hn4,
                vec![
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
                ],
            )],
            sensors: vec![],
        }
    }

    // ---- Schema::validate --------------------------------------------------

    /// Port of `test_domain_schema.ml :: self_check_accepts_sample`
    #[test]
    fn self_check_accepts_sample() {
        let s = sample_schema();
        match s.validate() {
            Ok(()) => {}
            Err(e) => panic!("unexpected validation error: {}", e),
        }
    }

    /// Port of `test_domain_schema.ml :: rejects_depth_violation`
    #[test]
    fn rejects_depth_violation() {
        let mut bad = sample_schema();
        bad.edges = vec![(
            Level::Hn4,
            vec![(
                Level::Hn3,
                vec![EdgeSpec::builder().label("x".to_string()).build()],
            )],
        )];
        match bad.validate() {
            Ok(()) => panic!("expected failure on hn4 -> hn3"),
            Err(_) => {}
        }
    }

    /// Port of `test_domain_schema.ml :: rejects_duplicate_sensor_level`
    #[test]
    fn rejects_duplicate_sensor_level() {
        let mut s = sample_schema();
        s.sensors = vec![Level::Hn4, Level::Hn4];
        match s.validate() {
            Ok(()) => panic!("expected duplicate level error"),
            Err(_) => {}
        }
    }

    /// Additional: duplicate edge label within same parent→child arc.
    #[test]
    fn rejects_duplicate_edge_label() {
        let mut bad = sample_schema();
        bad.edges = vec![(
            Level::Hn2,
            vec![(
                Level::Hn3,
                vec![
                    EdgeSpec::builder().label("dup".to_string()).build(),
                    EdgeSpec::builder().label("dup".to_string()).build(),
                ],
            )],
        )];
        match bad.validate() {
            Ok(()) => panic!("expected duplicate label error"),
            Err(e) => assert!(e.contains("duplicate label"), "wrong error: {}", e),
        }
    }

    /// Additional: min > max on an edge spec.
    #[test]
    fn rejects_min_greater_than_max() {
        let mut bad = sample_schema();
        bad.edges = vec![(
            Level::Hn2,
            vec![(
                Level::Hn3,
                vec![EdgeSpec::builder()
                    .label("x".to_string())
                    .min(Some(10))
                    .max(Some(5))
                    .build()],
            )],
        )];
        match bad.validate() {
            Ok(()) => panic!("expected min > max error"),
            Err(e) => assert!(e.contains("min > max"), "wrong error: {}", e),
        }
    }

    // ---- Schema query methods ----------------------------------------------

    /// Port of `test_domain_schema.ml :: allowed_children_lookup`
    #[test]
    fn allowed_children_lookup() {
        let s = sample_schema();
        let kids = s.allowed_children(Level::Hn3);
        assert_eq!(kids.len(), 1, "one child level");
        let (level, specs) = &kids[0];
        assert_eq!(level.to_string(), "hn4", "child is hn4");
        assert_eq!(specs.len(), 1, "one label");
        assert_eq!(specs[0].label, "building");
    }

    /// Port of `test_domain_schema.ml :: metadata_for_lookup`
    #[test]
    fn metadata_for_lookup() {
        let s = sample_schema();
        let md = s.metadata_for(Level::Hn4);
        assert_eq!(md.len(), 2, "two fields");
        let md_none = s.metadata_for(Level::Hn5);
        assert_eq!(md_none.len(), 0, "no fields at hn5");
    }

    /// Port of `test_domain_schema.ml :: allows_sensors_lookup`
    #[test]
    fn allows_sensors_lookup() {
        let mut s = sample_schema();
        s.sensors = vec![Level::Hn4];
        assert!(s.allows_sensors(Level::Hn4), "allowed at hn4");
        assert!(!s.allows_sensors(Level::Hn3), "not allowed at hn3");
    }

    /// edges_between returns empty for unknown pair.
    #[test]
    fn edges_between_unknown() {
        let s = sample_schema();
        let specs = s.edges_between(Level::Hn2, Level::Hn9);
        assert!(specs.is_empty());
    }

    /// edges_between returns correct specs for known pair.
    #[test]
    fn edges_between_known() {
        let s = sample_schema();
        let specs = s.edges_between(Level::Hn3, Level::Hn4);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].label, "building");
        assert_eq!(specs[0].min, Some(1));
        assert_eq!(specs[0].max, None);
    }

    // ---- validate_spec -----------------------------------------------------

    /// Port of `test_domain_metadata.ml :: spec rejects empty enum`
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

    /// Port of `test_domain_metadata.ml :: validate ok`
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

    /// Port of `test_domain_metadata.ml :: missing required`
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

    /// Port of `test_domain_metadata.ml :: number out of range`
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

    /// Port of `test_domain_metadata.ml :: enum good/bad`
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

    /// Number field accepts JSON integer values (OCaml `Int i` → float coercion).
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
