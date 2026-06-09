//! Command dispatch — wires each `Command` variant to the model logic layer.
//!
//! Per-command handler functions are generic over their closure dependencies
//! (consistent with the logic layer style: no traits, closures injected).
//!
//! The prod entry-point `run` builds the real repository closures from the
//! shared AWS clients and calls the matching handler.
//!
//! Ported 1:1 from `services/hierarchy/lib/api/api_command.ml`:
//! - `run_create_user`: Profile→group, Users::create, access grants, then
//!   Cognito create + add_to_group; on Cognito failure, roll back the DDB user.
//! - `run_delete_user`: Users::delete (cascade edges) + AdminDeleteUser.
//! - All other handlers delegate straight to the logic layer.
//!
//! See `../../../model/src/logic/` for the logic functions and their closure
//! contracts.

use std::future::Future;

use serde_json::{json, Value};

use model::domain::ids::{NodeId, SensorId, UserId};
use model::domain::node::Node;
use model::domain::sensor::Sensor;
use model::domain::user::User;
use model::domain::values::{CognitoGroup, Currency, EdgeKind, Language, MeterType, Profile};
use model::errors::RepositoryError;
use model::logic::{access, hierarchy, sensors, users};
use model::repository::EdgeSpec;

use crate::command::Command;

// ---------------------------------------------------------------------------
// Response helpers (mirror api_json.ml ok_response / error_response)
// ---------------------------------------------------------------------------

/// Returns the Lambda V2 envelope — `{statusCode, body}`.
/// The body is a JSON string.
fn ok(body: Value) -> Value {
    json!({
        "statusCode": 200,
        "body": body.to_string()
    })
}

fn bad_request(msg: &str) -> Value {
    json!({
        "statusCode": 400,
        "body": json!({
            "error": { "code": "Bad_request", "message": msg }
        }).to_string()
    })
}

fn repo_error_response(e: RepositoryError) -> Value {
    let (status, code, msg) = match &e {
        RepositoryError::NotFound(id) => (404, "Not_found", format!("{} not found", id)),
        RepositoryError::NotFoundUser(id) => (404, "Not_found", format!("{} not found", id)),
        RepositoryError::Conflict(m) => (409, "Conflict", m.clone()),
        RepositoryError::BadRequest(m) => (400, "Bad_request", m.clone()),
        RepositoryError::Validation(errs) => {
            let details: Vec<_> = errs
                .iter()
                .map(|e| json!({"path": e.path, "message": e.message}))
                .collect();
            return json!({
                "statusCode": 400,
                "body": json!({
                    "error": {
                        "code": "Validation",
                        "message": "validation failed",
                        "details": details
                    }
                }).to_string()
            });
        }
        RepositoryError::SchemaMissing(id) => {
            (400, "Schema_missing", format!("no hn2 schema found above {}", id))
        }
        RepositoryError::Codec(m) => (500, "Internal", m.clone()),
        RepositoryError::Aws(m) => (500, "Internal", m.clone()),
    };

    json!({
        "statusCode": status,
        "body": json!({
            "error": { "code": code, "message": msg }
        }).to_string()
    })
}

// ---------------------------------------------------------------------------
// JSON serialisers (mirror api_json.ml)
// ---------------------------------------------------------------------------

pub fn user_to_json(u: &User) -> Value {
    json!({
        "id":            u.id.to_string(),
        "email":         u.email,
        "name":          u.name,
        "cognito_group": u.cognito_group.to_string(),
        "language":      u.language.to_string(),
        "currency":      u.currency.to_string(),
        "created":       u.created.to_rfc3339(),
    })
}

/// Mirrors OCaml `api_json.ml :: node_ref_to_json`.
pub fn node_ref_to_json(id: &NodeId, name: &str) -> Value {
    json!({
        "id":   id.to_string(),
        "name": name,
    })
}

pub fn node_to_json(n: &Node) -> Value {
    let parent = n
        .parent
        .as_ref()
        .map(|p| Value::String(p.to_string()))
        .unwrap_or(Value::Null);
    json!({
        "id":       n.id.to_string(),
        "name":     n.name,
        "parent":   parent,
        "created":  n.created.to_rfc3339(),
        "metadata": n.metadata,
    })
}

pub fn sensor_to_json(s: &Sensor) -> Value {
    json!({
        "id":               s.id.to_string(),
        "created":          s.created.to_rfc3339(),
        "daq_id":           s.daq_id,
        "path":             s.path,
        "purpose":          s.purpose,
        "meter_type":       s.meter_type.to_string(),
        "unit":             s.unit.as_ref().map(|u| Value::String(u.clone())).unwrap_or(Value::Null),
        "resample_minutes": s.resample_minutes.map(|v| json!(v)).unwrap_or(Value::Null),
        "formula":          formula_to_json(&s.formula),
    })
}

fn formula_to_json(f: &model::domain::formula::Formula) -> Value {
    use model::domain::formula::Formula;
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

fn expr_to_string(e: &model::domain::formula::Expr) -> String {
    use model::domain::formula::Expr;
    match e {
        Expr::Num(n) => format!("{}", n),
        Expr::SelfRef => "self".to_string(),
        Expr::Ref(a) => a.clone(),
        Expr::Abs(inner) => format!("abs({})", expr_to_string(inner)),
        Expr::Add(l, r) => format!("{} + {}", expr_to_string(l), expr_to_string(r)),
        Expr::Sub(l, r) => format!("{} - {}", expr_to_string(l), expr_to_string(r)),
        Expr::Mul(l, r) => format!("{} * {}", expr_to_string(l), expr_to_string(r)),
        Expr::Div(l, r) => format!("{} / {}", expr_to_string(l), expr_to_string(r)),
    }
}

// ---------------------------------------------------------------------------
// Formula parsing
// ---------------------------------------------------------------------------

fn parse_formula(v: Option<Value>) -> Result<model::domain::formula::Formula, String> {
    use model::domain::formula::Formula;

    match v {
        None | Some(Value::Null) => Ok(Formula::Identity),
        Some(Value::String(ref s)) if s == "identity" => Ok(Formula::Identity),
        Some(Value::String(ref s)) if s == "zero" => Ok(Formula::Zero),
        Some(Value::Object(ref map)) => {
            let kind = map.get("kind").and_then(|v| v.as_str());
            match kind {
                Some("identity") | None => Ok(Formula::Identity),
                Some("zero") => Ok(Formula::Zero),
                Some("expr") => parse_expr_formula(map),
                Some(other) => Err(format!("unknown formula kind {:?}", other)),
            }
        }
        Some(_) => Err("formula must be an object or string".to_string()),
    }
}

fn parse_expr_formula(
    map: &serde_json::Map<String, Value>,
) -> Result<model::domain::formula::Formula, String> {
    use model::domain::formula::Formula;
    use model::domain::ids::SensorId;

    let expr_s = map
        .get("expr")
        .and_then(|v| v.as_str())
        .ok_or("formula.expr must be a string")?;

    // Parse refs: { alias: "S#n", ... }
    let refs: Vec<(String, SensorId)> = match map.get("refs") {
        None | Some(Value::Null) => vec![],
        Some(Value::Object(refs_map)) => {
            let mut out = Vec::new();
            for (alias, id_val) in refs_map {
                let id_s = id_val
                    .as_str()
                    .ok_or_else(|| format!("ref {} must be a string", alias))?;
                let sid = SensorId::parse(id_s)
                    .map_err(|e| format!("bad sensor id for alias {}: {}", alias, e))?;
                out.push((alias.clone(), sid));
            }
            out
        }
        _ => return Err("formula.refs must be an object".to_string()),
    };

    let ast = parse_expr_string(expr_s)?;

    // Validate: all aliases in expr must be in refs.
    let expr_aliases = collect_aliases(&ast);
    for alias in &expr_aliases {
        if !refs.iter().any(|(a, _)| a == alias) {
            return Err(format!("formula references unbound alias {:?}", alias));
        }
    }
    // Validate: no unused refs.
    for (alias, _) in &refs {
        if !expr_aliases.contains(alias) {
            return Err(format!("formula.refs has unused alias {:?}", alias));
        }
    }

    Ok(Formula::Expr { refs, expr: ast })
}

/// Collect all alias names (Ref nodes) from an expression.
fn collect_aliases(expr: &model::domain::formula::Expr) -> Vec<String> {
    use model::domain::formula::Expr;
    match expr {
        Expr::Ref(a) => vec![a.clone()],
        Expr::Abs(inner) => collect_aliases(inner),
        Expr::Add(l, r) | Expr::Sub(l, r) | Expr::Mul(l, r) | Expr::Div(l, r) => {
            let mut out = collect_aliases(l);
            out.extend(collect_aliases(r));
            out
        }
        Expr::Num(_) | Expr::SelfRef => vec![],
    }
}

/// Minimal recursive-descent parser for formula expressions.
/// Supports: literals, `self`, identifiers (aliases), `abs(...)`,
/// `+` `-` `*` `/` with standard left-to-right precedence, parentheses.
fn parse_expr_string(s: &str) -> Result<model::domain::formula::Expr, String> {
    let tokens = tokenise(s)?;
    let (expr, pos) = parse_additive(&tokens, 0)?;
    if pos != tokens.len() {
        return Err(format!("unexpected token at position {}", pos));
    }
    Ok(expr)
}

#[derive(Debug, Clone)]
enum Token {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
}

fn tokenise(s: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' | '\n' => {
                i += 1;
            }
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let num_s: String = chars[start..i].iter().collect();
                let n: f64 = num_s
                    .parse()
                    .map_err(|_| format!("bad number {:?}", num_s))?;
                tokens.push(Token::Num(n));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                tokens.push(Token::Ident(ident));
            }
            c => return Err(format!("unexpected character {:?}", c)),
        }
    }
    Ok(tokens)
}

