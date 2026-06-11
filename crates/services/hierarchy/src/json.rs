//! JSON (de)serialisation for the hierarchy API.
//!
//! This module is the single authoritative place for the JSON contract that
//! the hierarchy API exposes.  `dispatch.rs` and `query.rs` call into here
//! rather than duplicating the logic.

use serde_json::{json, Value};

use model::domain::formula::{expr_aliases, expr_to_string, parse_expr_str, Formula};
use model::domain::ids::{NodeId, SensorId};
use model::domain::node::Node;
use model::domain::schema::{EdgeSpec, FieldSpec, Schema};
use model::domain::sensor::Sensor;
use model::domain::user::User;
use model::domain::values::FieldType;

// ---------------------------------------------------------------------------
// error_body
// ---------------------------------------------------------------------------

/// Build a JSON error body `{ "error": { "code": …, "message": … } }` and
/// serialise to string.  `code` and `message` are caller-supplied.
#[cfg(test)]
pub fn error_body(code: &str, message: &str) -> String {
    let j = json!({
        "error": {
            "code":    code,
            "message": message,
        }
    });
    j.to_string()
}

// ---------------------------------------------------------------------------
// node_ref_to_json
// ---------------------------------------------------------------------------

/// Serialise an `(id, name)` pair used in node-child listings.
pub fn node_ref_to_json(id: &NodeId, name: &str) -> Value {
    json!({
        "id":   id.to_string(),
        "name": name,
    })
}

// ---------------------------------------------------------------------------
// schema_to_json / schema_of_json
// ---------------------------------------------------------------------------

/// Serialise a `Schema` (v2) to a `serde_json::Value`.
///
/// Shape: `{"version":2, "edges":{"company":{"building":{"min":1}}}, "metadata":{...}, "sensors":[...]}`
pub fn schema_to_json(sch: &Schema) -> Value {
    let edges_obj: serde_json::Map<String, Value> = sch
        .edges
        .iter()
        .map(|(parent, children)| {
            let inner_obj: serde_json::Map<String, Value> = children
                .iter()
                .map(|(child, spec)| {
                    let mut kv = serde_json::Map::new();
                    if let Some(m) = spec.min {
                        kv.insert("min".to_string(), json!(m));
                    }
                    if let Some(m) = spec.max {
                        kv.insert("max".to_string(), json!(m));
                    }
                    (child.clone(), Value::Object(kv))
                })
                .collect();
            (parent.clone(), Value::Object(inner_obj))
        })
        .collect();

    let metadata_obj: serde_json::Map<String, Value> = sch
        .metadata
        .iter()
        .map(|(typ, fields)| {
            let field_obj: serde_json::Map<String, Value> = fields
                .iter()
                .map(|(name, fs)| (name.clone(), field_spec_to_json(fs)))
                .collect();
            (typ.clone(), Value::Object(field_obj))
        })
        .collect();

    let sensors_arr: Vec<Value> =
        sch.sensors.iter().map(|t| Value::String(t.clone())).collect();

    json!({
        "version":  sch.version,
        "edges":    Value::Object(edges_obj),
        "metadata": Value::Object(metadata_obj),
        "sensors":  sensors_arr,
    })
}

/// Serialise a single `FieldSpec`.
fn field_spec_to_json(fs: &FieldSpec) -> Value {
    let mut kv: serde_json::Map<String, Value> = serde_json::Map::new();
    kv.insert("required".to_string(), Value::Bool(fs.required));
    match &fs.typ {
        FieldType::String { min_len, max_len } => {
            kv.insert("type".to_string(), json!("string"));
            if let Some(v) = min_len {
                kv.insert("min_len".to_string(), json!(v));
            }
            if let Some(v) = max_len {
                kv.insert("max_len".to_string(), json!(v));
            }
        }
        FieldType::Number { min, max } => {
            kv.insert("type".to_string(), json!("number"));
            if let Some(v) = min {
                kv.insert("min".to_string(), json!(v));
            }
            if let Some(v) = max {
                kv.insert("max".to_string(), json!(v));
            }
        }
        FieldType::Integer { min, max } => {
            kv.insert("type".to_string(), json!("integer"));
            if let Some(v) = min {
                kv.insert("min".to_string(), json!(v));
            }
            if let Some(v) = max {
                kv.insert("max".to_string(), json!(v));
            }
        }
        FieldType::Boolean => {
            kv.insert("type".to_string(), json!("boolean"));
        }
        FieldType::Timestamp => {
            kv.insert("type".to_string(), json!("timestamp"));
        }
        FieldType::Enum { one_of } => {
            kv.insert("type".to_string(), json!("enum"));
            let arr: Vec<Value> = one_of.iter().map(|s| Value::String(s.clone())).collect();
            kv.insert("one_of".to_string(), Value::Array(arr));
        }
    }
    Value::Object(kv)
}

