//! DynamoDB codec — maps domain structs ⟷ live DynamoDB item format.
//!
//! This is a direct port of `services/hierarchy/lib/repo/codec.ml`.  Every
//! attribute name, encoding choice (N strings, nested-M structure, sk verbs, …)
//! must match the OCaml reference exactly so that the Rust lambda can read and
//! write the 8,508 live items without migration.

use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;
use chrono::{DateTime, Utc};

use crate::domain::formula::{Expr, Formula};
use crate::domain::ids::{Level, NodeId, SensorId, UserId};
use crate::domain::node::Node;
use crate::domain::schema::{EdgeSpec, FieldSpec, Schema};
use crate::domain::sensor::Sensor;
use crate::domain::sensor_sk::SensorSk;
use crate::domain::user::User;
use crate::domain::values::{CognitoGroup, Currency, EdgeKind, FieldType, Language, MeterType};
use crate::errors::RepositoryError;

// ---------------------------------------------------------------------------
// Public type alias
// ---------------------------------------------------------------------------

/// A DynamoDB item: attribute name → value.
pub type Item = HashMap<String, AttributeValue>;

// ---------------------------------------------------------------------------
// Low-level helpers (mirrors codec.ml's `s`, `n`, `b`)
// ---------------------------------------------------------------------------

fn s(v: impl Into<String>) -> AttributeValue {
    AttributeValue::S(v.into())
}

fn n(v: impl Into<String>) -> AttributeValue {
    AttributeValue::N(v.into())
}

fn b(v: bool) -> AttributeValue {
    AttributeValue::Bool(v)
}

/// Format a `DateTime<Utc>` as RFC3339 with `Z` suffix — matches OCaml
/// `Ptime.to_rfc3339 ~tz_offset_s:0`.
fn dt_to_rfc3339z(dt: &DateTime<Utc>) -> String {
    let s = dt.to_rfc3339();
    if let Some(stripped) = s.strip_suffix("+00:00") {
        format!("{}Z", stripped)
    } else {
        s
    }
}

/// Parse an RFC3339 string; fall back to epoch on error (matches OCaml `Ptime.epoch`).
fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(DateTime::UNIX_EPOCH)
}

// ---------------------------------------------------------------------------
// Field-extraction helpers (mirrors codec.ml's `field`, `as_string`, …)
// ---------------------------------------------------------------------------

fn field<'a>(item: &'a Item, k: &str) -> Result<&'a AttributeValue, RepositoryError> {
    item.get(k).ok_or_else(|| RepositoryError::Codec(format!("missing field {:?}", k)))
}

fn field_map<'a>(
    map: &'a HashMap<String, AttributeValue>,
    k: &str,
) -> Result<&'a AttributeValue, RepositoryError> {
    map.get(k)
        .ok_or_else(|| RepositoryError::Codec(format!("missing field {:?}", k)))
}

fn as_s(v: &AttributeValue) -> Result<&str, RepositoryError> {
    match v {
        AttributeValue::S(s) => Ok(s.as_str()),
        _ => Err(RepositoryError::Codec("expected S".to_string())),
    }
}

fn as_m(v: &AttributeValue) -> Result<&HashMap<String, AttributeValue>, RepositoryError> {
    match v {
        AttributeValue::M(m) => Ok(m),
        _ => Err(RepositoryError::Codec("expected M".to_string())),
    }
}

fn as_l(v: &AttributeValue) -> Result<&Vec<AttributeValue>, RepositoryError> {
    match v {
        AttributeValue::L(l) => Ok(l),
        _ => Err(RepositoryError::Codec("expected L".to_string())),
    }
}

fn opt_int_of_n(v: Option<&AttributeValue>) -> Option<i32> {
    match v {
        Some(AttributeValue::N(s)) => s.parse::<i32>().ok(),
        _ => None,
    }
}

fn opt_i64_of_n(v: Option<&AttributeValue>) -> Option<i64> {
    match v {
        Some(AttributeValue::N(s)) => s.parse::<i64>().ok(),
        _ => None,
    }
}