fn parse_additive(
    tokens: &[Token],
    pos: usize,
) -> Result<(model::domain::formula::Expr, usize), String> {
    use model::domain::formula::Expr;
    let (mut lhs, mut pos) = parse_multiplicative(tokens, pos)?;
    loop {
        match tokens.get(pos) {
            Some(Token::Plus) => {
                let (rhs, new_pos) = parse_multiplicative(tokens, pos + 1)?;
                lhs = Expr::Add(Box::new(lhs), Box::new(rhs));
                pos = new_pos;
            }
            Some(Token::Minus) => {
                let (rhs, new_pos) = parse_multiplicative(tokens, pos + 1)?;
                lhs = Expr::Sub(Box::new(lhs), Box::new(rhs));
                pos = new_pos;
            }
            _ => break,
        }
    }
    Ok((lhs, pos))
}

fn parse_multiplicative(
    tokens: &[Token],
    pos: usize,
) -> Result<(model::domain::formula::Expr, usize), String> {
    use model::domain::formula::Expr;
    let (mut lhs, mut pos) = parse_unary(tokens, pos)?;
    loop {
        match tokens.get(pos) {
            Some(Token::Star) => {
                let (rhs, new_pos) = parse_unary(tokens, pos + 1)?;
                lhs = Expr::Mul(Box::new(lhs), Box::new(rhs));
                pos = new_pos;
            }
            Some(Token::Slash) => {
                let (rhs, new_pos) = parse_unary(tokens, pos + 1)?;
                lhs = Expr::Div(Box::new(lhs), Box::new(rhs));
                pos = new_pos;
            }
            _ => break,
        }
    }
    Ok((lhs, pos))
}

fn parse_unary(
    tokens: &[Token],
    pos: usize,
) -> Result<(model::domain::formula::Expr, usize), String> {
    use model::domain::formula::Expr;
    // abs(...)
    if let Some(Token::Ident(name)) = tokens.get(pos) {
        if name == "abs" && matches!(tokens.get(pos + 1), Some(Token::LParen)) {
            let (inner, new_pos) = parse_additive(tokens, pos + 2)?;
            if matches!(tokens.get(new_pos), Some(Token::RParen)) {
                return Ok((Expr::Abs(Box::new(inner)), new_pos + 1));
            }
            return Err("expected ')' after abs(...)".to_string());
        }
    }
    parse_primary(tokens, pos)
}

fn parse_primary(
    tokens: &[Token],
    pos: usize,
) -> Result<(model::domain::formula::Expr, usize), String> {
    use model::domain::formula::Expr;
    match tokens.get(pos) {
        Some(Token::Num(n)) => Ok((Expr::Num(*n), pos + 1)),
        Some(Token::Ident(name)) if name == "self" => Ok((Expr::SelfRef, pos + 1)),
        Some(Token::Ident(name)) => Ok((Expr::Ref(name.clone()), pos + 1)),
        Some(Token::LParen) => {
            let (inner, new_pos) = parse_additive(tokens, pos + 1)?;
            match tokens.get(new_pos) {
                Some(Token::RParen) => Ok((inner, new_pos + 1)),
                _ => Err("expected ')'".to_string()),
            }
        }
        Some(Token::Minus) => {
            let (inner, new_pos) = parse_primary(tokens, pos + 1)?;
            Ok((
                Expr::Sub(Box::new(Expr::Num(0.0)), Box::new(inner)),
                new_pos,
            ))
        }
        t => Err(format!("unexpected token {:?}", t)),
    }
}