/// Deserialise a `Schema` from a `serde_json::Value`. Only version 2 is accepted.
pub fn schema_of_json(v: &Value) -> Result<Schema, String> {
    let kvs = as_object(v)?;

    let version: u32 = match kvs.get("version") {
        Some(Value::Number(n)) if n.is_u64() => n.as_u64().unwrap() as u32,
        Some(Value::Number(n)) if n.is_i64() => {
            let i = n.as_i64().unwrap();
            if i < 0 { return Err("bad version".to_string()); }
            i as u32
        }
        _ => return Err("missing or non-integer field \"version\"".to_string()),
    };
    if version != 2 {
        return Err(format!(
            "unsupported schema version {}; expected 2 (type-keyed)",
            version
        ));
    }

    let edges_v = kvs.get("edges").ok_or_else(|| "missing field \"edges\"".to_string())?;
    let edges_map = as_object(edges_v)?;
    let mut edges: Vec<(String, Vec<(String, EdgeSpec)>)> = Vec::new();
    for (parent, inner_v) in edges_map {
        let inner_map = as_object(inner_v)?;
        let mut children: Vec<(String, EdgeSpec)> = Vec::new();
        for (child, body) in inner_map {
            let body_map = as_object(body)?;
            let min = body_map.get("min").and_then(opt_i32_of_json);
            let max = body_map.get("max").and_then(opt_i32_of_json);
            children.push((child.clone(), EdgeSpec { min, max }));
        }
        edges.push((parent.clone(), children));
    }

    let metadata: Vec<(String, Vec<(String, FieldSpec)>)> = match kvs.get("metadata") {
        None => vec![],
        Some(m_v) => {
            let m_map = as_object(m_v)?;
            let mut result = Vec::new();
            for (typ, inner_v) in m_map {
                let inner_map = as_object(inner_v)?;
                let mut fields: Vec<(String, FieldSpec)> = Vec::new();
                for (fname, spec_v) in inner_map {
                    fields.push((fname.clone(), decode_field_spec(spec_v)?));
                }
                result.push((typ.clone(), fields));
            }
            result
        }
    };

    let sensors: Vec<String> = match kvs.get("sensors") {
        None => vec![],
        Some(sensors_v) => {
            let arr = as_array(sensors_v)?;
            arr.iter()
                .map(|item| Ok(as_string(item)?.to_string()))
                .collect::<Result<Vec<_>, String>>()?
        }
    };

    Ok(Schema { version, edges, metadata, sensors })
}

/// Decode a single `FieldSpec` from a JSON value.
fn decode_field_spec(v: &Value) -> Result<FieldSpec, String> {
    let kvs = as_object(v)?;
    let typ_s = match kvs.get("type") {
        Some(Value::String(s)) => s.as_str(),
        _ => return Err("missing or non-string field \"type\"".to_string()),
    };
    let required = match kvs.get("required") {
        Some(Value::Bool(b)) => *b,
        _ => false,
    };
    let typ = match typ_s {
        "string" => FieldType::String {
            min_len: kvs.get("min_len").and_then(opt_i64_of_json),
            max_len: kvs.get("max_len").and_then(opt_i64_of_json),
        },
        "number" => FieldType::Number {
            min: kvs.get("min").and_then(opt_f64_of_json),
            max: kvs.get("max").and_then(opt_f64_of_json),
        },
        "integer" => FieldType::Integer {
            min: kvs.get("min").and_then(opt_i64_of_json),
            max: kvs.get("max").and_then(opt_i64_of_json),
        },
        "boolean" => FieldType::Boolean,
        "timestamp" => FieldType::Timestamp,
        "enum" => {
            let one_of_v = kvs
                .get("one_of")
                .ok_or_else(|| "enum requires \"one_of\"".to_string())?;
            let arr = as_array(one_of_v)?;
            let vals: Vec<String> = arr
                .iter()
                .map(|item| Ok(as_string(item)?.to_string()))
                .collect::<Result<Vec<_>, String>>()?;
            FieldType::Enum { one_of: vals }
        }
        other => return Err(format!("unknown field type {:?}", other)),
    };
    Ok(FieldSpec { typ, required })
}

