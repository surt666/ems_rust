//! DynamoDB codec — maps domain structs ⟷ live DynamoDB item format.
//!
//! Every attribute name and encoding choice (N strings, nested-M structure, sk
//! verbs, …) must match the live item format exactly so that the lambda can read
//! and write the 8,508 live items without migration.

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
use crate::domain::values::{
    CognitoGroup, Currency, EdgeKind, FieldType, Language, MeterType, Resource,
};
use crate::errors::RepositoryError;

// ---------------------------------------------------------------------------
// Public type alias
// ---------------------------------------------------------------------------

/// A DynamoDB item: attribute name → value.
pub type Item = HashMap<String, AttributeValue>;

// ---------------------------------------------------------------------------
// Low-level helpers (`s`, `n`, `b`)
// ---------------------------------------------------------------------------

fn s(v: impl Into<String>) -> AttributeValue {
    AttributeValue::S(v.into())
}

fn n(v: impl Into<String>) -> AttributeValue {
    AttributeValue::N(v.into())
}

const fn b(v: bool) -> AttributeValue {
    AttributeValue::Bool(v)
}

/// Format a `DateTime<Utc>` as RFC3339 with `Z` suffix.
fn dt_to_rfc3339z(dt: &DateTime<Utc>) -> String {
    let s = dt.to_rfc3339();
    if let Some(stripped) = s.strip_suffix("+00:00") {
        format!("{}Z", stripped)
    } else {
        s
    }
}

/// Parse an RFC3339 string; fall back to epoch on error.
fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(DateTime::UNIX_EPOCH)
}

// ---------------------------------------------------------------------------
// Field-extraction helpers (`field`, `as_string`, …)
// ---------------------------------------------------------------------------

fn field<'a>(item: &'a Item, k: &str) -> Result<&'a AttributeValue, RepositoryError> {
    item.get(k).ok_or_else(|| RepositoryError::Codec(format!("missing field {:?}", k)))
}

/// Extract an optional `S` attribute as an owned `String`.
fn opt_s(item: &Item, k: &str) -> Option<String> {
    match item.get(k) {
        Some(AttributeValue::S(v)) => Some(v.clone()),
        _ => None,
    }
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

/// Parse an optional numeric `N` attribute into any `FromStr` type.
fn opt_num_of_n<T: std::str::FromStr>(v: Option<&AttributeValue>) -> Option<T> {
    match v {
        Some(AttributeValue::N(s)) => s.parse::<T>().ok(),
        _ => None,
    }
}

/// `%.17g` — 17 significant digits, shortest representation.
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
    format_g17(f)
}

/// Reproduce `%.17g`: up to 17 significant digits, no trailing zeros,
/// scientific for |exp| >= 17 or exp < -4.
fn format_g17(f: f64) -> String {
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
// GSI key helpers (node_gsi1pk / sensor_gsi1pk)
// ---------------------------------------------------------------------------

pub(crate) fn node_gsi1pk(lvl: Level) -> String {
    format!("HN{}", lvl.depth())
}

/// GSI1 partition key for a sensor (and its `has_sensor` edge), scoped to the
/// owning company (the HN2 segment of `path`) so the sensor index shards per
/// company instead of piling every sensor in the system into one hot `"S"`
/// partition. A company-wide listing (formula picker / Målere) is then a single
/// `gsi1pk = "S#HN2#<id>"` query rather than a `begins_with` over the world.
///
/// `"HN0#root|HN1#10|HN2#200|HN3#1|S#1"` -> `"S#HN2#200"`. A well-formed sensor
/// always sits under a company; if no HN2 segment is present we fall back to a
/// bare `"S"` so a malformed row is never silently unindexed.
pub(crate) fn sensor_gsi1pk(path: &str) -> String {
    match hn2_segment(path) {
        Some(seg) => format!("S#{seg}"),
        None => "S".to_string(),
    }
}

/// The `HN2#<id>` (company) segment of a hierarchy path, if present.
pub(crate) fn hn2_segment(path: &str) -> Option<String> {
    path.split('|')
        .filter(|s| !s.is_empty())
        .find(|seg| matches!(NodeId::parse(seg), Ok(nid) if nid.level() == Level::Hn2))
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Formula encode/decode (expr_to_attr / formula_to_attr)
// ---------------------------------------------------------------------------

/// Encode a binary expression node `{t, l, r}`.
fn binop(tag: &str, l: &Expr, r: &Expr) -> AttributeValue {
    let mut m: HashMap<String, AttributeValue> = HashMap::new();
    m.insert("t".to_string(), s(tag));
    m.insert("l".to_string(), expr_to_av(l));
    m.insert("r".to_string(), expr_to_av(r));
    AttributeValue::M(m)
}

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
        Expr::Add(a, b) => binop("add", a, b),
        Expr::Sub(a, b) => binop("sub", a, b),
        Expr::Mul(a, b) => binop("mul", a, b),
        Expr::Div(a, b) => binop("div", a, b),
    }
}