// ---------------------------------------------------------------------------
// add_node handler
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn handle_add_node<FGN, FGNFut, FLC, FLCFut, FAN, FANFut>(
    parent_id: String,
    name: String,
    level: Option<String>,
    label: Option<String>,
    metadata: Option<Value>,
    schema_val: Option<Value>,
    get_node: FGN,
    list_children: FLC,
    add_node_fn: FAN,
) -> Value
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FLC: FnOnce(NodeId, Option<EdgeKind>) -> FLCFut,
    FLCFut: Future<Output = Result<Vec<Node>, RepositoryError>>,
    FAN: FnOnce(model::domain::ids::Level, Box<dyn FnOnce(u32) -> (Node, EdgeSpec) + Send>) -> FANFut,
    FANFut: Future<Output = Result<Node, RepositoryError>>,
{
    let parent = match NodeId::parse(&parent_id) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad parent_id: {}", e)),
    };

    let level_parsed = match level {
        None => None,
        Some(ref s) => match model::domain::ids::Level::parse(s) {
            Ok(l) => Some(l),
            Err(e) => return bad_request(&format!("bad level: {}", e)),
        },
    };

    let schema_parsed = match schema_val {
        None => None,
        Some(ref v) => {
            match serde_json::from_value::<model::domain::schema::Schema>(v.clone()) {
                Ok(s) => Some(s),
                Err(e) => return bad_request(&format!("invalid schema: {}", e)),
            }
        }
    };

    let meta = metadata.unwrap_or_else(|| json!({}));

    match hierarchy::add_node(
        parent,
        level_parsed,
        label,
        name,
        meta,
        schema_parsed,
        get_node,
        list_children,
        add_node_fn,
    )
    .await
    {
        Ok(n) => ok(node_to_json(&n)),
        Err(e) => repo_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// delete_node handler
// ---------------------------------------------------------------------------

pub async fn handle_delete_node<FGN, FGNFut, FDN, FDNFut>(
    id: String,
    get_node: FGN,
    delete_node_fn: FDN,
) -> Value
where
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FDN: FnOnce(NodeId) -> FDNFut,
    FDNFut: Future<Output = Result<(), RepositoryError>>,
{
    let node_id = match NodeId::parse(&id) {
        Ok(i) => i,
        Err(e) => return bad_request(&format!("bad id: {}", e)),
    };

    // Verify the node exists (mirrors OCaml behaviour).
    match get_node(node_id.clone()).await {
        Ok(None) => return repo_error_response(RepositoryError::NotFound(node_id)),
        Err(e) => return repo_error_response(e),
        Ok(Some(_)) => {}
    }

    match delete_node_fn(node_id.clone()).await {
        Ok(()) => ok(json!({ "deleted": id })),
        Err(e) => repo_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// create_user handler
// ---------------------------------------------------------------------------

/// Handler for `create_user`.
///
/// Mirrors `run_create_user` in `api_command.ml`:
/// 1. Parse profile → cognito_group.
/// 2. `users::create` (DDB put).
/// 3. Best-effort `grant_administrates` for each allowed node (ignore errors).
/// 4. Best-effort `block` for each blocked node (ignore errors).
/// 5. `create_cognito(email, name)` + `add_to_group(email, group)`.
/// 6. On Cognito failure: `users::delete` rolls back the DDB user; return error.
///
/// `get_node` and `put_edge` are `Fn` (called multiple times in the loop).
#[allow(clippy::too_many_arguments)]
pub async fn handle_create_user<
    FGU,
    FGUFut,
    FPU,
    FPUFut,
    FGN,
    FGNFut,
    FPE,
    FPEFut,
    FLA,
    FLAFut,
    FLB,
    FLBFut,
    FDE,
    FDEFut,
    FDU,
    FDUFut,
    FCC,
    FCCFut,
    FAG,
    FAGFut,
>(
    email: String,
    name: String,
    profile_s: String,
    language_s: Option<String>,
    currency_s: Option<String>,
    allowed: Vec<String>,
    blocked: Vec<String>,
    get_user: FGU,
    put_user: FPU,
    get_node: FGN,
    put_edge: FPE,
    list_administrated: FLA,
    list_blocked: FLB,
    delete_edge: FDE,
    delete_user: FDU,
    create_cognito: FCC,
    add_to_group: FAG,
) -> Value
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FPU: FnOnce(User) -> FPUFut,
    FPUFut: Future<Output = Result<(), RepositoryError>>,
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FPE: Fn(EdgeSpec) -> FPEFut,
    FPEFut: Future<Output = Result<(), RepositoryError>>,
    FLA: FnOnce(UserId) -> FLAFut,
    FLAFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
    FLB: FnOnce(UserId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
    FDE: Fn(String, String, EdgeKind) -> FDEFut,
    FDEFut: Future<Output = Result<(), RepositoryError>>,
    FDU: FnOnce(UserId) -> FDUFut,
    FDUFut: Future<Output = Result<(), RepositoryError>>,
    FCC: FnOnce(String, String) -> FCCFut,
    FCCFut: Future<Output = Result<(), RepositoryError>>,
    FAG: FnOnce(String, CognitoGroup) -> FAGFut,
    FAGFut: Future<Output = Result<(), RepositoryError>>,
{
    // 1. Parse profile → cognito_group.
    let profile = match Profile::parse(&profile_s) {
        Ok(p) => p,
        Err(e) => return bad_request(&format!("bad profile: {}", e)),
    };
    let cognito_group = profile.to_cognito_group();

    // Parse optional language / currency.
    let language = match language_s {
        None => None,
        Some(ref s) => match Language::parse(s) {
            Ok(l) => Some(l),
            Err(e) => return bad_request(&format!("bad language: {}", e)),
        },
    };
    let currency = match currency_s {
        None => None,
        Some(ref s) => match Currency::parse(s) {
            Ok(c) => Some(c),
            Err(e) => return bad_request(&format!("bad currency: {}", e)),
        },
    };

    // 2. Create user in DDB.
    let user = match users::create(
        email.clone(),
        name,
        cognito_group,
        language,
        currency,
        get_user,
        put_user,
    )
    .await
    {
        Ok(u) => u,
        Err(e) => return repo_error_response(e),
    };

    let user_id = UserId::of_email(&email);

    // 3 & 4. Best-effort access grants (mirrors OCaml `ignore (f ...)` pattern).
    // access::grant_administrates / block each take FnOnce closures; we drive
    // them manually so that `get_node` and `put_edge` (Fn) can be called once
    // per iteration.
    for node_s in &allowed {
        if let Ok(node_id) = NodeId::parse(node_s) {
            let uid = user_id.clone();
            let u_clone = user.clone();
            let _ = access::grant_administrates(
                uid,
                node_id,
                move |_id| {
                    let u = u_clone;
                    async move { Ok(Some(u)) }
                },
                &get_node,
                &put_edge,
            )
            .await;
        }
    }
    for node_s in &blocked {
        if let Ok(node_id) = NodeId::parse(node_s) {
            let uid = user_id.clone();
            let u_clone = user.clone();
            let _ = access::block(
                uid,
                node_id,
                move |_id| {
                    let u = u_clone;
                    async move { Ok(Some(u)) }
                },
                &get_node,
                &put_edge,
            )
            .await;
        }
    }

    // 5. Cognito create + add to group.
    if let Err(e) = create_cognito(email.clone(), user.name.clone()).await {
        // 6. Rollback: delete the DDB user (cascade edges).
        let u_rb = user.clone();
        let _ = users::delete(
            user_id,
            move |_id| {
                let u = u_rb;
                async move { Ok(Some(u)) }
            },
            list_administrated,
            list_blocked,
            delete_edge,
            delete_user,
        )
        .await;
        return repo_error_response(e);
    }

    if let Err(e) = add_to_group(email, cognito_group).await {
        return repo_error_response(e);
    }

    ok(user_to_json(&user))
}

// ---------------------------------------------------------------------------
// update_user handler
// ---------------------------------------------------------------------------

pub async fn handle_update_user<FGU, FGUFut, FPU, FPUFut>(
    id: String,
    name: Option<String>,
    cognito_group_s: Option<String>,
    language_s: Option<String>,
    currency_s: Option<String>,
    get_user: FGU,
    put_user: FPU,
) -> Value
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FPU: FnOnce(User) -> FPUFut,
    FPUFut: Future<Output = Result<(), RepositoryError>>,
{
    let user_id = match UserId::parse(&id) {
        Ok(i) => i,
        Err(e) => return bad_request(&format!("bad id: {}", e)),
    };
    let cognito_group = match cognito_group_s {
        None => None,
        Some(ref s) => match CognitoGroup::parse(s) {
            Ok(g) => Some(g),
            Err(e) => return bad_request(&format!("bad cognito_group: {}", e)),
        },
    };
    let language = match language_s {
        None => None,
        Some(ref s) => match Language::parse(s) {
            Ok(l) => Some(l),
            Err(e) => return bad_request(&format!("bad language: {}", e)),
        },
    };
    let currency = match currency_s {
        None => None,
        Some(ref s) => match Currency::parse(s) {
            Ok(c) => Some(c),
            Err(e) => return bad_request(&format!("bad currency: {}", e)),
        },
    };

    match users::update(
        user_id,
        name,
        cognito_group,
        language,
        currency,
        get_user,
        put_user,
    )
    .await
    {
        Ok(u) => ok(user_to_json(&u)),
        Err(e) => repo_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// delete_user handler
// ---------------------------------------------------------------------------

/// Handler for `delete_user`.
///
/// Mirrors `run_delete_user`: resolves user id (email preferred, U# fallback),
/// cascades edges via `users::delete`, then `delete_cognito(email)`.
pub async fn handle_delete_user<
    FGU,
    FGUFut,
    FLA,
    FLAFut,
    FLB,
    FLBFut,
    FDE,
    FDEFut,
    FDU,
    FDUFut,
    FDC,
    FDCFut,
>(
    email_or_id: &str,
    get_user: FGU,
    list_administrated: FLA,
    list_blocked: FLB,
    delete_edge: FDE,
    delete_user: FDU,
    delete_cognito: FDC,
) -> Value
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FLA: FnOnce(UserId) -> FLAFut,
    FLAFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
    FLB: FnOnce(UserId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
    FDE: Fn(String, String, EdgeKind) -> FDEFut,
    FDEFut: Future<Output = Result<(), RepositoryError>>,
    FDU: FnOnce(UserId) -> FDUFut,
    FDUFut: Future<Output = Result<(), RepositoryError>>,
    FDC: FnOnce(String) -> FDCFut,
    FDCFut: Future<Output = Result<(), RepositoryError>>,
{
    let user_id = if email_or_id.starts_with("U#") {
        match UserId::parse(email_or_id) {
            Ok(id) => id,
            Err(e) => return bad_request(&format!("bad user id: {}", e)),
        }
    } else {
        UserId::of_email(email_or_id)
    };

    let deleted_id = match users::delete(
        user_id,
        get_user,
        list_administrated,
        list_blocked,
        delete_edge,
        delete_user,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => return repo_error_response(e),
    };

    let email = deleted_id.email().to_string();
    let _ = delete_cognito(email).await;

    ok(json!({ "deleted": deleted_id.to_string() }))
}

// ---------------------------------------------------------------------------
// block_user handler
// ---------------------------------------------------------------------------

pub async fn handle_block_user<FGU, FGUFut, FGN, FGNFut, FPE, FPEFut>(
    user_id_s: String,
    node_id_s: String,
    get_user: FGU,
    get_node: FGN,
    put_edge: FPE,
) -> Value
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FPE: FnOnce(EdgeSpec) -> FPEFut,
    FPEFut: Future<Output = Result<(), RepositoryError>>,
{
    let user_id = match UserId::parse(&user_id_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad user_id: {}", e)),
    };
    let node_id = match NodeId::parse(&node_id_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad node_id: {}", e)),
    };

    match access::block(user_id, node_id, get_user, get_node, put_edge).await {
        Ok(()) => ok(json!({ "ok": true })),
        Err(e) => repo_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// unblock_user handler
// ---------------------------------------------------------------------------

pub async fn handle_unblock_user<FDE, FDEFut>(
    user_id_s: String,
    node_id_s: String,
    delete_edge: FDE,
) -> Value
where
    FDE: FnOnce(String, String, EdgeKind) -> FDEFut,
    FDEFut: Future<Output = Result<(), RepositoryError>>,
{
    let user_id = match UserId::parse(&user_id_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad user_id: {}", e)),
    };
    let node_id = match NodeId::parse(&node_id_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad node_id: {}", e)),
    };

    match access::unblock(user_id, node_id, delete_edge).await {
        Ok(()) => ok(json!({ "ok": true })),
        Err(e) => repo_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// grant_administrates handler
// ---------------------------------------------------------------------------

pub async fn handle_grant_administrates<FGU, FGUFut, FGN, FGNFut, FPE, FPEFut>(
    user_id_s: String,
    node_id_s: String,
    get_user: FGU,
    get_node: FGN,
    put_edge: FPE,
) -> Value
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FPE: FnOnce(EdgeSpec) -> FPEFut,
    FPEFut: Future<Output = Result<(), RepositoryError>>,
{
    let user_id = match UserId::parse(&user_id_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad user_id: {}", e)),
    };
    let node_id = match NodeId::parse(&node_id_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad node_id: {}", e)),
    };

    match access::grant_administrates(user_id, node_id, get_user, get_node, put_edge).await {
        Ok(()) => ok(json!({ "ok": true })),
        Err(e) => repo_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// attach_sensor handler
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn handle_attach_sensor<FGN, FGNFut, FAS, FASFut, FGA, FDS, FDSFut>(
    parent_id: String,
    daq_id: String,
    purpose: String,
    meter_type_s: String,
    unit: Option<String>,
    resample_val: Option<Value>,
    formula_val: Option<Value>,
    get_node: FGN,
    add_sensor: FAS,
    get_active_sensor: FGA,
    delete_sensor: FDS,
) -> Value
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FAS: FnOnce(Box<dyn FnOnce(u32) -> (Sensor, EdgeSpec) + Send>) -> FASFut,
    FASFut: Future<Output = Result<Sensor, RepositoryError>>,
    FGA: Fn(SensorId) -> Option<Sensor> + Clone + 'static,
    FDS: FnOnce(SensorId, NodeId) -> FDSFut,
    FDSFut: Future<Output = Result<(), RepositoryError>>,
{
    let parent = match NodeId::parse(&parent_id) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad parent_id: {}", e)),
    };
    let meter_type = match MeterType::parse(&meter_type_s) {
        Ok(mt) => mt,
        Err(e) => return bad_request(&format!("bad meter_type: {}", e)),
    };

    // Coerce resample_minutes: int or numeric string; empty/null → None.
    let resample_minutes: Option<i32> = match resample_val {
        Some(Value::Number(ref n)) => n.as_i64().map(|i| i as i32),
        Some(Value::String(ref s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<i32>().ok()
            }
        }
        _ => None,
    };

    let formula = match parse_formula(formula_val) {
        Ok(f) => f,
        Err(e) => return bad_request(&e),
    };

    match sensors::attach(
        parent,
        daq_id,
        purpose,
        meter_type,
        unit,
        formula,
        resample_minutes,
        get_node,
        add_sensor,
        get_active_sensor,
        delete_sensor,
    )
    .await
    {
        Ok(s) => ok(sensor_to_json(&s)),
        Err(e) => repo_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// replace_sensor_device handler
// ---------------------------------------------------------------------------

pub async fn handle_replace_sensor_device<FGA, FRD, FRDFut>(
    sensor_id_s: String,
    new_daq_id: String,
    get_active_sensor: FGA,
    replace_sensor_device: FRD,
) -> Value
where
    FGA: FnOnce(SensorId) -> Option<Sensor>,
    FRD: FnOnce(chrono::DateTime<chrono::Utc>, Sensor) -> FRDFut,
    FRDFut: Future<Output = Result<(), RepositoryError>>,
{
    let sid = match SensorId::parse(&sensor_id_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad sensor_id: {}", e)),
    };

    match sensors::replace_device(sid, new_daq_id, get_active_sensor, replace_sensor_device).await
    {
        Ok(s) => ok(sensor_to_json(&s)),
        Err(e) => repo_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// Top-level dispatch — prod entry point
// ---------------------------------------------------------------------------

/// Dispatch a `Command` to the real AWS-backed repository.
///
/// Obtains shared clients (OnceCell) and builds the real repository closures.
pub async fn run(cmd: Command) -> Value {
    use model::repository::dynamodb::{edge, node as ddb_node, sensor as ddb_sensor, user};

    let ddb = model::get_dynamodb_client().await;
    let cog = model::get_cognito_client().await;
    let table = model::get_table_name();
    let pool_id = model::get_user_pool_id();

    match cmd {
        Command::AddNode {
            parent_id,
            name,
            level,
            label,
            metadata,
            schema,
        } => {
            handle_add_node(
                parent_id,
                name,
                level,
                label,
                metadata,
                schema,
                {
                    let t = table.clone();
                    move |id| {
                        let t = t.clone();
                        async move { ddb_node::get_node(ddb, &t, &id).await }
                    }
                },
                {
                    let t = table.clone();
                    move |parent, kind: Option<EdgeKind>| {
                        let t = t.clone();
                        async move {
                            ddb_node::list_children(ddb, &t, &parent, kind.as_ref()).await
                        }
                    }
                },
                {
                    let t = table.clone();
                    move |level, build: Box<dyn FnOnce(u32) -> (Node, EdgeSpec) + Send>| {
                        let t = t.clone();
                        // Wrap the FnOnce in Arc<Mutex<Option>> so it satisfies Fn.
                        let build_cell = std::sync::Arc::new(std::sync::Mutex::new(Some(build)));
                        async move {
                            ddb_node::allocate_and_put_node(ddb, &t, level, move |id| {
                                let build = build_cell
                                    .lock()
                                    .unwrap()
                                    .take()
                                    .expect("build called more than once");
                                let (node, repo_edge) = build(id);
                                let alloc_edge = ddb_node::AllocEdgeSpec {
                                    from_: repo_edge.from_,
                                    to_: repo_edge.to_,
                                    kind: repo_edge.kind,
                                    name: repo_edge.name,
                                    created: node.created,
                                    self_path: Some(node.path.clone()),
                                };
                                (node, alloc_edge)
                            })
                            .await
                        }
                    }
                },
            )
            .await
        }

        Command::DeleteNode { id } => {
            handle_delete_node(
                id,
                {
                    let t = table.clone();
                    move |nid| {
                        let t = t.clone();
                        async move { ddb_node::get_node(ddb, &t, &nid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |nid| {
                        let t = t.clone();
                        async move { ddb_node::delete_subtree(ddb, &t, &nid).await }
                    }
                },
            )
            .await
        }

        Command::AttachSensor {
            parent_id,
            daq_id,
            purpose,
            meter_type,
            unit,
            resample_minutes,
            formula,
        } => {
            handle_attach_sensor(
                parent_id,
                daq_id,
                purpose,
                meter_type,
                unit,
                resample_minutes,
                formula,
                {
                    let t = table.clone();
                    move |id| {
                        let t = t.clone();
                        async move { ddb_node::get_node(ddb, &t, &id).await }
                    }
                },
                {
                    let t = table.clone();
                    move |build: Box<dyn FnOnce(u32) -> (Sensor, EdgeSpec) + Send>| {
                        let t = t.clone();
                        let build_cell =
                            std::sync::Arc::new(std::sync::Mutex::new(Some(build)));
                        async move {
                            use model::repository::dynamodb::codec;
                            let item =
                                ddb_node::allocate_and_put_sensor(ddb, &t, move |id| {
                                    let build = build_cell
                                        .lock()
                                        .unwrap()
                                        .take()
                                        .expect("build called more than once");
                                    let (sensor, repo_edge) = build(id);
                                    let sensor_item = codec::sensor_to_item(&sensor);
                                    let alloc_edge = ddb_node::AllocEdgeSpec {
                                        from_: repo_edge.from_,
                                        to_: repo_edge.to_,
                                        kind: repo_edge.kind,
                                        name: repo_edge.name,
                                        created: sensor.created,
                                        self_path: Some(sensor.path.clone()),
                                    };
                                    (sensor_item, alloc_edge)
                                })
                                .await?;
                            codec::sensor_of_item(&item)
                                .map_err(|e| RepositoryError::Codec(e.to_string()))
                        }
                    }
                },
                // Synchronous get_active_sensor: not available in the prod path.
                |_sid| None,
                {
                    let t = table.clone();
                    move |sid, parent| {
                        let t = t.clone();
                        async move { ddb_sensor::delete_sensor(ddb, &t, &sid, &parent).await }
                    }
                },
            )
            .await
        }

        Command::ReplaceSensorDevice { sensor_id, daq_id } => {
            handle_replace_sensor_device(
                sensor_id,
                daq_id,
                |_sid| None,
                {
                    let t = table.clone();
                    move |old_created, new_sensor| {
                        let t = t.clone();
                        async move {
                            ddb_sensor::transact_replace(ddb, &t, old_created, &new_sensor).await
                        }
                    }
                },
            )
            .await
        }

        Command::CreateUser {
            email,
            name,
            profile,
            language,
            currency,
            allowed,
            blocked,
        } => {
            let pool1 = pool_id.clone();
            let pool2 = pool_id.clone();
            handle_create_user(
                email,
                name,
                profile,
                language,
                currency,
                allowed,
                blocked,
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::get_user(ddb, &t, &uid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |u| {
                        let t = t.clone();
                        async move { user::put_user(ddb, &t, &u).await }
                    }
                },
                {
                    let t = table.clone();
                    move |id| {
                        let t = t.clone();
                        async move { ddb_node::get_node(ddb, &t, &id).await }
                    }
                },
                {
                    let t = table.clone();
                    move |spec: EdgeSpec| {
                        let t = t.clone();
                        async move {
                            edge::put_edge(
                                ddb,
                                &t,
                                &spec.from_,
                                &spec.to_,
                                &spec.kind,
                                &spec.name,
                                &chrono::Utc::now(),
                                None,
                            )
                            .await
                        }
                    }
                },
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::list_administrated_nodes(ddb, &t, &uid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::list_blocked_nodes(ddb, &t, &uid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |from_, to_, kind| {
                        let t = t.clone();
                        async move { edge::delete_edge(ddb, &t, &from_, &to_, &kind).await }
                    }
                },
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::delete_user(ddb, &t, &uid).await }
                    }
                },
                move |email, name| {
                    let pool1 = pool1.clone();
                    async move {
                        model::repository::cognito::user::create_cognito_user(
                            cog, &pool1, &email, &name,
                        )
                        .await
                    }
                },
                move |email, group| {
                    let pool2 = pool2.clone();
                    async move {
                        model::repository::cognito::user::add_user_to_group(
                            cog, &pool2, &email, group,
                        )
                        .await
                    }
                },
            )
            .await
        }

        Command::UpdateUser {
            id,
            name,
            cognito_group,
            language,
            currency,
        } => {
            handle_update_user(
                id,
                name,
                cognito_group,
                language,
                currency,
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::get_user(ddb, &t, &uid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |u| {
                        let t = t.clone();
                        async move { user::put_user(ddb, &t, &u).await }
                    }
                },
            )
            .await
        }

        Command::DeleteUser { email, id } => {
            let email_or_id = email.or(id).unwrap_or_default();
            let pool = pool_id.clone();
            handle_delete_user(
                &email_or_id,
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::get_user(ddb, &t, &uid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::list_administrated_nodes(ddb, &t, &uid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::list_blocked_nodes(ddb, &t, &uid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |from_, to_, kind| {
                        let t = t.clone();
                        async move { edge::delete_edge(ddb, &t, &from_, &to_, &kind).await }
                    }
                },
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::delete_user(ddb, &t, &uid).await }
                    }
                },
                move |email| {
                    let pool = pool.clone();
                    async move {
                        model::repository::cognito::user::delete_cognito_user(cog, &pool, &email)
                            .await
                    }
                },
            )
            .await
        }

        Command::BlockUser { user_id, node_id } => {
            handle_block_user(
                user_id,
                node_id,
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::get_user(ddb, &t, &uid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |nid| {
                        let t = t.clone();
                        async move { ddb_node::get_node(ddb, &t, &nid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |spec: EdgeSpec| {
                        let t = t.clone();
                        async move {
                            edge::put_edge(
                                ddb,
                                &t,
                                &spec.from_,
                                &spec.to_,
                                &spec.kind,
                                &spec.name,
                                &chrono::Utc::now(),
                                None,
                            )
                            .await
                        }
                    }
                },
            )
            .await
        }

        Command::UnblockUser { user_id, node_id } => {
            handle_unblock_user(
                user_id,
                node_id,
                {
                    let t = table.clone();
                    move |from_, to_, kind| {
                        let t = t.clone();
                        async move { edge::delete_edge(ddb, &t, &from_, &to_, &kind).await }
                    }
                },
            )
            .await
        }

        Command::GrantAdministrates { user_id, node_id } => {
            handle_grant_administrates(
                user_id,
                node_id,
                {
                    let t = table.clone();
                    move |uid| {
                        let t = t.clone();
                        async move { user::get_user(ddb, &t, &uid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |nid| {
                        let t = t.clone();
                        async move { ddb_node::get_node(ddb, &t, &nid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |spec: EdgeSpec| {
                        let t = t.clone();
                        async move {
                            edge::put_edge(
                                ddb,
                                &t,
                                &spec.from_,
                                &spec.to_,
                                &spec.kind,
                                &spec.name,
                                &chrono::Utc::now(),
                                None,
                            )
                            .await
                        }
                    }
                },
            )
            .await
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — port of `services/hierarchy/test/test_api_command.ml`
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use serde_json::json;

    use model::domain::ids::{Level, NodeId, SensorId, UserId};
    use model::domain::node;
    use model::domain::schema::{EdgeSpec as SchemaEdgeSpec, Schema};
    use model::domain::values::{CognitoGroup, EdgeKind};
    use model::errors::RepositoryError;
    use model::repository::memory::Store;
    use model::repository::EdgeSpec as RepoEdgeSpec;

    use super::*;

    // -----------------------------------------------------------------------
    // Closure factories over Store
    // -----------------------------------------------------------------------

    fn make_get_user(
        s: Rc<Store>,
    ) -> impl Fn(UserId)
           -> std::future::Ready<Result<Option<model::domain::user::User>, RepositoryError>>
           + Clone {
        move |uid| std::future::ready(Ok(s.get_user(&uid)))
    }

    fn make_put_user(
        s: Rc<Store>,
    ) -> impl FnOnce(
        model::domain::user::User,
    ) -> std::future::Ready<Result<(), RepositoryError>> {
        move |u| {
            s.put_user(&u);
            std::future::ready(Ok(()))
        }
    }

    fn make_get_node(
        s: Rc<Store>,
    ) -> impl Fn(NodeId)
           -> std::future::Ready<Result<Option<node::Node>, RepositoryError>>
           + Clone {
        move |nid| std::future::ready(Ok(s.get_node(&nid)))
    }

    fn make_put_edge(
        s: Rc<Store>,
    ) -> impl Fn(RepoEdgeSpec) -> std::future::Ready<Result<(), RepositoryError>> {
        move |spec| {
            s.put_edge(spec);
            std::future::ready(Ok(()))
        }
    }

    fn make_delete_edge(
        s: Rc<Store>,
    ) -> impl Fn(String, String, EdgeKind)
           -> std::future::Ready<Result<(), RepositoryError>> {
        move |from_, to_, kind| {
            s.delete_edge(&from_, &to_, &kind);
            std::future::ready(Ok(()))
        }
    }

    fn make_list_administrated(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<Vec<NodeId>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_administrated_nodes(&uid)))
    }

    fn make_list_blocked(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<Vec<NodeId>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_blocked_nodes(&uid)))
    }

    fn make_delete_user(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<(), RepositoryError>> {
        move |uid| {
            s.delete_user(&uid);
            std::future::ready(Ok(()))
        }
    }

    fn make_add_sensor(
        s: Rc<Store>,
    ) -> impl FnOnce(
        Box<dyn FnOnce(u32) -> (model::domain::sensor::Sensor, RepoEdgeSpec) + Send>,
    ) -> std::future::Ready<Result<model::domain::sensor::Sensor, RepositoryError>> {
        move |build| {
            let sensor = s.add_sensor(build);
            std::future::ready(Ok(sensor))
        }
    }

    fn make_get_active(
        s: Rc<Store>,
    ) -> impl Fn(SensorId) -> Option<model::domain::sensor::Sensor> + Clone {
        move |sid| s.get_active_sensor(&sid)
    }

    fn make_delete_sensor(
        s: Rc<Store>,
    ) -> impl FnOnce(SensorId, NodeId) -> std::future::Ready<Result<(), RepositoryError>> {
        move |sid, parent| {
            s.delete_sensor(&sid, &parent);
            std::future::ready(Ok(()))
        }
    }

    fn make_add_node(
        s: Rc<Store>,
    ) -> impl FnOnce(
        Level,
        Box<dyn FnOnce(u32) -> (node::Node, RepoEdgeSpec) + Send>,
    ) -> std::future::Ready<Result<node::Node, RepositoryError>> {
        move |level, build| {
            let n = s.add_node(level, build);
            std::future::ready(Ok(n))
        }
    }

    fn make_list_children(
        s: Rc<Store>,
    ) -> impl FnOnce(
        NodeId,
        Option<EdgeKind>,
    ) -> std::future::Ready<Result<Vec<node::Node>, RepositoryError>> {
        move |nid, kind| std::future::ready(Ok(s.list_children(&nid, kind.as_ref())))
    }

    /// Cognito stubs — all succeed.
    fn cog_ok_create(
    ) -> impl FnOnce(String, String) -> std::future::Ready<Result<(), RepositoryError>> {
        |_, _| std::future::ready(Ok(()))
    }

    fn cog_ok_add(
    ) -> impl FnOnce(String, CognitoGroup) -> std::future::Ready<Result<(), RepositoryError>> {
        |_, _| std::future::ready(Ok(()))
    }

    fn cog_ok_delete(
    ) -> impl FnOnce(String) -> std::future::Ready<Result<(), RepositoryError>> {
        |_| std::future::ready(Ok(()))
    }

    // -----------------------------------------------------------------------
    // Schema / node helpers
    // -----------------------------------------------------------------------

    fn ts() -> chrono::DateTime<chrono::Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn company_schema() -> Schema {
        Schema {
            version: 1,
            edges: vec![(
                Level::Hn2,
                vec![(
                    Level::Hn3,
                    vec![SchemaEdgeSpec::builder()
                        .label("property".to_string())
                        .build()],
                )],
            )],
            metadata: vec![],
            sensors: vec![],
        }
    }

    fn building_schema() -> Schema {
        Schema {
            version: 1,
            edges: vec![(
                Level::Hn2,
                vec![(
                    Level::Hn3,
                    vec![SchemaEdgeSpec::builder()
                        .label("building".to_string())
                        .build()],
                )],
            )],
            metadata: vec![],
            sensors: vec![Level::Hn3],
        }
    }

    fn seed_company(store: &Rc<Store>) -> NodeId {
        let c2 = NodeId::make(Level::Hn2, 10002);
        let n2 = node::make(
            10002,
            Level::Hn2,
            "Acme",
            NodeId::root(),
            &format!("{}|HN1#10001", NodeId::root()),
            ts(),
            json!({}),
            Some(company_schema()),
        );
        store.put_node(&n2);
        c2
    }

    fn seed_building_company(store: &Rc<Store>) -> NodeId {
        let c2 = NodeId::make(Level::Hn2, 10002);
        let n2 = node::make(
            10002,
            Level::Hn2,
            "Co",
            NodeId::root(),
            &format!("{}|HN1#10001", NodeId::root()),
            ts(),
            json!({}),
            Some(building_schema()),
        );
        store.put_node(&n2);
        c2
    }

    async fn seed_building(store: &Rc<Store>, c2: NodeId) -> NodeId {
        model::logic::hierarchy::add_node(
            c2,
            Some(Level::Hn3),
            None,
            "B".to_string(),
            json!({}),
            None,
            make_get_node(store.clone()),
            make_list_children(store.clone()),
            make_add_node(store.clone()),
        )
        .await
        .expect("seed building")
        .id
    }

    fn status(v: &Value) -> u64 {
        v["statusCode"].as_u64().unwrap_or(0)
    }

    fn body(v: &Value) -> Value {
        serde_json::from_str(v["body"].as_str().unwrap_or("{}")).unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Test: add_node happy path  (OCaml: `add_node_happy_path`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn add_node_happy_path() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let resp = handle_add_node(
            c2.to_string(),
            "P".to_string(),
            Some("hn3".to_string()),
            None,
            Some(json!({})),
            None,
            make_get_node(store.clone()),
            make_list_children(store.clone()),
            make_add_node(store.clone()),
        )
        .await;

        assert_eq!(status(&resp), 200, "add_node should return 200; got {resp:?}");
    }

    // -----------------------------------------------------------------------
    // Test: delete_node roundtrip  (OCaml: `delete_node_roundtrip`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_node_roundtrip() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let add_resp = handle_add_node(
            c2.to_string(),
            "P".to_string(),
            Some("hn3".to_string()),
            None,
            Some(json!({})),
            None,
            make_get_node(store.clone()),
            make_list_children(store.clone()),
            make_add_node(store.clone()),
        )
        .await;
        assert_eq!(status(&add_resp), 200);
        let id_s = body(&add_resp)["id"]
            .as_str()
            .expect("id field")
            .to_string();

        let del_resp = handle_delete_node(
            id_s.clone(),
            make_get_node(store.clone()),
            {
                let s = store.clone();
                move |nid| {
                    s.delete_node(&nid);
                    std::future::ready(Ok(()))
                }
            },
        )
        .await;

        assert_eq!(
            status(&del_resp),
            200,
            "delete_node should return 200; got {del_resp:?}"
        );
        assert!(store.get_node(&NodeId::parse(&id_s).unwrap()).is_none());
    }

    // -----------------------------------------------------------------------
    // Test: create_user happy  (OCaml: `create_user_happy`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_user_happy() {
        let store = Rc::new(Store::new());

        let resp = handle_create_user(
            "alice@ex".to_string(),
            "Alice".to_string(),
            "Developer".to_string(),
            None,
            None,
            vec![],
            vec![],
            make_get_user(store.clone()),
            make_put_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            cog_ok_create(),
            cog_ok_add(),
        )
        .await;

        assert_eq!(status(&resp), 200, "create_user should return 200; got {resp:?}");
        assert_eq!(body(&resp)["email"].as_str().unwrap_or(""), "alice@ex");
    }

    // -----------------------------------------------------------------------
    // Test: create_user SysAdm → Admin group + allowed edges stored
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_user_sysadm_and_allowed_edges() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let resp = handle_create_user(
            "boss@ex".to_string(),
            "Boss".to_string(),
            "SysAdm".to_string(),
            None,
            None,
            vec![c2.to_string()],
            vec![],
            make_get_user(store.clone()),
            make_put_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            cog_ok_create(),
            cog_ok_add(),
        )
        .await;

        assert_eq!(status(&resp), 200, "sysadm should return 200; got {resp:?}");
        assert_eq!(body(&resp)["cognito_group"].as_str().unwrap_or(""), "Admin");

        let uid = UserId::of_email("boss@ex");
        assert!(
            store.list_administrated_nodes(&uid).contains(&c2),
            "Administrates edge must be stored for allowed node"
        );
    }

    // -----------------------------------------------------------------------
    // Test: create_user bad profile → 400
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_user_bad_profile_returns_400() {
        let store = Rc::new(Store::new());

        let resp = handle_create_user(
            "x@ex".to_string(),
            "X".to_string(),
            "BadProfile".to_string(),
            None,
            None,
            vec![],
            vec![],
            make_get_user(store.clone()),
            make_put_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            cog_ok_create(),
            cog_ok_add(),
        )
        .await;

        assert_eq!(status(&resp), 400, "bad profile should be 400; got {resp:?}");
    }

    // -----------------------------------------------------------------------
    // Test: delete_user roundtrip  (OCaml: `delete_user_roundtrip`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_user_roundtrip() {
        let store = Rc::new(Store::new());

        handle_create_user(
            "bob@ex".to_string(),
            "Bob".to_string(),
            "Reader".to_string(),
            None,
            None,
            vec![],
            vec![],
            make_get_user(store.clone()),
            make_put_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            cog_ok_create(),
            cog_ok_add(),
        )
        .await;

        let del = handle_delete_user(
            "U#bob@ex",
            make_get_user(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            cog_ok_delete(),
        )
        .await;

        assert_eq!(status(&del), 200, "delete_user should return 200; got {del:?}");
        assert!(store.get_user(&UserId::of_email("bob@ex")).is_none());
    }

    // -----------------------------------------------------------------------
    // Test: delete_user by email
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_user_by_email() {
        let store = Rc::new(Store::new());

        handle_create_user(
            "carol@ex".to_string(),
            "Carol".to_string(),
            "Reader".to_string(),
            None,
            None,
            vec![],
            vec![],
            make_get_user(store.clone()),
            make_put_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            cog_ok_create(),
            cog_ok_add(),
        )
        .await;

        let del = handle_delete_user(
            "carol@ex",
            make_get_user(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            cog_ok_delete(),
        )
        .await;

        assert_eq!(status(&del), 200, "delete by email; got {del:?}");
        assert!(store.get_user(&UserId::of_email("carol@ex")).is_none());
    }

    // -----------------------------------------------------------------------
    // Test: create_user Cognito failure → DDB rollback
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_user_cognito_failure_rolls_back() {
        let store = Rc::new(Store::new());

        let resp = handle_create_user(
            "fail@ex".to_string(),
            "Fail".to_string(),
            "Reader".to_string(),
            None,
            None,
            vec![],
            vec![],
            make_get_user(store.clone()),
            make_put_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            |_, _| std::future::ready(Err(RepositoryError::Aws("cognito down".to_string()))),
            cog_ok_add(),
        )
        .await;

        assert_ne!(status(&resp), 200, "should fail on Cognito error; got {resp:?}");
        assert!(
            store.get_user(&UserId::of_email("fail@ex")).is_none(),
            "DDB user must be rolled back after Cognito failure"
        );
    }

    // -----------------------------------------------------------------------
    // Test: block_user happy  (OCaml: `block_user_happy`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn block_user_happy() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        handle_create_user(
            "dave@ex".to_string(),
            "Dave".to_string(),
            "Developer".to_string(),
            None,
            None,
            vec![],
            vec![],
            make_get_user(store.clone()),
            make_put_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            cog_ok_create(),
            cog_ok_add(),
        )
        .await;

        let resp = handle_block_user(
            "U#dave@ex".to_string(),
            c2.to_string(),
            make_get_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
        )
        .await;

        assert_eq!(status(&resp), 200, "block_user should return 200; got {resp:?}");
        assert_eq!(body(&resp)["ok"].as_bool().unwrap_or(false), true);
    }

    // -----------------------------------------------------------------------
    // Test: unblock_user roundtrip  (OCaml: `unblock_user_roundtrip`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn unblock_user_roundtrip() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        handle_create_user(
            "eve@ex".to_string(),
            "Eve".to_string(),
            "Developer".to_string(),
            None,
            None,
            vec![],
            vec![],
            make_get_user(store.clone()),
            make_put_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
            make_list_administrated(store.clone()),
            make_list_blocked(store.clone()),
            make_delete_edge(store.clone()),
            make_delete_user(store.clone()),
            cog_ok_create(),
            cog_ok_add(),
        )
        .await;

        handle_block_user(
            "U#eve@ex".to_string(),
            c2.to_string(),
            make_get_user(store.clone()),
            make_get_node(store.clone()),
            make_put_edge(store.clone()),
        )
        .await;

        let resp = handle_unblock_user(
            "U#eve@ex".to_string(),
            c2.to_string(),
            make_delete_edge(store.clone()),
        )
        .await;

        assert_eq!(status(&resp), 200, "unblock should return 200; got {resp:?}");
    }

    // -----------------------------------------------------------------------
    // Test: attach_sensor happy  (OCaml: `attach_sensor_happy`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn attach_sensor_happy() {
        let store = Rc::new(Store::new());
        let c2 = seed_building_company(&store);
        let bldg = seed_building(&store, c2).await;

        let resp = handle_attach_sensor(
            bldg.to_string(),
            "daq:1".to_string(),
            "Electricity".to_string(),
            "counter".to_string(),
            Some("kWh".to_string()),
            Some(json!(15)),
            None,
            make_get_node(store.clone()),
            make_add_sensor(store.clone()),
            make_get_active(store.clone()),
            make_delete_sensor(store.clone()),
        )
        .await;

        assert_eq!(status(&resp), 200, "attach_sensor should return 200; got {resp:?}");
    }

    // -----------------------------------------------------------------------
    // Test: attach_sensor resample from string  (OCaml: `attach_sensor_resample_from_string`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn attach_sensor_resample_from_string() {
        let store = Rc::new(Store::new());
        let c2 = seed_building_company(&store);
        let bldg = seed_building(&store, c2).await;

        // "15" → 15
        let resp = handle_attach_sensor(
            bldg.to_string(),
            "daq:str".to_string(),
            "Electricity".to_string(),
            "counter".to_string(),
            None,
            Some(json!("15")),
            None,
            make_get_node(store.clone()),
            make_add_sensor(store.clone()),
            make_get_active(store.clone()),
            make_delete_sensor(store.clone()),
        )
        .await;

        assert_eq!(status(&resp), 200);
        assert_eq!(
            body(&resp)["resample_minutes"].as_i64().unwrap_or(-1),
            15,
            "string \"15\" should be coerced to 15"
        );

        // "" → null
        let resp2 = handle_attach_sensor(
            bldg.to_string(),
            "daq:empty".to_string(),
            "Electricity".to_string(),
            "counter".to_string(),
            None,
            Some(json!("")),
            None,
            make_get_node(store.clone()),
            make_add_sensor(store.clone()),
            make_get_active(store.clone()),
            make_delete_sensor(store.clone()),
        )
        .await;

        assert_eq!(status(&resp2), 200);
        assert!(body(&resp2)["resample_minutes"].is_null());
    }

    // -----------------------------------------------------------------------
    // Test: attach_sensor with formula  (OCaml: `attach_sensor_with_formula`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn attach_sensor_with_formula() {
        let store = Rc::new(Store::new());
        let c2 = seed_building_company(&store);
        let bldg = seed_building(&store, c2).await;

        // First sensor to reference.
        let s1 = handle_attach_sensor(
            bldg.to_string(),
            "daq:ref".to_string(),
            "Electricity".to_string(),
            "counter".to_string(),
            None,
            None,
            None,
            make_get_node(store.clone()),
            make_add_sensor(store.clone()),
            make_get_active(store.clone()),
            make_delete_sensor(store.clone()),
        )
        .await;
        assert_eq!(status(&s1), 200);
        let s1_id = body(&s1)["id"].as_str().expect("sensor id").to_string();

        let formula = json!({
            "kind": "expr",
            "expr": "abs(self - a)",
            "refs": { "a": s1_id }
        });

        let resp = handle_attach_sensor(
            bldg.to_string(),
            "daq:f".to_string(),
            "E".to_string(),
            "counter".to_string(),
            None,
            None,
            Some(formula),
            make_get_node(store.clone()),
            make_add_sensor(store.clone()),
            make_get_active(store.clone()),
            make_delete_sensor(store.clone()),
        )
        .await;

        assert_eq!(status(&resp), 200, "attach with formula; got {resp:?}");
        assert_eq!(body(&resp)["formula"]["kind"].as_str().unwrap_or(""), "expr");
    }

    // -----------------------------------------------------------------------
    // Test: attach_sensor unbound alias → 400  (OCaml: `attach_sensor_unbound_alias_400`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn attach_sensor_unbound_alias_400() {
        let store = Rc::new(Store::new());
        let c2 = seed_building_company(&store);
        let bldg = seed_building(&store, c2).await;

        let formula = json!({
            "kind": "expr",
            "expr": "self - a",
            "refs": {}
        });

        let resp = handle_attach_sensor(
            bldg.to_string(),
            "daq:bad".to_string(),
            "E".to_string(),
            "counter".to_string(),
            None,
            None,
            Some(formula),
            make_get_node(store.clone()),
            make_add_sensor(store.clone()),
            make_get_active(store.clone()),
            make_delete_sensor(store.clone()),
        )
        .await;

        assert_eq!(status(&resp), 400, "unbound alias should be 400; got {resp:?}");
        assert!(
            resp["body"].as_str().unwrap_or("").contains("unbound alias"),
            "response should mention unbound alias; body: {}",
            resp["body"]
        );
    }
}