fn opt_f64_of_n(v: Option<&AttributeValue>) -> Option<f64> {
    match v {
        Some(AttributeValue::N(s)) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// OCaml `Printf.sprintf "%.17g"` — 17 significant digits, shortest representation.
fn format_float_g17(f: f64) -> String {
    if f.is_nan() {
        return "nan".to_string();
    }
    if f.is_infinite() {
        return if f > 0.0 {
            "infinity".to_string()
        } else {
            "-infinity".to_string()
        };
    }
    // Format with enough precision, then trim.
    // Use Rust's default f64 Display which already gives shortest representation,
    // but we need exactly %.17g behavior (17 significant digits).
    format!("{:.17e}", f)
        .parse::<f64>()
        .map(|_| ocaml_g17(f))
        .unwrap_or_else(|_| format!("{}", f))
}

/// Reproduce OCaml's `%.17g`: up to 17 significant digits, no trailing zeros,
/// scientific for |exp| >= 17 or exp < -4.
fn ocaml_g17(f: f64) -> String {
    // Use 17-digit scientific, then reformat like %g
    let s = format!("{:.16e}", f); // 17 sig figs in "d.dddde+XX" form
    let (mant_s, exp_s) = s.split_once('e').unwrap();
    let exp: i32 = exp_s.parse().unwrap();
    let mant_trimmed = mant_s.trim_end_matches('0').trim_end_matches('.');

    if (-4..17).contains(&exp) {
        // Fixed form
        let (sign, pos) = if let Some(stripped) = mant_trimmed.strip_prefix('-') {
            ("-", stripped)
        } else {
            ("", mant_trimmed)
        };
        let (int_digits, frac_digits) = if let Some((a, b)) = pos.split_once('.') {
            (a, b)
        } else {
            (pos, "")
        };
        let mut digits: Vec<char> = int_digits.chars().chain(frac_digits.chars()).collect();
        let dot_pos = exp + 1;
        if dot_pos <= 0 {
            let leading_zeros = (-dot_pos) as usize;
            let frac: String = digits.iter().collect();
            let zeros = "0".repeat(leading_zeros);
            if frac.is_empty() {
                format!("{}0", sign)
            } else {
                format!("{}0.{}{}", sign, zeros, frac)
            }
        } else {
            let dp = dot_pos as usize;
            while digits.len() < dp {
                digits.push('0');
            }
            let (l, r) = digits.split_at(dp);
            let ls: String = l.iter().collect();
            let rs: String = r.iter().collect();
            if rs.is_empty() {
                format!("{}{}", sign, ls)
            } else {
                format!("{}{}.{}", sign, ls, rs)
            }
        }
    } else {
        let exp_formatted = if exp >= 0 {
            format!("e+{}", exp)
        } else {
            format!("e{}", exp)
        };
        format!("{}{}", mant_trimmed, exp_formatted)
    }
}

// ---------------------------------------------------------------------------
// GSI key helpers (mirrors codec.ml's node_gsi1pk / sensor_gsi1pk)
// ---------------------------------------------------------------------------

fn node_gsi1pk(lvl: Level) -> String {
    format!("HN{}", lvl.depth())
}

const SENSOR_GSI1PK: &str = "S";

// ---------------------------------------------------------------------------
// Formula encode/decode (mirrors codec.ml's expr_to_attr / formula_to_attr)
// ---------------------------------------------------------------------------

fn expr_to_av(e: &Expr) -> AttributeValue {
    let mut m: HashMap<String, AttributeValue> = HashMap::new();
    match e {
        Expr::Num(f) => {
            m.insert("t".to_string(), s("num"));
            m.insert("v".to_string(), n(format_float_g17(*f)));
            AttributeValue::M(m)
        }
        Expr::SelfRef => {
            m.insert("t".to_string(), s("self"));
            AttributeValue::M(m)
        }
        Expr::Ref(a) => {
            m.insert("t".to_string(), s("ref"));
            m.insert("a".to_string(), s(a.clone()));
            AttributeValue::M(m)
        }
        Expr::Abs(inner) => {
            m.insert("t".to_string(), s("abs"));
            m.insert("e".to_string(), expr_to_av(inner));
            AttributeValue::M(m)
        }
        Expr::Add(a, b) => {
            m.insert("t".to_string(), s("add"));
            m.insert("l".to_string(), expr_to_av(a));
            m.insert("r".to_string(), expr_to_av(b));
            AttributeValue::M(m)
        }
        Expr::Sub(a, b) => {
            m.insert("t".to_string(), s("sub"));
            m.insert("l".to_string(), expr_to_av(a));
            m.insert("r".to_string(), expr_to_av(b));
            AttributeValue::M(m)
        }
        Expr::Mul(a, b) => {
            m.insert("t".to_string(), s("mul"));
            m.insert("l".to_string(), expr_to_av(a));
            m.insert("r".to_string(), expr_to_av(b));
            AttributeValue::M(m)
        }
        Expr::Div(a, b) => {
            m.insert("t".to_string(), s("div"));
            m.insert("l".to_string(), expr_to_av(a));
            m.insert("r".to_string(), expr_to_av(b));
            AttributeValue::M(m)
        }
    }
}

fn expr_of_av(v: &AttributeValue) -> Result<Expr, RepositoryError> {
    let kvs = as_m(v)?;
    let tag = as_s(field_map(kvs, "t")?)?;
    match tag {
        "num" => {
            let nv = field_map(kvs, "v")?;
            match nv {
                AttributeValue::N(s) => s
                    .parse::<f64>()
                    .map(Expr::Num)
                    .map_err(|_| RepositoryError::Codec("bad num".to_string())),
                _ => Err(RepositoryError::Codec("num needs N".to_string())),
            }
        }
        "self" => Ok(Expr::SelfRef),
        "ref" => {
            let a = as_s(field_map(kvs, "a")?)?;
            Ok(Expr::Ref(a.to_string()))
        }
        "abs" => {
            let e = field_map(kvs, "e")?;
            Ok(Expr::Abs(Box::new(expr_of_av(e)?)))
        }
        "add" | "sub" | "mul" | "div" => {
            let l = expr_of_av(field_map(kvs, "l")?)?;
            let r = expr_of_av(field_map(kvs, "r")?)?;
            match tag {
                "add" => Ok(Expr::Add(Box::new(l), Box::new(r))),
                "sub" => Ok(Expr::Sub(Box::new(l), Box::new(r))),
                "mul" => Ok(Expr::Mul(Box::new(l), Box::new(r))),
                _ => Ok(Expr::Div(Box::new(l), Box::new(r))),
            }
        }
        other => Err(RepositoryError::Codec(format!("unknown expr tag {:?}", other))),
    }
}

fn formula_to_av(f: &Formula) -> AttributeValue {
    let mut m: HashMap<String, AttributeValue> = HashMap::new();
    match f {
        Formula::Identity => {
            m.insert("kind".to_string(), s("identity"));
            AttributeValue::M(m)
        }
        Formula::Zero => {
            m.insert("kind".to_string(), s("zero"));
            AttributeValue::M(m)
        }
        Formula::Expr { expr, refs } => {
            m.insert("kind".to_string(), s("expr"));
            m.insert("ast".to_string(), expr_to_av(expr));
            let refs_m: HashMap<String, AttributeValue> = refs
                .iter()
                .map(|(alias, sid)| (alias.clone(), n(sid.id().to_string())))
                .collect();
            m.insert("refs".to_string(), AttributeValue::M(refs_m));
            AttributeValue::M(m)
        }
    }
}

fn formula_of_av(v: &AttributeValue) -> Result<Formula, RepositoryError> {
    let kvs = as_m(v)?;
    let kind = as_s(field_map(kvs, "kind")?)?;
    match kind {
        "identity" => Ok(Formula::Identity),
        "zero" => Ok(Formula::Zero),
        "expr" => {
            let ast = expr_of_av(field_map(kvs, "ast")?)?;
            let refs_av = field_map(kvs, "refs")?;
            let refs_m = as_m(refs_av)?;
            let mut refs: Vec<(String, SensorId)> = Vec::new();
            for (alias, nv) in refs_m {
                match nv {
                    AttributeValue::N(s) => {
                        let i = s.parse::<u32>().map_err(|_| {
                            RepositoryError::Codec(format!("bad ref id {:?}", s))
                        })?;
                        refs.push((alias.clone(), SensorId::make(i)));
                    }
                    _ => {
                        return Err(RepositoryError::Codec(
                            "expr ref must be N".to_string(),
                        ))
                    }
                }
            }
            // Preserve insertion order (OCaml List.rev after fold)
            Ok(Formula::Expr { expr: ast, refs })
        }
        other => Err(RepositoryError::Codec(format!("unknown formula kind {:?}", other))),
    }
}

// ---------------------------------------------------------------------------
// Schema encode/decode (mirrors codec.ml's schema_to_attr / decode_schema)
// ---------------------------------------------------------------------------

fn field_type_to_av(ft: &FieldType) -> AttributeValue {
    let mut m: HashMap<String, AttributeValue> = HashMap::new();
    match ft {
        FieldType::String { min_len, max_len } => {
            m.insert("type".to_string(), s("string"));
            if let Some(v) = min_len {
                m.insert("min_len".to_string(), n(v.to_string()));
            }
            if let Some(v) = max_len {
                m.insert("max_len".to_string(), n(v.to_string()));
            }
            AttributeValue::M(m)
        }
        FieldType::Number { min, max } => {
            m.insert("type".to_string(), s("number"));
            if let Some(v) = min {
                m.insert("min".to_string(), n(format_float_g17(*v)));
            }
            if let Some(v) = max {
                m.insert("max".to_string(), n(format_float_g17(*v)));
            }
            AttributeValue::M(m)
        }
        FieldType::Integer { min, max } => {
            m.insert("type".to_string(), s("integer"));
            if let Some(v) = min {
                m.insert("min".to_string(), n(v.to_string()));
            }
            if let Some(v) = max {
                m.insert("max".to_string(), n(v.to_string()));
            }
            AttributeValue::M(m)
        }
        FieldType::Boolean => {
            m.insert("type".to_string(), s("boolean"));
            AttributeValue::M(m)
        }
        FieldType::Timestamp => {
            m.insert("type".to_string(), s("timestamp"));
            AttributeValue::M(m)
        }
        FieldType::Enum { one_of } => {
            m.insert("type".to_string(), s("enum"));
            m.insert(
                "one_of".to_string(),
                AttributeValue::L(one_of.iter().map(|v| s(v.clone())).collect()),
            );
            AttributeValue::M(m)
        }
    }
}

fn field_spec_to_av(fs: &FieldSpec) -> AttributeValue {
    // Mirror OCaml spec_m: start with the field_type map, then add "required"
    let inner = field_type_to_av(&fs.typ);
    match inner {
        AttributeValue::M(mut kvs) => {
            kvs.insert("required".to_string(), b(fs.required));
            AttributeValue::M(kvs)
        }
        other => other,
    }
}

fn edge_spec_to_av(spec: &EdgeSpec) -> AttributeValue {
    let mut m: HashMap<String, AttributeValue> = HashMap::new();
    if let Some(min) = spec.min {
        m.insert("min".to_string(), n(min.to_string()));
    }
    if let Some(max) = spec.max {
        m.insert("max".to_string(), n(max.to_string()));
    }
    AttributeValue::M(m)
}

/// Encode a `Schema` as a nested `M` AttributeValue.
/// Mirrors OCaml `schema_to_attr`.
pub fn schema_to_av(sch: &Schema) -> AttributeValue {
    // edges: M{ level_str -> M{ child_str -> M{ label -> M{min?,max?} } } }
    let edges_m: HashMap<String, AttributeValue> = sch
        .edges
        .iter()
        .map(|(lvl, children)| {
            let inner: HashMap<String, AttributeValue> = children
                .iter()
                .map(|(child, specs)| {
                    let label_m: HashMap<String, AttributeValue> = specs
                        .iter()
                        .map(|sp| (sp.label.clone(), edge_spec_to_av(sp)))
                        .collect();
                    (child.to_string(), AttributeValue::M(label_m))
                })
                .collect();
            (lvl.to_string(), AttributeValue::M(inner))
        })
        .collect();

    // metadata: M{ level_str -> M{ field_name -> M{required, type, ...} } }
    let metadata_m: HashMap<String, AttributeValue> = sch
        .metadata
        .iter()
        .map(|(lvl, fields)| {
            let inner: HashMap<String, AttributeValue> = fields
                .iter()
                .map(|(name, fs)| (name.clone(), field_spec_to_av(fs)))
                .collect();
            (lvl.to_string(), AttributeValue::M(inner))
        })
        .collect();

    // sensors: L[ S"hn4", S"hn5", ... ]
    let sensors_l: Vec<AttributeValue> =
        sch.sensors.iter().map(|lvl| s(lvl.to_string())).collect();

    let mut m: HashMap<String, AttributeValue> = HashMap::new();
    m.insert("version".to_string(), n(sch.version.to_string()));
    m.insert("edges".to_string(), AttributeValue::M(edges_m));
    m.insert("metadata".to_string(), AttributeValue::M(metadata_m));
    m.insert("sensors".to_string(), AttributeValue::L(sensors_l));
    AttributeValue::M(m)
}

fn decode_field_spec(v: &AttributeValue) -> Result<FieldSpec, RepositoryError> {
    let kvs = as_m(v)?;
    let typ_s = as_s(field_map(kvs, "type")?)?;
    let required = match kvs.get("required") {
        Some(AttributeValue::Bool(b)) => *b,
        _ => false,
    };
    let typ = match typ_s {
        "string" => FieldType::String {
            min_len: opt_i64_of_n(kvs.get("min_len")),
            max_len: opt_i64_of_n(kvs.get("max_len")),
        },
        "number" => FieldType::Number {
            min: opt_f64_of_n(kvs.get("min")),
            max: opt_f64_of_n(kvs.get("max")),
        },
        "integer" => FieldType::Integer {
            min: opt_i64_of_n(kvs.get("min")),
            max: opt_i64_of_n(kvs.get("max")),
        },
        "boolean" => FieldType::Boolean,
        "timestamp" => FieldType::Timestamp,
        "enum" => {
            let xs = as_l(field_map(kvs, "one_of")?)?;
            let vals: Result<Vec<String>, _> = xs.iter().map(|x| as_s(x).map(str::to_string)).collect();
            FieldType::Enum { one_of: vals? }
        }
        other => {
            return Err(RepositoryError::Codec(format!(
                "unknown field type {:?}",
                other
            )))
        }
    };
    Ok(FieldSpec { typ, required })
}

fn decode_edge_spec(label: &str, body: &AttributeValue) -> Result<EdgeSpec, RepositoryError> {
    let kvs = as_m(body)?;
    Ok(EdgeSpec {
        label: label.to_string(),
        min: opt_int_of_n(kvs.get("min")),
        max: opt_int_of_n(kvs.get("max")),
    })
}

/// Decode a `Schema` from a nested `M` AttributeValue.
/// Mirrors OCaml `decode_schema`.
#[allow(clippy::type_complexity)]
pub fn schema_of_av(v: &AttributeValue) -> Result<Schema, RepositoryError> {
    let kvs = as_m(v)?;
    let version = match field_map(kvs, "version")? {
        AttributeValue::N(s) => s.parse::<u32>().map_err(|_| {
            RepositoryError::Codec("bad version N".to_string())
        })?,
        _ => return Err(RepositoryError::Codec("expected N for version".to_string())),
    };

    #[allow(clippy::type_complexity)]
    let edges_v = field_map(kvs, "edges")?;
    let edges_kvs = as_m(edges_v)?;
    let mut edges: Vec<(Level, Vec<(Level, Vec<EdgeSpec>)>)> = Vec::new();
    for (lvl_s, inner_v) in edges_kvs {
        let lvl = match lvl_s.parse::<Level>() {
            Ok(l) => l,
            Err(_) => continue,
        };
        let inner_kvs = as_m(inner_v)?;
        let mut children: Vec<(Level, Vec<EdgeSpec>)> = Vec::new();
        for (child_s, labels_v) in inner_kvs {
            let child = match child_s.parse::<Level>() {
                Ok(l) => l,
                Err(_) => continue,
            };
            let label_kvs = as_m(labels_v)?;
            let mut specs: Vec<EdgeSpec> = Vec::new();
            for (label, body) in label_kvs {
                specs.push(decode_edge_spec(label, body)?);
            }
            children.push((child, specs));
        }
        edges.push((lvl, children));
    }

    let metadata = match kvs.get("metadata") {
        None => vec![],
        Some(m_v) => {
            let m_kvs = as_m(m_v)?;
            let mut result: Vec<(Level, Vec<(String, FieldSpec)>)> = Vec::new();
            for (lvl_s, inner_v) in m_kvs {
                let lvl = match lvl_s.parse::<Level>() {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                let inner_kvs = as_m(inner_v)?;
                let mut fields: Vec<(String, FieldSpec)> = Vec::new();
                for (fname, spec_v) in inner_kvs {
                    fields.push((fname.clone(), decode_field_spec(spec_v)?));
                }
                result.push((lvl, fields));
            }
            result
        }
    };

    let sensors = match kvs.get("sensors") {
        None => vec![],
        Some(v) => {
            let xs = as_l(v)?;
            let mut result: Vec<Level> = Vec::new();
            for item in xs {
                let s_str = as_s(item)?;
                if let Ok(lvl) = s_str.parse::<Level>() {
                    result.push(lvl);
                }
            }
            result
        }
    };

    Ok(Schema {
        version,
        edges,
        metadata,
        sensors,
    })
}

// ---------------------------------------------------------------------------
// Node encode/decode
// ---------------------------------------------------------------------------

/// Encode a `Node` as a DynamoDB `Item`.
/// Mirrors OCaml `node_to_item`.
pub fn node_to_item(nd: &Node) -> Item {
    let id = nd.id.to_string();
    let mut item: Item = HashMap::new();
    item.insert("pk".to_string(), s(id.clone()));
    item.insert("sk".to_string(), s(id.clone()));
    item.insert("type".to_string(), s("node"));
    item.insert("name".to_string(), s(nd.name.clone()));
    item.insert("created".to_string(), s(dt_to_rfc3339z(&nd.created)));
    item.insert(
        "metadata".to_string(),
        serde_dynamo::to_attribute_value(&nd.metadata)
            .map_err(|e| RepositoryError::Codec(format!("metadata encode: {}", e)))
            .unwrap_or_else(|_| AttributeValue::M(HashMap::new())),
    );
    item.insert("gsi1pk".to_string(), s(node_gsi1pk(nd.level())));
    item.insert("gsi1sk".to_string(), s(nd.path.clone()));
    if let Some(ref sch) = nd.schema {
        item.insert("schema".to_string(), schema_to_av(sch));
    }
    item
}

/// Decode a `Node` from a DynamoDB `Item`.
/// Mirrors OCaml `node_of_item`.
pub fn node_of_item(item: &Item) -> Result<Node, RepositoryError> {
    let pk_s = as_s(field(item, "pk")?)?;
    let id = NodeId::parse(pk_s)
        .map_err(|e| RepositoryError::Codec(format!("bad node id {:?}: {}", pk_s, e)))?;
    let name = as_s(field(item, "name")?)?.to_string();
    let created_s = as_s(field(item, "created")?)?;
    let created = parse_ts(created_s);
    let metadata = match item.get("metadata") {
        Some(v) => {
            let result: Result<serde_json::Value, _> =
                serde_dynamo::from_attribute_value(v.clone());
            result.map_err(|e| RepositoryError::Codec(format!("metadata decode: {}", e)))?
        },
        None => serde_json::json!({}),
    };
    let schema = match item.get("schema") {
        None => None,
        Some(v) => schema_of_av(v).ok(),
    };
    let path = match item.get("gsi1sk") {
        Some(AttributeValue::S(v)) => v.clone(),
        _ => {
            return Err(RepositoryError::Codec(format!(
                "node {} missing gsi1sk/path",
                id
            )))
        }
    };
    let parent = parent_from_path(&path);
    Ok(Node {
        id,
        name,
        parent,
        path,
        created,
        metadata,
        schema,
    })
}

/// Extract the parent `NodeId` from a path string.
/// Mirrors OCaml `parent_from_path`: second-to-last pipe-separated segment.
fn parent_from_path(path: &str) -> Option<NodeId> {
    let parts: Vec<&str> = path.split('|').filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [.., par, _self] => NodeId::parse(par).ok(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Edge encode (mirrors codec.ml's edge_item / edge_with_anchor)
// ---------------------------------------------------------------------------

/// Parameters for encoding a generic edge.
pub struct EdgeParams<'a> {
    pub from_: &'a str,
    pub to_: &'a str,
    pub kind: &'a EdgeKind,
    pub name: &'a str,
    pub created: &'a DateTime<Utc>,
}

/// Encode a generic (user-side) edge — `administrates` / `blocked`.
/// Mirrors OCaml `edge_item`.
pub fn edge_to_item(p: EdgeParams<'_>) -> Item {
    let mut item: Item = HashMap::new();
    item.insert("pk".to_string(), s(p.from_));
    item.insert(
        "sk".to_string(),
        s(format!("{}#{}", p.kind.sk_verb(), p.to_)),
    );
    item.insert("type".to_string(), s("edge"));
    item.insert("kind".to_string(), s(p.kind.kind_string()));
    item.insert("name".to_string(), s(p.name));
    item.insert("created".to_string(), s(dt_to_rfc3339z(p.created)));
    if let Some(verb) = p.kind.gsi_verb() {
        item.insert("gsi1pk".to_string(), s(p.to_));
        item.insert(
            "gsi1sk".to_string(),
            s(format!("{}#{}", verb, p.from_)),
        );
    }
    item
}

/// Parameters for encoding a hierarchy-side (anchor) edge.
pub struct AnchorEdgeParams<'a> {
    pub from_: &'a str,
    pub to_: &'a str,
    pub kind: &'a EdgeKind,
    pub name: &'a str,
    pub created: &'a DateTime<Utc>,
    /// The *child's* full path (used as `gsi1sk`).
    pub self_path: &'a str,
}

/// Encode a HN-side edge (`has_<label>` / `has_sensor`).
/// Mirrors OCaml `edge_with_anchor`.
pub fn anchor_edge_to_item(p: AnchorEdgeParams<'_>) -> Item {
    let gsi1pk_v: String = match p.kind {
        EdgeKind::HasSensor => SENSOR_GSI1PK.to_string(),
        EdgeKind::HasLabel(_) => {
            // gsi1pk is derived from the child's level
            if let Ok(child_id) = NodeId::parse(p.to_) {
                node_gsi1pk(child_id.level())
            } else {
                panic!("anchor_edge_to_item: bad to_ {:?}", p.to_)
            }
        }
        _ => panic!("anchor_edge_to_item: only HN-side edges"),
    };
    let mut item: Item = HashMap::new();
    item.insert("pk".to_string(), s(p.from_));
    item.insert(
        "sk".to_string(),
        s(format!("{}#{}", p.kind.sk_verb(), p.to_)),
    );
    item.insert("type".to_string(), s("edge"));
    item.insert("kind".to_string(), s(p.kind.kind_string()));
    item.insert("name".to_string(), s(p.name));
    item.insert("created".to_string(), s(dt_to_rfc3339z(p.created)));
    item.insert("gsi1pk".to_string(), s(gsi1pk_v));
    item.insert("gsi1sk".to_string(), s(p.self_path));
    item
}

/// Decode a generic edge item into its key fields.
/// Returns `(from, to, kind, name, created, gsi1pk_opt, gsi1sk_opt)`.
#[allow(clippy::type_complexity)]
pub fn edge_of_item(
    item: &Item,
) -> Result<
    (
        String,
        String,
        EdgeKind,
        String,
        DateTime<Utc>,
        Option<String>,
        Option<String>,
    ),
    RepositoryError,
> {
    let from_ = as_s(field(item, "pk")?)?.to_string();
    let sk = as_s(field(item, "sk")?)?;
    let kind_s = as_s(field(item, "kind")?)?;
    let kind = EdgeKind::parse(kind_s)
        .map_err(|e| RepositoryError::Codec(format!("bad edge kind {:?}: {}", kind_s, e)))?;
    // to_ is the part after the first '#' in sk
    let to_ = sk
        .find('#')
        .map(|i| sk[i + 1..].to_string())
        .ok_or_else(|| RepositoryError::Codec(format!("bad edge sk {:?}", sk)))?;
    let name = as_s(field(item, "name")?)?.to_string();
    let created_s = as_s(field(item, "created")?)?;
    let created = parse_ts(created_s);
    let gsi1pk = item.get("gsi1pk").and_then(|v| {
        if let AttributeValue::S(s) = v {
            Some(s.clone())
        } else {
            None
        }
    });
    let gsi1sk = item.get("gsi1sk").and_then(|v| {
        if let AttributeValue::S(s) = v {
            Some(s.clone())
        } else {
            None
        }
    });
    Ok((from_, to_, kind, name, created, gsi1pk, gsi1sk))
}

// ---------------------------------------------------------------------------
// Sensor encode/decode
// ---------------------------------------------------------------------------

/// Encode a `Sensor` as a DynamoDB `Item` (always as the `active` SK).
/// Mirrors OCaml `sensor_to_item ~active:true`.
pub fn sensor_to_item(sn: &Sensor) -> Item {
    let pk = sn.id.to_string();
    let sk = SensorSk::Active(sn.created).to_string();
    let mut item: Item = HashMap::new();
    item.insert("pk".to_string(), s(pk));
    item.insert("sk".to_string(), s(sk));
    item.insert("type".to_string(), s("sensor"));
    item.insert("daq_id".to_string(), s(sn.daq_id.clone()));
    item.insert("gsi1pk".to_string(), s(SENSOR_GSI1PK));
    item.insert("gsi1sk".to_string(), s(sn.path.clone()));
    item.insert("purpose".to_string(), s(sn.purpose.clone()));
    item.insert(
        "meter_type".to_string(),
        s(sn.meter_type.to_string()),
    );
    item.insert("formula".to_string(), formula_to_av(&sn.formula));
    item.insert("created".to_string(), s(dt_to_rfc3339z(&sn.created)));
    if let Some(rm) = sn.resample_minutes {
        item.insert("resample_minutes".to_string(), n(rm.to_string()));
    }
    if let Some(ref u) = sn.unit {
        item.insert("unit".to_string(), s(u.clone()));
    }
    item
}

/// Decode a `Sensor` from a DynamoDB `Item`.
/// Mirrors OCaml `sensor_of_item`.
pub fn sensor_of_item(item: &Item) -> Result<Sensor, RepositoryError> {
    let pk_s = as_s(field(item, "pk")?)?;
    let id = SensorId::parse(pk_s)
        .map_err(|e| RepositoryError::Codec(format!("bad sensor id {:?}: {}", pk_s, e)))?;
    let daq_id = as_s(field(item, "daq_id")?)?.to_string();
    let path = as_s(field(item, "gsi1sk")?)?.to_string();
    let purpose = as_s(field(item, "purpose")?)?.to_string();
    let mt_s = as_s(field(item, "meter_type")?)?;
    let meter_type = mt_s.parse::<MeterType>()
        .map_err(|e| RepositoryError::Codec(format!("bad meter_type {:?}: {}", mt_s, e)))?;
    let unit = match item.get("unit") {
        Some(AttributeValue::S(u)) => Some(u.clone()),
        _ => None,
    };
    let created_s = as_s(field(item, "created")?)?;
    let created = parse_ts(created_s);
    let formula = match item.get("formula") {
        Some(v) => formula_of_av(v)?,
        None => Formula::Identity,
    };
    let resample_minutes = opt_int_of_n(item.get("resample_minutes"));
    Ok(Sensor {
        id,
        created,
        daq_id,
        path,
        purpose,
        meter_type,
        unit,
        formula,
        resample_minutes,
    })
}

// ---------------------------------------------------------------------------
// User encode/decode
// ---------------------------------------------------------------------------

/// Encode a `User` as a DynamoDB `Item`.
/// Mirrors OCaml `user_item`.
pub fn user_to_item(u: &User) -> Item {
    let uid = u.id.to_string();
    let mut item: Item = HashMap::new();
    item.insert("pk".to_string(), s(uid.clone()));
    item.insert("sk".to_string(), s(uid.clone()));
    item.insert("type".to_string(), s("user"));
    item.insert("name".to_string(), s(u.name.clone()));
    item.insert("cognito_group".to_string(), s(u.cognito_group.to_string()));
    item.insert("language".to_string(), s(u.language.to_string()));
    item.insert("currency".to_string(), s(u.currency.to_string()));
    item.insert("created".to_string(), s(dt_to_rfc3339z(&u.created)));
    item.insert("gsi1pk".to_string(), s("user"));
    item.insert("gsi1sk".to_string(), s(uid));
    item
}

/// Decode a `User` from a DynamoDB `Item`.
/// Mirrors OCaml `user_of_item`.
pub fn user_of_item(item: &Item) -> Result<User, RepositoryError> {
    let pk_s = as_s(field(item, "pk")?)?;
    let id = UserId::parse(pk_s)
        .map_err(|e| RepositoryError::Codec(format!("bad user id {:?}: {}", pk_s, e)))?;
    let email = id.email().to_string();
    let name = as_s(field(item, "name")?)?.to_string();
    let g_s = as_s(field(item, "cognito_group")?)?;
    let cognito_group = g_s.parse::<CognitoGroup>()
        .map_err(|e| RepositoryError::Codec(format!("bad cognito_group {:?}: {}", g_s, e)))?;
    let language = match item.get("language") {
        Some(AttributeValue::S(s)) => s.parse::<Language>().unwrap_or_default(),
        _ => Language::default(),
    };
    let currency = match item.get("currency") {
        Some(AttributeValue::S(s)) => s.parse::<Currency>().unwrap_or_default(),
        _ => Currency::default(),
    };
    let created_s = as_s(field(item, "created")?)?;
    let created = parse_ts(created_s);
    Ok(User {
        email,
        id,
        name,
        cognito_group,
        language,
        currency,
        created,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::Path;

    // -----------------------------------------------------------------------
    // Fixture loader
    // -----------------------------------------------------------------------

    /// Load a fixture JSON file and convert the `{"S":..} / {"N":..} / ...` envelope
    /// format into a `HashMap<String, AttributeValue>`.
    fn load_fixture(name: &str) -> Item {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let path = Path::new(manifest)
            .join("tests")
            .join("fixtures")
            .join(name);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read fixture {:?}: {}", path.display(), e));
        let json: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse fixture {:?}: {}", path.display(), e));
        let obj = json.as_object().expect("fixture must be a JSON object");
        obj.iter()
            .map(|(k, v)| (k.clone(), json_envelope_to_av(v)))
            .collect()
    }

    /// Recursively convert the DynamoDB JSON envelope (`{"S": "..."}`, `{"N": "..."}`, etc.)
    /// to `AttributeValue`.
    fn json_envelope_to_av(v: &serde_json::Value) -> AttributeValue {
        let obj = v.as_object().expect("each attribute must be a JSON object");
        assert_eq!(obj.len(), 1, "each attribute wrapper must have exactly one key");
        let (type_tag, inner) = obj.iter().next().unwrap();
        match type_tag.as_str() {
            "S" => AttributeValue::S(inner.as_str().unwrap().to_string()),
            "N" => AttributeValue::N(inner.as_str().unwrap().to_string()),
            "BOOL" => AttributeValue::Bool(inner.as_bool().unwrap()),
            "NULL" => AttributeValue::Null(inner.as_bool().unwrap_or(true)),
            "M" => {
                let m = inner.as_object().expect("M must be object");
                let hm: HashMap<String, AttributeValue> =
                    m.iter().map(|(k, v)| (k.clone(), json_envelope_to_av(v))).collect();
                AttributeValue::M(hm)
            }
            "L" => {
                let l = inner.as_array().expect("L must be array");
                AttributeValue::L(l.iter().map(json_envelope_to_av).collect())
            }
            "B" => AttributeValue::Null(true), // not used
            other => panic!("unknown attribute type tag {:?}", other),
        }
    }

    /// Compare two Items attribute-by-attribute (order-independent, N strings by value).
    fn assert_items_eq(expected: &Item, actual: &Item, context: &str) {
        let mut expected_keys: Vec<&str> = expected.keys().map(|s| s.as_str()).collect();
        expected_keys.sort();
        let mut actual_keys: Vec<&str> = actual.keys().map(|s| s.as_str()).collect();
        actual_keys.sort();
        assert_eq!(
            expected_keys, actual_keys,
            "{}: attribute keys differ\n  expected: {:?}\n  actual:   {:?}",
            context, expected_keys, actual_keys
        );
        for k in &expected_keys {
            assert_av_eq(
                expected.get(*k).unwrap(),
                actual.get(*k).unwrap(),
                &format!("{}.{}", context, k),
            );
        }
    }

    fn assert_av_eq(expected: &AttributeValue, actual: &AttributeValue, path: &str) {
        match (expected, actual) {
            (AttributeValue::S(a), AttributeValue::S(b)) => {
                assert_eq!(a, b, "{}: S values differ", path);
            }
            (AttributeValue::N(a), AttributeValue::N(b)) => {
                // Compare as parsed f64 to handle "180" vs "180.0" etc.
                let fa: f64 = a.parse().unwrap_or(f64::NAN);
                let fb: f64 = b.parse().unwrap_or(f64::NAN);
                if fa.is_nan() && fb.is_nan() {
                    // both nan, ok
                } else {
                    assert!(
                        (fa - fb).abs() < 1e-10 || a == b,
                        "{}: N values differ: {:?} vs {:?}",
                        path, a, b
                    );
                }
            }
            (AttributeValue::Bool(a), AttributeValue::Bool(b)) => {
                assert_eq!(a, b, "{}: BOOL values differ", path);
            }
            (AttributeValue::Null(_), AttributeValue::Null(_)) => {}
            (AttributeValue::M(a), AttributeValue::M(b)) => {
                let mut aks: Vec<&str> = a.keys().map(|s| s.as_str()).collect();
                aks.sort();
                let mut bks: Vec<&str> = b.keys().map(|s| s.as_str()).collect();
                bks.sort();
                assert_eq!(aks, bks, "{}: M keys differ", path);
                for k in &aks {
                    assert_av_eq(
                        a.get(*k).unwrap(),
                        b.get(*k).unwrap(),
                        &format!("{}[{}]", path, k),
                    );
                }
            }
            (AttributeValue::L(a), AttributeValue::L(b)) => {
                assert_eq!(a.len(), b.len(), "{}: L lengths differ", path);
                for (i, (av, bv)) in a.iter().zip(b.iter()).enumerate() {
                    assert_av_eq(av, bv, &format!("{}[{}]", path, i));
                }
            }
            _ => panic!("{}: type mismatch: {:?} vs {:?}", path, expected, actual),
        }
    }

    // -----------------------------------------------------------------------
    // node_hn2 fixture (node WITH schema + metadata)
    // -----------------------------------------------------------------------

    #[test]
    fn node_hn2_decode() {
        let item = load_fixture("node_hn2.json");
        let node = node_of_item(&item).expect("decode node_hn2");
        assert_eq!(node.id.to_string(), "HN2#10003");
        assert_eq!(node.name, "SeedCo01");
        assert_eq!(node.path, "HN0#root|HN1#10001|HN2#10003");
        assert!(node.schema.is_some(), "should have schema");
        let sch = node.schema.as_ref().unwrap();
        assert_eq!(sch.version, 1);
        assert_eq!(sch.sensors, vec![Level::Hn4, Level::Hn5]);
        // parent = HN1#10001 (second-to-last segment)
        assert_eq!(
            node.parent.as_ref().map(|id| id.to_string()),
            Some("HN1#10001".to_string())
        );
    }

    #[test]
    fn node_hn2_roundtrip() {
        let item = load_fixture("node_hn2.json");
        let node = node_of_item(&item).expect("decode node_hn2");
        let reencoded = node_to_item(&node);
        assert_items_eq(&item, &reencoded, "node_hn2 roundtrip");
    }

    // -----------------------------------------------------------------------
    // node_hn3 fixture (plain node, no schema)
    // -----------------------------------------------------------------------

    #[test]
    fn node_hn3_decode() {
        let item = load_fixture("node_hn3.json");
        let node = node_of_item(&item).expect("decode node_hn3");
        assert_eq!(node.id.to_string(), "HN3#10004");
        assert_eq!(node.name, "Prop01");
        assert_eq!(node.path, "HN0#root|HN1#10001|HN2#10003|HN3#10004");
        assert!(node.schema.is_none(), "no schema");
    }

    #[test]
    fn node_hn3_roundtrip() {
        let item = load_fixture("node_hn3.json");
        let node = node_of_item(&item).expect("decode node_hn3");
        let reencoded = node_to_item(&node);
        assert_items_eq(&item, &reencoded, "node_hn3 roundtrip");
    }

    // -----------------------------------------------------------------------
    // sensor fixture
    // -----------------------------------------------------------------------

    #[test]
    fn sensor_decode() {
        let item = load_fixture("sensor.json");
        let sensor = sensor_of_item(&item).expect("decode sensor");
        assert_eq!(sensor.id.to_string(), "S#10010");
        assert_eq!(sensor.purpose, "Energy");
        assert_eq!(sensor.daq_id, "daq:gwb143_json_v1:6003111553:90143675:0");
        assert!(matches!(sensor.meter_type, MeterType::Counter));
        assert_eq!(sensor.unit, Some("KWh".to_string()));
        assert_eq!(sensor.resample_minutes, Some(5));
        assert!(matches!(sensor.formula, Formula::Identity));
        assert_eq!(
            sensor.path,
            "HN0#root|HN1#10001|HN2#10003|HN3#10004|HN4#10001|S#10010"
        );
    }

    #[test]
    fn sensor_roundtrip() {
        let item = load_fixture("sensor.json");
        let sensor = sensor_of_item(&item).expect("decode sensor");
        let reencoded = sensor_to_item(&sensor);
        assert_items_eq(&item, &reencoded, "sensor roundtrip");
    }

    // -----------------------------------------------------------------------
    // user fixture
    // -----------------------------------------------------------------------

    #[test]
    fn user_decode() {
        let item = load_fixture("user.json");
        let user = user_of_item(&item).expect("decode user");
        assert_eq!(user.id.to_string(), "U#steen666@gmail.com");
        assert_eq!(user.email, "steen666@gmail.com");
        assert_eq!(user.name, "Steen Larsen");
        assert_eq!(user.cognito_group, CognitoGroup::Admin);
        assert_eq!(user.language, Language::Danish);
        assert_eq!(user.currency, Currency::Dkk);
    }

    #[test]
    fn user_roundtrip() {
        let item = load_fixture("user.json");
        let user = user_of_item(&item).expect("decode user");
        let reencoded = user_to_item(&user);
        assert_items_eq(&item, &reencoded, "user roundtrip");
    }

    // -----------------------------------------------------------------------
    // edge_has_property fixture
    // -----------------------------------------------------------------------

    #[test]
    fn edge_has_property_decode() {
        let item = load_fixture("edge_has_property.json");
        let (from_, to_, kind, name, _created, gsi1pk, gsi1sk) =
            edge_of_item(&item).expect("decode edge_has_property");
        assert_eq!(from_, "HN2#10003");
        assert_eq!(to_, "HN3#10004");
        assert_eq!(kind, EdgeKind::HasLabel("property".to_string()));
        assert_eq!(name, "Prop01");
        // gsi1pk = "HN3" (child's level)
        assert_eq!(gsi1pk.as_deref(), Some("HN3"));
        // gsi1sk = child's path
        assert_eq!(
            gsi1sk.as_deref(),
            Some("HN0#root|HN1#10001|HN2#10003|HN3#10004")
        );
    }

    #[test]
    fn edge_has_property_roundtrip() {
        let item = load_fixture("edge_has_property.json");
        let (from_, to_, kind, name, created, _gsi1pk, gsi1sk) =
            edge_of_item(&item).expect("decode edge_has_property");
        let self_path = gsi1sk.expect("gsi1sk required for anchor edge");
        let reencoded = anchor_edge_to_item(AnchorEdgeParams {
            from_: &from_,
            to_: &to_,
            kind: &kind,
            name: &name,
            created: &created,
            self_path: &self_path,
        });
        assert_items_eq(&item, &reencoded, "edge_has_property roundtrip");
    }

    // -----------------------------------------------------------------------
    // Reads / Writes edge codec unit tests (new access edge kinds)
    // -----------------------------------------------------------------------

    /// A `Reads` edge encodes to sk="reads#<node>", gsi1sk="readers#<user>",
    /// kind="reads" and decodes back.
    #[test]
    fn edge_reads_codec_roundtrip() {
        use chrono::DateTime;

        let user_s = "U#reader@ex";
        let node_s = "HN2#10002";
        let created: chrono::DateTime<chrono::Utc> = "2026-01-01T00:00:00Z".parse().unwrap();

        let item = edge_to_item(EdgeParams {
            from_: user_s,
            to_: node_s,
            kind: &EdgeKind::Reads,
            name: "",
            created: &created,
        });

        // sk = "reads#HN2#10002"
        assert_eq!(
            item.get("sk").and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None }),
            Some("reads#HN2#10002"),
            "Reads sk"
        );
        // kind = "reads"
        assert_eq!(
            item.get("kind").and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None }),
            Some("reads"),
            "Reads kind"
        );
        // gsi1pk = node_s, gsi1sk = "readers#<user>"
        assert_eq!(
            item.get("gsi1pk").and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None }),
            Some(node_s),
            "Reads gsi1pk"
        );
        assert_eq!(
            item.get("gsi1sk").and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None }),
            Some("readers#U#reader@ex"),
            "Reads gsi1sk"
        );

        // Decode round-trip.
        let (from_, to_, kind, _name, _created, _gsi1pk, gsi1sk) =
            edge_of_item(&item).expect("decode Reads edge");
        assert_eq!(from_, user_s);
        assert_eq!(to_, node_s);
        assert_eq!(kind, EdgeKind::Reads);
        assert_eq!(gsi1sk.as_deref(), Some("readers#U#reader@ex"));
    }

    /// A `Writes` edge encodes to sk="writes#<node>", gsi1sk="writers#<user>",
    /// kind="writes" and decodes back.
    #[test]
    fn edge_writes_codec_roundtrip() {
        let user_s = "U#writer@ex";
        let node_s = "HN2#10003";
        let created: chrono::DateTime<chrono::Utc> = "2026-01-01T00:00:00Z".parse().unwrap();

        let item = edge_to_item(EdgeParams {
            from_: user_s,
            to_: node_s,
            kind: &EdgeKind::Writes,
            name: "",
            created: &created,
        });

        assert_eq!(
            item.get("sk").and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None }),
            Some("writes#HN2#10003"),
        );
        assert_eq!(
            item.get("kind").and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None }),
            Some("writes"),
        );
        assert_eq!(
            item.get("gsi1sk").and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None }),
            Some("writers#U#writer@ex"),
        );

        let (from_, to_, kind, _name, _created, _gsi1pk, gsi1sk) =
            edge_of_item(&item).expect("decode Writes edge");
        assert_eq!(from_, user_s);
        assert_eq!(to_, node_s);
        assert_eq!(kind, EdgeKind::Writes);
        assert_eq!(gsi1sk.as_deref(), Some("writers#U#writer@ex"));
    }

    // -----------------------------------------------------------------------
    // edge_administrates fixture
    // -----------------------------------------------------------------------

    #[test]
    fn edge_administrates_decode() {
        let item = load_fixture("edge_administrates.json");
        let (from_, to_, kind, name, _created, gsi1pk, gsi1sk) =
            edge_of_item(&item).expect("decode edge_administrates");
        assert_eq!(from_, "U#stel@enity.io");
        assert_eq!(to_, "HN2#10002");
        assert_eq!(kind, EdgeKind::Administrates);
        assert_eq!(name, "");
        // gsi1pk = to_ (the target node id)
        assert_eq!(gsi1pk.as_deref(), Some("HN2#10002"));
        // gsi1sk = "administrators#<from_>"
        assert_eq!(gsi1sk.as_deref(), Some("administrators#U#stel@enity.io"));
    }

    #[test]
    fn edge_administrates_roundtrip() {
        let item = load_fixture("edge_administrates.json");
        let (from_, to_, kind, name, created, _gsi1pk, _gsi1sk) =
            edge_of_item(&item).expect("decode edge_administrates");
        let reencoded = edge_to_item(EdgeParams {
            from_: &from_,
            to_: &to_,
            kind: &kind,
            name: &name,
            created: &created,
        });
        assert_items_eq(&item, &reencoded, "edge_administrates roundtrip");
    }

    // -----------------------------------------------------------------------
    // Proptest: encode ∘ decode = identity for Node
    // -----------------------------------------------------------------------

    fn arb_level() -> impl Strategy<Value = Level> {
        (0u8..=5).prop_map(|d| Level::of_depth(d).unwrap())
    }

    fn arb_node_id() -> impl Strategy<Value = NodeId> {
        prop_oneof![
            Just(NodeId::Root),
            (arb_level(), 1u32..500u32).prop_map(|(lvl, id)| NodeId::make(lvl, id)),
        ]
    }

    fn arb_ascii_name() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 ]{1,20}".prop_map(|s| s)
    }

    fn arb_node() -> impl Strategy<Value = Node> {
        (arb_node_id(), arb_ascii_name(), 0i64..2000000000i64).prop_map(
            |(id, name, ts_secs)| {
                let created = DateTime::from_timestamp(ts_secs, 0).unwrap_or(DateTime::UNIX_EPOCH);
                let path = id.to_string(); // minimal path = just self
                Node {
                    id,
                    name,
                    parent: None,
                    path,
                    created,
                    metadata: serde_json::json!({}),
                    schema: None,
                }
            },
        )
    }

    proptest! {
        #[test]
        fn prop_node_encode_decode(n in arb_node()) {
            let item = node_to_item(&n);
            let decoded = node_of_item(&item).expect("decode should succeed");
            prop_assert_eq!(decoded.id, n.id);
            prop_assert_eq!(decoded.name, n.name);
            prop_assert_eq!(decoded.path, n.path);
            prop_assert_eq!(decoded.schema, n.schema);
        }
    }

    // -----------------------------------------------------------------------
    // Proptest: encode ∘ decode = identity for User
    // -----------------------------------------------------------------------

    fn arb_cognito_group() -> impl Strategy<Value = CognitoGroup> {
        prop_oneof![
            Just(CognitoGroup::Reader),
            Just(CognitoGroup::Writer),
            Just(CognitoGroup::Admin),
        ]
    }

    fn arb_user() -> impl Strategy<Value = User> {
        (
            "[a-z]{3,8}@[a-z]{3,8}\\.[a-z]{2,4}",
            arb_ascii_name(),
            arb_cognito_group(),
            0i64..2000000000i64,
        )
            .prop_map(|(email, name, cognito_group, ts_secs)| {
                let created =
                    DateTime::from_timestamp(ts_secs, 0).unwrap_or(DateTime::UNIX_EPOCH);
                let id = UserId::of_email(&email);
                User {
                    email,
                    id,
                    name,
                    cognito_group,
                    language: Language::Danish,
                    currency: Currency::Dkk,
                    created,
                }
            })
    }

    proptest! {
        #[test]
        fn prop_user_encode_decode(u in arb_user()) {
            let item = user_to_item(&u);
            let decoded = user_of_item(&item).expect("decode should succeed");
            prop_assert_eq!(decoded.email, u.email);
            prop_assert_eq!(decoded.name, u.name);
            prop_assert_eq!(decoded.cognito_group, u.cognito_group);
            prop_assert_eq!(decoded.id.to_string(), u.id.to_string());
        }
    }
}