// ---------------------------------------------------------------------------
// node_to_json
// ---------------------------------------------------------------------------

/// Serialise a `Node` to a `serde_json::Value`.
/// Keys: `id`, `name`, `parent` (null if root), `created` (RFC3339 `Z`),
/// `metadata`; plus `schema` when present.
pub fn node_to_json(n: &Node) -> Value {
    let parent = n
        .parent
        .as_ref()
        .map(|p| Value::String(p.to_string()))
        .unwrap_or(Value::Null);
    let created = n.created.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), Value::String(n.id.to_string()));
    map.insert("name".to_string(), Value::String(n.name.clone()));
    map.insert("parent".to_string(), parent);
    map.insert("created".to_string(), Value::String(created));
    map.insert("label".to_string(), Value::String(n.label.clone()));
    map.insert("metadata".to_string(), n.metadata.clone());
    if let Some(sch) = &n.schema {
        map.insert("schema".to_string(), schema_to_json(sch));
    }
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// formula_to_json
// ---------------------------------------------------------------------------

/// Serialise a `Formula` to a `serde_json::Value`.
pub fn formula_to_json(f: &Formula) -> Value {
    match f {
        Formula::Identity => json!({ "kind": "identity" }),
        Formula::Zero => json!({ "kind": "zero" }),
        Formula::Expr { refs, expr } => {
            let refs_obj: serde_json::Map<String, Value> = refs
                .iter()
                .map(|(a, sid)| (a.clone(), Value::String(sid.to_string())))
                .collect();
            json!({
                "kind": "expr",
                "expr": expr_to_string(expr),
                "refs": Value::Object(refs_obj),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// formula_of_json
// ---------------------------------------------------------------------------

/// Deserialise a `Formula` from an optional `serde_json::Value`.
///
/// Behaviour:
/// - `None` or `Some(null)` → `Identity`
/// - `Some("identity")` → `Identity`
/// - `Some("zero")` → `Zero`
/// - `Some({ "kind": "identity" })` → `Identity`
/// - `Some({ "kind": "zero" })` → `Zero`
/// - `Some({ "kind": "expr", "expr": …, "refs": … })` → `Expr`
/// - `Some({ "expr": …, "refs": … })` (no `kind`) → implicit `Expr`
/// - `refs` may be a JSON object or a JSON-encoded string.
/// - Unbound alias → `Err`; unused ref → `Err`.
pub fn formula_of_json(v: Option<&Value>) -> Result<Formula, String> {
    match v {
        None | Some(Value::Null) => Ok(Formula::Identity),
        Some(Value::String(s)) if s == "identity" => Ok(Formula::Identity),
        Some(Value::String(s)) if s == "zero" => Ok(Formula::Zero),
        Some(Value::Object(map)) => {
            let kind = map.get("kind").and_then(|v| v.as_str());
            let has_expr = map.get("expr").and_then(|v| v.as_str()).is_some();
            // "implicit expr": no kind but has expr field
            let want_expr = kind == Some("expr") || (kind.is_none() && has_expr);
            match kind {
                Some("identity") => Ok(Formula::Identity),
                Some("zero") => Ok(Formula::Zero),
                _ if want_expr => {
                    let expr_s = map
                        .get("expr")
                        .and_then(|v| v.as_str())
                        .ok_or("formula.expr must be a string")?;
                    let ast = parse_expr_str(expr_s)
                        .map_err(|e| format!("formula.expr parse error: {}", e))?;
                    let refs_val = map
                        .get("refs")
                        .cloned()
                        .unwrap_or(Value::Null);
                    let refs = parse_refs_json(&refs_val)?;
                    let aliases = expr_aliases(&ast);
                    // Every alias used in the expression must appear in refs.
                    if let Some(a) = aliases.iter().find(|a| !refs.iter().any(|(r, _)| r == *a)) {
                        return Err(format!("formula references unbound alias {:?}", a));
                    }
                    // Every ref must be used by the expression.
                    if let Some((a, _)) = refs.iter().find(|(a, _)| !aliases.contains(a)) {
                        return Err(format!("formula.refs has unused alias {:?}", a));
                    }
                    Ok(Formula::Expr { refs, expr: ast })
                }
                _ => Err(
                    "formula object must have a \"kind\" field (identity|zero|expr)".to_string(),
                ),
            }
        }
        Some(_) => Err("formula must be an object or string".to_string()),
    }
}

/// Parse the `refs` field of a formula.  Accepts:
/// - JSON object (`{"a": "S#1", …}`)
/// - JSON null → empty list
/// - A JSON-encoded string containing an object (`'{"a":"S#1"}'`)
fn parse_refs_json(v: &Value) -> Result<Vec<(String, SensorId)>, String> {
    let kvs: Vec<(String, Value)> = match v {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Value::Null => vec![],
        Value::String(s) => {
            // refs may be a JSON-serialised string (DynamoDB / form path).
            match serde_json::from_str::<Value>(s) {
                Ok(Value::Object(map)) => {
                    map.into_iter().collect()
                }
                Ok(_) => {
                    return Err("formula.refs string must encode a JSON object".to_string())
                }
                Err(_) => return Err("formula.refs is not valid JSON".to_string()),
            }
        }
        _ => return Err("formula.refs must be an object".to_string()),
    };
    kvs.into_iter()
        .map(|(alias, id_val)| {
            let id_s = id_val
                .as_str()
                .ok_or_else(|| format!("ref for alias {:?} must be a string sensor id", alias))?;
            let sid = SensorId::parse(id_s)
                .map_err(|e| format!("bad sensor id for alias {:?}: {}", alias, e))?;
            Ok((alias, sid))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// sensor_to_json
// ---------------------------------------------------------------------------

/// Serialise a `Sensor` to a `serde_json::Value`.
pub fn sensor_to_json(s: &Sensor) -> Value {
    let unit = s.unit.as_ref().map(|u| Value::String(u.clone())).unwrap_or(Value::Null);
    let resample = s.resample_minutes.map(|v| json!(v)).unwrap_or(Value::Null);
    let created = s.created.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    json!({
        "id":               s.id.to_string(),
        "created":          created,
        "daq_id":           s.daq_id,
        "path":             s.path,
        "purpose":          s.purpose,
        "meter_type":       s.meter_type.to_string(),
        "unit":             unit,
        "resample_minutes": resample,
        "formula":          formula_to_json(&s.formula),
    })
}

// ---------------------------------------------------------------------------
// user_to_json
// ---------------------------------------------------------------------------

/// Serialise a `User` to a `serde_json::Value`.
pub fn user_to_json(u: &User) -> Value {
    let created = u.created.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    json!({
        "id":            u.id.to_string(),
        "email":         u.email,
        "name":          u.name,
        "cognito_group": u.cognito_group.to_string(),
        "language":      u.language.to_string(),
        "currency":      u.currency.to_string(),
        "created":       created,
    })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn as_object(v: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    v.as_object().ok_or_else(|| "expected object".to_string())
}

fn as_array(v: &Value) -> Result<&Vec<Value>, String> {
    v.as_array().ok_or_else(|| "expected array".to_string())
}

fn as_string(v: &Value) -> Result<&str, String> {
    v.as_str().ok_or_else(|| "expected string".to_string())
}

fn opt_i32_of_json(v: &Value) -> Option<i32> {
    match v {
        Value::Number(n) => n.as_i64().map(|i| i as i32),
        _ => None,
    }
}

fn opt_i64_of_json(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

fn opt_f64_of_json(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}


// ---------------------------------------------------------------------------
// Tests (13 cases)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use model::domain::formula::{parse_expr_str as parse_expr, Formula};
    use model::domain::ids::{Level, NodeId, SensorId};
    use model::domain::node;
    use model::domain::sensor::Sensor;
    use model::domain::values::MeterType;
    use serde_json::json;

    // ------------------------------------------------------------------
    // 1. error_body_has_expected_shape
    //
    // Test: build an error body with code "bad_request" and message "boom",
    // checks code == "bad_request" and message == "boom".
    // ------------------------------------------------------------------
    #[test]
    fn error_body_has_expected_shape() {
        let body = error_body("bad_request", "boom");
        let v: Value = serde_json::from_str(&body).unwrap();
        let code = v["error"]["code"].as_str().expect("code must be string");
        let msg = v["error"]["message"].as_str().expect("message must be string");
        assert_eq!(code, "bad_request");
        assert_eq!(msg, "boom");
    }

    // ------------------------------------------------------------------
    // 2. node_to_json_has_expected_keys
    //
    // Test: creates a Node at Hn4 with parent Hn3,
    // calls node_to_json, checks for keys id/name/parent/created/metadata.
    // ------------------------------------------------------------------
    #[test]
    fn node_to_json_has_expected_keys() {
        let n = node::make(
            10004,
            Level::Hn4,
            "X",
            NodeId::make(Level::Hn3, 10003),
            &NodeId::root().to_string(),
            json!({ "lat": 55.0 }),
            None,
        );
        let j = node_to_json(&n);
        let keys: Vec<&str> = j
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        for k in &["id", "name", "parent", "created", "label", "metadata"] {
            assert!(keys.contains(k), "missing key {}", k);
        }
    }

    // ------------------------------------------------------------------
    // 3. schema_json_v2_roundtrip
    //
    // Parses a v2 JSON schema, checks key fields, then roundtrips back.
    // ------------------------------------------------------------------
    #[test]
    fn schema_json_v2_roundtrip() {
        let v = json!({
            "version": 2,
            "edges": {
                "company": { "group": {}, "property": {}, "building": {} },
                "group": { "building": {} },
                "property": { "building": {} },
                "building": { "area": { "min": 1 } }
            },
            "metadata": {
                "building": {
                    "lat": { "type": "number", "required": true, "min": -90.0, "max": 90.0 }
                }
            },
            "sensors": ["building", "area"]
        });
        let sch = schema_of_json(&v).expect("must parse");
        assert!(sch.edge_between("company", "building").is_some());
        assert_eq!(sch.edge_between("building", "area").unwrap().min, Some(1));
        let back = schema_to_json(&sch);
        let sch2 = schema_of_json(&back).expect("roundtrip");
        assert_eq!(sch, sch2);
    }

    // ------------------------------------------------------------------
    // 3b. schema_json_v1_rejected
    //
    // A v1 schema JSON must be rejected with the version number in the error.
    // ------------------------------------------------------------------
    #[test]
    fn schema_json_v1_rejected() {
        let v = json!({ "version": 1, "edges": {} });
        let err = schema_of_json(&v).unwrap_err();
        assert!(err.contains("version 1"), "got: {}", err);
    }

    // ------------------------------------------------------------------
    // 3c. node_json_includes_label
    //
    // node_to_json must include a "label" key with the node's type.
    // ------------------------------------------------------------------
    #[test]
    fn node_json_includes_label() {
        let mut nd = model::domain::node::make(
            7, Level::Hn3, "B1", NodeId::parse("HN2#1").unwrap(),
            "HN0#root|HN1#1|HN2#1", serde_json::json!({}), None,
        );
        nd.label = "building".to_string();
        let j = node_to_json(&nd);
        assert_eq!(j["label"], json!("building"));
    }

    // ------------------------------------------------------------------
    // 4. formula_of_json_default_identity
    //
    // Test: formula_of_json None → Identity.
    // ------------------------------------------------------------------
    #[test]
    fn formula_of_json_default_identity() {
        match formula_of_json(None) {
            Ok(Formula::Identity) => {}
            other => panic!("absent formula should be Identity, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // 5. formula_of_json_zero
    //
    // Test: formula_of_json (Some { kind: "zero" }) → Zero.
    // ------------------------------------------------------------------
    #[test]
    fn formula_of_json_zero() {
        let j = json!({ "kind": "zero" });
        match formula_of_json(Some(&j)) {
            Ok(Formula::Zero) => {}
            other => panic!("kind=zero should be Zero, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // 6. formula_of_json_expr_with_refs
    //
    // Test: expr "abs(self - a)" with refs {"a": "S#12"}.
    // Checks ast → "abs(self - a)", refs.len() == 1.
    // ------------------------------------------------------------------
    #[test]
    fn formula_of_json_expr_with_refs() {
        let j = json!({
            "kind": "expr",
            "expr": "abs(self - a)",
            "refs": { "a": "S#12" }
        });
        match formula_of_json(Some(&j)) {
            Ok(Formula::Expr { expr, refs }) => {
                assert_eq!(expr_to_string(&expr), "abs(self - a)");
                assert_eq!(refs.len(), 1);
            }
            other => panic!("should parse expr, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // 7. formula_of_json_refs_as_string
    //
    // Test: refs given as a JSON-encoded string.
    // ------------------------------------------------------------------
    #[test]
    fn formula_of_json_refs_as_string() {
        let j = json!({
            "kind": "expr",
            "expr": "self - a",
            "refs": r#"{"a":"S#7"}"#
        });
        match formula_of_json(Some(&j)) {
            Ok(Formula::Expr { .. }) => {}
            other => panic!("refs as JSON string should parse, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // 8. formula_of_json_unbound_alias_errors
    //
    // Test: expr uses "a" but refs is empty → Error.
    // ------------------------------------------------------------------
    #[test]
    fn formula_of_json_unbound_alias_errors() {
        let j = json!({
            "kind": "expr",
            "expr": "self - a",
            "refs": {}
        });
        match formula_of_json(Some(&j)) {
            Err(_) => {}
            Ok(f) => panic!("unbound alias must error, got {:?}", f),
        }
    }

    // ------------------------------------------------------------------
    // 9. sensor_to_json_includes_formula
    //
    // Test: sensor with formula = Zero → serialised formula.kind == "zero".
    // ------------------------------------------------------------------
    #[test]
    fn sensor_to_json_includes_formula() {
        let s = Sensor::builder()
            .id(SensorId::make(5))
            .created(DateTime::from_timestamp(0, 0).unwrap())
            .daq_id("d".to_string())
            .path("HN0#root|S#5".to_string())
            .purpose("E".to_string())
            .meter_type(MeterType::Counter)
            .formula(Formula::Zero)
            .build();
        let j = sensor_to_json(&s);
        let kind = j["formula"]["kind"]
            .as_str()
            .expect("formula.kind must be string");
        assert_eq!(kind, "zero");
    }

    // ------------------------------------------------------------------
    // 10. formula_of_json_unused_ref_errors
    //
    // Test: expr is "self" (no aliases), refs has "a" → Error.
    // ------------------------------------------------------------------
    #[test]
    fn formula_of_json_unused_ref_errors() {
        let j = json!({
            "kind": "expr",
            "expr": "self",
            "refs": { "a": "S#1" }
        });
        match formula_of_json(Some(&j)) {
            Err(_) => {}
            Ok(f) => panic!("unused ref must error, got {:?}", f),
        }
    }

    // ------------------------------------------------------------------
    // 11. formula_of_json_implicit_expr
    //
    // Test: no "kind" but has "expr" → implicit Expr.
    // ------------------------------------------------------------------
    #[test]
    fn formula_of_json_implicit_expr() {
        let j = json!({
            "expr": "self - a",
            "refs": { "a": "S#1" }
        });
        match formula_of_json(Some(&j)) {
            Ok(Formula::Expr { .. }) => {}
            other => panic!("kind-absent expr should parse, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // 12. formula_of_json_string_shorthands
    //
    // Test: null→Identity; "identity"→Identity; "zero"→Zero.
    // ------------------------------------------------------------------
    #[test]
    fn formula_of_json_string_shorthands() {
        match formula_of_json(Some(&Value::Null)) {
            Ok(Formula::Identity) => {}
            other => panic!("null→identity, got {:?}", other),
        }
        match formula_of_json(Some(&json!("identity"))) {
            Ok(Formula::Identity) => {}
            other => panic!("string identity, got {:?}", other),
        }
        match formula_of_json(Some(&json!("zero"))) {
            Ok(Formula::Zero) => {}
            other => panic!("string zero, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // 13. formula_json_roundtrips
    //
    // Test: Formula::Expr with "abs(self - a)" → formula_to_json
    // → formula_of_json → Expr with same ast/refs.
    // ------------------------------------------------------------------
    #[test]
    fn formula_json_roundtrips() {
        let ast = parse_expr("abs(self - a)").unwrap();
        let f = Formula::Expr {
            refs: vec![("a".to_string(), SensorId::make(12))],
            expr: ast,
        };
        let j = formula_to_json(&f);
        match formula_of_json(Some(&j)) {
            Ok(Formula::Expr { expr, refs }) => {
                assert_eq!(expr_to_string(&expr), "abs(self - a)");
                assert_eq!(refs.len(), 1);
            }
            other => panic!("round-trip failed, got {:?}", other),
        }
    }
}