fn expr_of_av(v: &AttributeValue) -> Result<Expr, RepositoryError> {
    let kvs = as_m(v)?;
    let tag = as_s(field(kvs, "t")?)?;
    match tag {
        "num" => {
            let nv = field(kvs, "v")?;
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
            let a = as_s(field(kvs, "a")?)?;
            Ok(Expr::Ref(a.to_string()))
        }
        "abs" => {
            let e = field(kvs, "e")?;
            Ok(Expr::Abs(Box::new(expr_of_av(e)?)))
        }
        "add" | "sub" | "mul" | "div" => {
            let ctor: fn(Box<Expr>, Box<Expr>) -> Expr = match tag {
                "add" => Expr::Add,
                "sub" => Expr::Sub,
                "mul" => Expr::Mul,
                _ => Expr::Div,
            };
            let l = expr_of_av(field(kvs, "l")?)?;
            let r = expr_of_av(field(kvs, "r")?)?;
            Ok(ctor(Box::new(l), Box::new(r)))
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
    let kind = as_s(field(kvs, "kind")?)?;
    match kind {
        "identity" => Ok(Formula::Identity),
        "zero" => Ok(Formula::Zero),
        "expr" => {
            let ast = expr_of_av(field(kvs, "ast")?)?;
            let refs_av = field(kvs, "refs")?;
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
            // Preserve insertion order
            Ok(Formula::Expr { expr: ast, refs })
        }
        other => Err(RepositoryError::Codec(format!("unknown formula kind {:?}", other))),
    }
}

// ---------------------------------------------------------------------------
// Schema encode/decode (schema_to_attr / decode_schema)
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
    // spec map: start with the field_type map, then add "required"
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

/// Encode a `Schema` (v2) as a nested `M` AttributeValue.
/// edges: M{ parent_type -> M{ child_type -> M{min?,max?} } }
pub fn schema_to_av(sch: &Schema) -> AttributeValue {
    let edges_m: HashMap<String, AttributeValue> = sch
        .edges
        .iter()
        .map(|(parent, children)| {
            let inner: HashMap<String, AttributeValue> = children
                .iter()
                .map(|(child, spec)| (child.clone(), edge_spec_to_av(spec)))
                .collect();
            (parent.clone(), AttributeValue::M(inner))
        })
        .collect();

    let metadata_m: HashMap<String, AttributeValue> = sch
        .metadata
        .iter()
        .map(|(typ, fields)| {
            let inner: HashMap<String, AttributeValue> = fields
                .iter()
                .map(|(name, fs)| (name.clone(), field_spec_to_av(fs)))
                .collect();
            (typ.clone(), AttributeValue::M(inner))
        })
        .collect();

    let sensors_l: Vec<AttributeValue> =
        sch.sensors.iter().map(|t| s(t.clone())).collect();

    let mut m: HashMap<String, AttributeValue> = HashMap::new();
    m.insert("version".to_string(), n(sch.version.to_string()));
    m.insert("edges".to_string(), AttributeValue::M(edges_m));
    m.insert("metadata".to_string(), AttributeValue::M(metadata_m));
    m.insert("sensors".to_string(), AttributeValue::L(sensors_l));
    AttributeValue::M(m)
}

fn decode_field_spec(v: &AttributeValue) -> Result<FieldSpec, RepositoryError> {
    let kvs = as_m(v)?;
    let typ_s = as_s(field(kvs, "type")?)?;
    let required = match kvs.get("required") {
        Some(AttributeValue::Bool(b)) => *b,
        _ => false,
    };
    let typ = match typ_s {
        "string" => FieldType::String {
            min_len: opt_num_of_n::<i64>(kvs.get("min_len")),
            max_len: opt_num_of_n::<i64>(kvs.get("max_len")),
        },
        "number" => FieldType::Number {
            min: opt_num_of_n::<f64>(kvs.get("min")),
            max: opt_num_of_n::<f64>(kvs.get("max")),
        },
        "integer" => FieldType::Integer {
            min: opt_num_of_n::<i64>(kvs.get("min")),
            max: opt_num_of_n::<i64>(kvs.get("max")),
        },
        "boolean" => FieldType::Boolean,
        "timestamp" => FieldType::Timestamp,
        "enum" => {
            let xs = as_l(field(kvs, "one_of")?)?;
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

fn decode_edge_spec(body: &AttributeValue) -> Result<EdgeSpec, RepositoryError> {
    let kvs = as_m(body)?;
    Ok(EdgeSpec {
        min: opt_num_of_n::<i32>(kvs.get("min")),
        max: opt_num_of_n::<i32>(kvs.get("max")),
    })
}

/// Decode a `Schema` (v2 only) from a nested `M` AttributeValue.
/// Version-1 (level-keyed) schemas are rejected with an explicit error.
#[allow(clippy::type_complexity)]
pub fn schema_of_av(v: &AttributeValue) -> Result<Schema, RepositoryError> {
    let kvs = as_m(v)?;
    let version = match field(kvs, "version")? {
        AttributeValue::N(s) => s.parse::<u32>().map_err(|_| {
            RepositoryError::Codec("bad version N".to_string())
        })?,
        _ => return Err(RepositoryError::Codec("expected N for version".to_string())),
    };
    if version != 2 {
        return Err(RepositoryError::Codec(format!(
            "schema version {} — run the v2 migration (scripts/migrate_schema_v2.py); only version 2 is supported",
            version
        )));
    }

    // DynamoDB `M` maps are unordered and Rust's `HashMap` iteration order is
    // randomized per instance, so decode every level and sort by key. This keeps
    // the schema's field/child order stable across decodes (no per-reload
    // shuffling) and matches the deterministic order `schema_of_json` produces.
    let edges_kvs = as_m(field(kvs, "edges")?)?;
    let mut edges: Vec<(String, Vec<(String, EdgeSpec)>)> = Vec::new();
    for (parent, inner_v) in edges_kvs {
        let inner_kvs = as_m(inner_v)?;
        let mut children: Vec<(String, EdgeSpec)> = Vec::new();
        for (child, body) in inner_kvs {
            children.push((child.clone(), decode_edge_spec(body)?));
        }
        children.sort_by(|a, b| a.0.cmp(&b.0));
        edges.push((parent.clone(), children));
    }
    edges.sort_by(|a, b| a.0.cmp(&b.0));

    let metadata = match kvs.get("metadata") {
        None => vec![],
        Some(m_v) => {
            let m_kvs = as_m(m_v)?;
            let mut result: Vec<(String, Vec<(String, FieldSpec)>)> = Vec::new();
            for (typ, inner_v) in m_kvs {
                let inner_kvs = as_m(inner_v)?;
                let mut fields: Vec<(String, FieldSpec)> = Vec::new();
                for (fname, spec_v) in inner_kvs {
                    fields.push((fname.clone(), decode_field_spec(spec_v)?));
                }
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                result.push((typ.clone(), fields));
            }
            result.sort_by(|a, b| a.0.cmp(&b.0));
            result
        }
    };

    let sensors = match kvs.get("sensors") {
        None => vec![],
        Some(v) => {
            let xs = as_l(v)?;
            let mut result: Vec<String> = Vec::new();
            for item in xs {
                result.push(as_s(item)?.to_string());
            }
            result
        }
    };

    Ok(Schema { version, edges, metadata, sensors })
}

// ---------------------------------------------------------------------------
// Node encode/decode
// ---------------------------------------------------------------------------

/// Encode a `Node` as a DynamoDB `Item`.
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
            .unwrap_or_else(|_| AttributeValue::M(HashMap::new())),
    );
    item.insert("gsi1pk".to_string(), s(node_gsi1pk(nd.level())));
    item.insert("gsi1sk".to_string(), s(nd.path.clone()));
    if !nd.label.is_empty() {
        item.insert("label".to_string(), s(nd.label.clone()));
    }
    if let Some(ref sch) = nd.schema {
        item.insert("schema".to_string(), schema_to_av(sch));
    }
    item
}

/// Decode a `Node` from a DynamoDB `Item`.
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
        Some(v) => Some(schema_of_av(v)?),
    };
    let path = opt_s(item, "gsi1sk")
        .ok_or_else(|| RepositoryError::Codec(format!("node {} missing gsi1sk/path", id)))?;
    let parent = parent_from_path(&path);
    let label = opt_s(item, "label").unwrap_or_default();
    Ok(Node::builder()
        .id(id)
        .name(name)
        .parent(parent)
        .path(path)
        .created(created)
        .metadata(metadata)
        .schema(schema)
        .label(label)
        .build())
}

/// Extract the parent `NodeId` from a path string.
/// The second-to-last pipe-separated segment.
fn parent_from_path(path: &str) -> Option<NodeId> {
    let parts: Vec<&str> = path.split('|').filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [.., par, _self] => NodeId::parse(par).ok(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Edge encode (edge_item / edge_with_anchor)
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
pub fn anchor_edge_to_item(p: AnchorEdgeParams<'_>) -> Item {
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
    // GSI projection is SPARSE. Only node-child edges (`has_<label>`) are indexed,
    // keyed by the child's level — that's how the subtree walk finds child nodes.
    // `has_sensor` edges are deliberately NOT projected: the sensor GSI partition
    // (`S#HN2#<company>`) must hold *only active sensors*, so a company-wide sensor
    // Query reads no edge/history noise (less RCU, nothing to filter). The edge is
    // still fully reachable on the base table (`pk=parent, sk=has_sensor#<id>`) —
    // that's how `list_child_refs` reads it and how delete derives its key.
    match p.kind {
        EdgeKind::HasSensor => {}
        EdgeKind::HasLabel(_) => {
            let gsi1pk_v = match NodeId::parse(p.to_) {
                Ok(child_id) => node_gsi1pk(child_id.level()),
                Err(_) => panic!("anchor_edge_to_item: bad to_ {:?}", p.to_),
            };
            item.insert("gsi1pk".to_string(), s(gsi1pk_v));
            item.insert("gsi1sk".to_string(), s(p.self_path));
        }
        _ => panic!("anchor_edge_to_item: only HN-side edges"),
    }
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
    let gsi1pk = opt_s(item, "gsi1pk");
    let gsi1sk = opt_s(item, "gsi1sk");
    Ok((from_, to_, kind, name, created, gsi1pk, gsi1sk))
}

// ---------------------------------------------------------------------------
// Sensor encode/decode
// ---------------------------------------------------------------------------

/// Encode a `Sensor` as a DynamoDB `Item` (always as the `active` SK).
pub fn sensor_to_item(sn: &Sensor) -> Item {
    let pk = sn.id.to_string();
    let sk = SensorSk::Active(sn.created).to_string();
    let mut item: Item = HashMap::new();
    item.insert("pk".to_string(), s(pk));
    item.insert("sk".to_string(), s(sk));
    item.insert("type".to_string(), s("sensor"));
    item.insert("daq_id".to_string(), s(sn.daq_id.clone()));
    item.insert("gsi1pk".to_string(), s(sensor_gsi1pk(&sn.path)));
    item.insert("gsi1sk".to_string(), s(sn.path.clone()));
    item.insert("purpose".to_string(), s(sn.purpose.to_string()));
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

/// Encode a *history* (superseded) sensor row: identical payload to the active item
/// but with a bare-timestamp sk and **no GSI projection**. History is queryable on the
/// base `pk=S#<id>` partition; keeping it out of the sensor GSI keeps that partition
/// sparse (active sensors only). `superseded_created` is the `created` of the version
/// being retired (its original active timestamp).
pub fn sensor_history_to_item(sn: &Sensor, superseded_created: DateTime<Utc>) -> Item {
    let mut item = sensor_to_item(sn);
    item.insert("sk".to_string(), s(SensorSk::History(superseded_created).to_string()));
    item.remove("gsi1pk");
    item.remove("gsi1sk");
    item
}

// ---------------------------------------------------------------------------
// Daq lock — a physical daq feeds at most one *active* logical meter.
//
// Stored as an overloaded base-table row keyed by the daq:
//   pk = "DAQ#<daq_id>"   sk = "belongs_to#S#<logical_id>"   path = <full path>
// One row per daq partition, so `Query pk="DAQ#<daq>"` returns ≤ 1 — that count
// is the uniqueness invariant. Enforced by a pre-fetch guard (not a conditional
// write), so the row is written unconditionally alongside the sensor.
// ---------------------------------------------------------------------------

/// Partition key for a daq lock row: `"DAQ#<daq_id>"`.
pub(crate) fn daq_lock_pk(daq_id: &str) -> String {
    format!("DAQ#{daq_id}")
}

/// Sort key naming the owning logical sensor: `"belongs_to#S#<id>"`.
fn daq_lock_sk(sensor_id: SensorId) -> String {
    format!("belongs_to#{sensor_id}")
}

/// The full `{pk, sk}` key of the daq lock row — for `Delete`.
pub(crate) fn daq_lock_key(daq_id: &str, sensor_id: SensorId) -> Item {
    let mut key: Item = HashMap::new();
    key.insert("pk".to_string(), s(daq_lock_pk(daq_id)));
    key.insert("sk".to_string(), s(daq_lock_sk(sensor_id)));
    key
}

/// Build the daq lock row asserting `daq_id` belongs to `sensor_id` at `path`.
pub(crate) fn daq_lock_item(daq_id: &str, sensor_id: SensorId, path: &str) -> Item {
    let mut item = daq_lock_key(daq_id, sensor_id);
    item.insert("type".to_string(), s("daq_lock"));
    item.insert("path".to_string(), s(path));
    item
}

/// The daq lock row for a freshly-encoded sensor item (the active row carries
/// `daq_id`, `pk` = `S#<id>`, `gsi1sk` = path). `None` if it has no daq.
pub(crate) fn daq_lock_of_sensor_item(item: &Item) -> Option<Item> {
    let daq = opt_s(item, "daq_id")?;
    let sensor_id = SensorId::parse(&opt_s(item, "pk")?).ok()?;
    let path = opt_s(item, "gsi1sk").unwrap_or_default();
    Some(daq_lock_item(&daq, sensor_id, &path))
}

/// Decode a daq lock row into its owning `(logical id, path)`.
pub(crate) fn daq_lock_owner(item: &Item) -> Option<(SensorId, String)> {
    let id = SensorId::parse(opt_s(item, "sk")?.strip_prefix("belongs_to#")?).ok()?;
    Some((id, opt_s(item, "path").unwrap_or_default()))
}

/// Decode a `Sensor` from a DynamoDB `Item`.
pub fn sensor_of_item(item: &Item) -> Result<Sensor, RepositoryError> {
    let pk_s = as_s(field(item, "pk")?)?;
    let id = SensorId::parse(pk_s)
        .map_err(|e| RepositoryError::Codec(format!("bad sensor id {:?}: {}", pk_s, e)))?;
    let daq_id = as_s(field(item, "daq_id")?)?.to_string();
    let path = as_s(field(item, "gsi1sk")?)?.to_string();
    let purpose_s = as_s(field(item, "purpose")?)?;
    let purpose = purpose_s
        .parse::<Resource>()
        .map_err(|e| RepositoryError::Codec(format!("bad purpose {:?}: {}", purpose_s, e)))?;
    let mt_s = as_s(field(item, "meter_type")?)?;
    let meter_type = mt_s.parse::<MeterType>()
        .map_err(|e| RepositoryError::Codec(format!("bad meter_type {:?}: {}", mt_s, e)))?;
    let unit = opt_s(item, "unit");
    let created_s = as_s(field(item, "created")?)?;
    let created = parse_ts(created_s);
    let formula = match item.get("formula") {
        Some(v) => formula_of_av(v)?,
        None => Formula::Identity,
    };
    let resample_minutes = opt_num_of_n::<i32>(item.get("resample_minutes"));
    Ok(Sensor::builder()
        .id(id)
        .created(created)
        .daq_id(daq_id)
        .path(path)
        .purpose(purpose)
        .meter_type(meter_type)
        .unit(unit)
        .formula(formula)
        .resample_minutes(resample_minutes)
        .build())
}

// ---------------------------------------------------------------------------
// User encode/decode
// ---------------------------------------------------------------------------

/// Encode a `User` as a DynamoDB `Item`.
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
pub fn user_of_item(item: &Item) -> Result<User, RepositoryError> {
    let pk_s = as_s(field(item, "pk")?)?;
    let id = UserId::parse(pk_s)
        .map_err(|e| RepositoryError::Codec(format!("bad user id {:?}: {}", pk_s, e)))?;
    let email = id.email().to_string();
    let name = as_s(field(item, "name")?)?.to_string();
    let g_s = as_s(field(item, "cognito_group")?)?;
    let cognito_group = g_s.parse::<CognitoGroup>()
        .map_err(|e| RepositoryError::Codec(format!("bad cognito_group {:?}: {}", g_s, e)))?;
    let language = opt_s(item, "language")
        .and_then(|v| v.parse::<Language>().ok())
        .unwrap_or_default();
    let currency = opt_s(item, "currency")
        .and_then(|v| v.parse::<Currency>().ok())
        .unwrap_or_default();
    let created_s = as_s(field(item, "created")?)?;
    let created = parse_ts(created_s);
    Ok(User::builder()
        .email(email)
        .id(id)
        .name(name)
        .cognito_group(cognito_group)
        .language(language)
        .currency(currency)
        .created(created)
        .build())
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
        assert_eq!(sch.version, 2);
        assert_eq!(
            sch.sensors.iter().collect::<std::collections::HashSet<_>>(),
            ["building".to_string(), "area".to_string()].iter().collect()
        );
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
    // node label codec tests
    // -----------------------------------------------------------------------

    #[test]
    fn node_label_roundtrip() {
        let mut nd = crate::domain::node::make(
            7, Level::Hn3, "B1", NodeId::parse("HN2#1").unwrap(),
            "HN0#root|HN1#1|HN2#1", serde_json::json!({}), None,
        );
        nd.label = "building".to_string();
        let item = node_to_item(&nd);
        let back = node_of_item(&item).unwrap();
        assert_eq!(back.label, "building");
    }

    #[test]
    fn node_missing_label_decodes_empty() {
        let nd = crate::domain::node::make(
            8, Level::Hn3, "B2", NodeId::parse("HN2#1").unwrap(),
            "HN0#root|HN1#1|HN2#1", serde_json::json!({}), None,
        );
        let item = node_to_item(&nd); // label empty → attribute omitted
        let back = node_of_item(&item).unwrap();
        assert_eq!(back.label, "");
    }

    /// An item with an unmigrated v1 schema must surface the migration-hint
    /// Codec error through `node_of_item` (the decode path `get_node` uses) —
    /// never decode to a schema-less node or collapse to absence.
    #[test]
    fn node_with_v1_schema_errors_with_migration_hint() {
        let nd = crate::domain::node::make(
            9, Level::Hn2, "OldCo", NodeId::parse("HN1#1").unwrap(),
            "HN0#root|HN1#1", serde_json::json!({}), None,
        );
        let mut item = node_to_item(&nd);
        let mut v1: HashMap<String, AttributeValue> = HashMap::new();
        v1.insert("version".to_string(), n("1".to_string()));
        v1.insert("edges".to_string(), AttributeValue::M(HashMap::new()));
        item.insert("schema".to_string(), AttributeValue::M(v1));

        let err = node_of_item(&item).unwrap_err();
        assert!(matches!(err, RepositoryError::Codec(_)), "got: {:?}", err);
        let msg = format!("{:?}", err);
        assert!(msg.contains("migration"), "got: {}", msg);
        assert!(msg.contains("version 1"), "got: {}", msg);
    }

    // -----------------------------------------------------------------------
    // schema v2 encode/decode
    // -----------------------------------------------------------------------

    /// Canonical v2 type-graph fixture (matches the model domain fixture).
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
                    ("area".to_string(), EdgeSpec::builder().min(Some(1)).build()),
                ]),
            ],
            metadata: vec![(
                "building".to_string(),
                vec![("lat".to_string(), FieldSpec {
                    typ: FieldType::Number { min: Some(-90.0), max: Some(90.0) },
                    required: true,
                })],
            )],
            sensors: vec!["building".to_string(), "area".to_string()],
        }
    }

    #[test]
    fn schema_v1_rejected_with_migration_hint() {
        // hand-build a v1-shaped attribute: version 1
        let mut m: HashMap<String, AttributeValue> = HashMap::new();
        m.insert("version".to_string(), n("1".to_string()));
        m.insert("edges".to_string(), AttributeValue::M(HashMap::new()));
        let err = schema_of_av(&AttributeValue::M(m)).unwrap_err();
        let msg = format!("{:?}", err);
        assert!(msg.contains("version 1"), "got: {}", msg);
        assert!(msg.contains("migration"), "got: {}", msg);
    }

    #[test]
    fn schema_v2_roundtrip() {
        let s0 = sample_schema(); // v2 fixture
        let av = schema_to_av(&s0);
        let s1 = schema_of_av(&av).unwrap();
        // av maps are unordered — compare as sets
        assert_eq!(s1.version, 2);
        assert_eq!(s1.edges.len(), s0.edges.len());
        for (parent, children) in &s0.edges {
            let dec = s1.edges.iter().find(|(p, _)| p == parent).expect(parent);
            assert_eq!(dec.1.len(), children.len());
        }
        assert_eq!(
            s1.sensors.iter().collect::<std::collections::HashSet<_>>(),
            s0.sensors.iter().collect::<std::collections::HashSet<_>>()
        );
    }

    /// Decoding sorts metadata fields and edge children by name, so the order is
    /// stable across decodes (DynamoDB `M` + HashMap iteration is otherwise
    /// randomized) regardless of the order they were encoded in.
    #[test]
    fn schema_decode_orders_fields_and_children_deterministically() {
        let num = || FieldSpec {
            typ: FieldType::Number { min: None, max: None },
            required: false,
        };
        let s0 = Schema {
            version: 2,
            edges: vec![("company".to_string(), vec![
                ("zulu".to_string(), EdgeSpec::builder().build()),
                ("alpha".to_string(), EdgeSpec::builder().build()),
                ("mike".to_string(), EdgeSpec::builder().build()),
            ])],
            // fields deliberately NOT in alphabetical order
            metadata: vec![("company".to_string(), vec![
                ("lng".to_string(), num()),
                ("lat".to_string(), num()),
                ("alt".to_string(), num()),
            ])],
            sensors: vec![],
        };
        let dec = schema_of_av(&schema_to_av(&s0)).unwrap();

        let fields: Vec<&str> = dec.metadata[0].1.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(fields, vec!["alt", "lat", "lng"], "fields must be name-sorted");

        let children: Vec<&str> =
            dec.edges.iter().find(|(p, _)| p == "company").unwrap().1.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(children, vec!["alpha", "mike", "zulu"], "children must be name-sorted");
    }

    // -----------------------------------------------------------------------
    // sensor fixture
    // -----------------------------------------------------------------------

    #[test]
    fn sensor_decode() {
        let item = load_fixture("sensor.json");
        let sensor = sensor_of_item(&item).expect("decode sensor");
        assert_eq!(sensor.id.to_string(), "S#10010");
        assert_eq!(sensor.purpose, Resource::Electricity);
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
    // Sparse sensor GSI: has_sensor edges and history rows must NOT be projected
    // -----------------------------------------------------------------------

    /// A `has_sensor` edge is NOT indexed — no gsi1pk/gsi1sk — so the sensor GSI
    /// partition stays sparse (active sensors only). The base-table sk still points at
    /// the sensor, which is how `list_child_refs` / delete reach it.
    #[test]
    fn has_sensor_edge_is_not_in_gsi() {
        let created: chrono::DateTime<chrono::Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let item = anchor_edge_to_item(AnchorEdgeParams {
            from_: "HN4#10001",
            to_: "S#10011",
            kind: &EdgeKind::HasSensor,
            name: "",
            created: &created,
            self_path: "HN0#root|HN1#10001|HN2#10003|HN3#10004|HN4#10001|S#10011",
        });
        assert_eq!(
            item.get("sk").and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None }),
            Some("has_sensor#S#10011"),
        );
        assert!(!item.contains_key("gsi1pk"), "has_sensor edge must not carry gsi1pk");
        assert!(!item.contains_key("gsi1sk"), "has_sensor edge must not carry gsi1sk");
    }

    /// Active sensor rows ARE in the GSI (that's the anchor); history rows are NOT.
    #[test]
    fn active_sensor_indexed_history_not() {
        let sensor = sensor_of_item(&load_fixture("sensor.json")).expect("decode sensor");
        let active = sensor_to_item(&sensor);
        assert!(active.contains_key("gsi1pk"), "active sensor must be indexed");
        assert!(active.contains_key("gsi1sk"), "active sensor must be indexed");

        let hist = sensor_history_to_item(&sensor, sensor.created);
        assert!(!hist.contains_key("gsi1pk"), "history row must not carry gsi1pk");
        assert!(!hist.contains_key("gsi1sk"), "history row must not carry gsi1sk");
        // sk switched to the bare-timestamp history form (no active# prefix).
        let sk = hist.get("sk").and_then(|v| if let AttributeValue::S(s) = v { Some(s.as_str()) } else { None }).unwrap();
        assert!(!sk.starts_with("active#"), "history sk must not be active#, got {sk}");
    }

    // -----------------------------------------------------------------------
    // Reads / Writes edge codec unit tests (new access edge kinds)
    // -----------------------------------------------------------------------

    /// A `Reads` edge encodes to sk="reads#<node>", gsi1sk="readers#<user>",
    /// kind="reads" and decodes back.
    #[test]
    fn edge_reads_codec_roundtrip() {
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
                Node::builder()
                    .id(id)
                    .name(name)
                    .path(path)
                    .created(created)
                    .build()
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
                User::builder()
                    .email(email)
                    .name(name)
                    .cognito_group(cognito_group)
                    .language(Language::Danish)
                    .currency(Currency::Dkk)
                    .created(created)
                    .build()
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
