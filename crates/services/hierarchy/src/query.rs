//! Query dispatch — wires each JSON-query and HTML-query action to the model
//! logic layer.
//!
//! Per-query handler functions are generic over their injected-closure
//! dependencies (no traits; mirrors `dispatch.rs`).
//!
//! The prod entry-point `run_query` builds real repository closures from the
//! shared AWS clients and calls the matching handler.
//!
//! Ported 1:1 from:
//! - `services/hierarchy/lib/api/api_query.ml`  (JSON query actions)
//! - `services/hierarchy/lib/api/api_html.ml`   (HTML query actions)
//! - `services/hierarchy/lib/handler.ml`        (routing + response shape)

use std::future::Future;

use maud::Markup;
use serde_json::{json, Value};

use model::domain::ids::{NodeId, SensorId, UserId};
use model::domain::node::Node;
use model::domain::sensor::Sensor;
use model::domain::user::User;
use model::domain::values::{CognitoGroup, EdgeKind};
use model::errors::RepositoryError;
use model::logic::{access, hierarchy, users};
#[cfg(test)]
use model::logic::sensors;

use crate::dispatch::{node_ref_to_json, node_to_json, sensor_to_json, user_to_json};
use crate::html::{forms, node as html_node, tree};

// ---------------------------------------------------------------------------
// Response helpers — mirror api_json.ml ok_response / error_response
// ---------------------------------------------------------------------------

fn ok(body: Value) -> (u16, String) {
    (200, body.to_string())
}

fn bad_request(msg: &str) -> (u16, String) {
    (
        400,
        json!({ "error": { "code": "Bad_request", "message": msg } }).to_string(),
    )
}

fn repo_error(e: RepositoryError) -> (u16, String) {
    let (status, code, msg) = match &e {
        RepositoryError::NotFound(id) => (404u16, "Not_found", format!("{} not found", id)),
        RepositoryError::NotFoundUser(id) => (404, "Not_found", format!("{} not found", id)),
        RepositoryError::Conflict(m) => (409, "Conflict", m.clone()),
        RepositoryError::BadRequest(m) => (400, "Bad_request", m.clone()),
        RepositoryError::Validation(errs) => {
            let details: Vec<_> = errs
                .iter()
                .map(|e| json!({ "path": e.path, "message": e.message }))
                .collect();
            return (
                400,
                json!({ "error": { "code": "Validation", "message": "validation failed", "details": details } })
                    .to_string(),
            );
        }
        RepositoryError::SchemaMissing(id) => (
            400,
            "Schema_missing",
            format!("no hn2 schema found above {}", id),
        ),
        RepositoryError::Codec(m) => (500, "Internal", m.clone()),
        RepositoryError::Aws(m) => (500, "Internal", m.clone()),
    };
    (
        status,
        json!({ "error": { "code": code, "message": msg } }).to_string(),
    )
}

/// HTML-query response helpers.
fn html_ok(markup: Markup) -> (u16, String) {
    (200, markup.into_string())
}

fn html_error(msg: &str) -> (u16, String) {
    use maud::html;
    (400, html! { div class="error" { (msg) } }.into_string())
}

fn html_repo_error(e: RepositoryError) -> (u16, String) {
    let status: u16 = match &e {
        RepositoryError::NotFound(_) | RepositoryError::NotFoundUser(_) => 404,
        RepositoryError::Conflict(_) => 409,
        RepositoryError::BadRequest(_)
        | RepositoryError::Validation(_)
        | RepositoryError::SchemaMissing(_) => 400,
        RepositoryError::Codec(_) | RepositoryError::Aws(_) => 500,
    };
    let msg = match &e {
        RepositoryError::NotFound(id) => format!("{} not found", id),
        RepositoryError::NotFoundUser(id) => format!("{} not found", id),
        RepositoryError::Conflict(m)
        | RepositoryError::BadRequest(m)
        | RepositoryError::Codec(m)
        | RepositoryError::Aws(m) => m.clone(),
        RepositoryError::Validation(errs) => errs
            .first()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| "validation failed".to_string()),
        RepositoryError::SchemaMissing(id) => format!("no hn2 schema found above {}", id),
    };
    use maud::html;
    (status, html! { div class="error" { (msg) } }.into_string())
}

// ---------------------------------------------------------------------------
// JSON query handlers
// ---------------------------------------------------------------------------

/// `GET /query/get_node?id=HN2#...`
pub async fn handle_get_node<FGN, FGNFut>(id_s: &str, get_node: FGN) -> (u16, String)
where
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
{
    let node_id = match NodeId::parse(id_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad id: {}", e)),
    };
    match hierarchy::get_node(node_id, get_node).await {
        Ok(n) => ok(node_to_json(&n)),
        Err(e) => repo_error(e),
    }
}

/// `GET /query/list_children?parent=HN2#...&label=...&full=true`
pub async fn handle_list_children<FLC, FLCFut, FLCR, FLCRFut>(
    parent_s: &str,
    label: Option<String>,
    full: bool,
    list_children_fn: FLC,
    list_child_refs_fn: FLCR,
) -> (u16, String)
where
    FLC: FnOnce(NodeId, Option<EdgeKind>) -> FLCFut,
    FLCFut: Future<Output = Result<Vec<Node>, RepositoryError>>,
    FLCR: FnOnce(NodeId, Option<EdgeKind>) -> FLCRFut,
    FLCRFut: Future<Output = Result<Vec<(NodeId, String)>, RepositoryError>>,
{
    let parent = match NodeId::parse(parent_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad parent: {}", e)),
    };
    if full {
        match hierarchy::list_children(parent, label, list_children_fn).await {
            Ok(ns) => ok(json!({ "children": ns.iter().map(node_to_json).collect::<Vec<_>>() })),
            Err(e) => repo_error(e),
        }
    } else {
        match hierarchy::list_child_refs(parent, label, list_child_refs_fn).await {
            Ok(refs) => ok(json!({ "children": refs.iter().map(|(id, name)| node_ref_to_json(id, name)).collect::<Vec<_>>() })),
            Err(e) => repo_error(e),
        }
    }
}

/// `GET /query/list_sensors?parent=HN4#...`
#[cfg(test)]
pub async fn handle_list_sensors<FLS, FLSFut, FGA>(
    parent_s: &str,
    list_sensor_ids: FLS,
    get_active_sensor: FGA,
) -> (u16, String)
where
    FLS: FnOnce(NodeId) -> FLSFut,
    FLSFut: Future<Output = Result<Vec<SensorId>, RepositoryError>>,
    FGA: Fn(SensorId) -> Option<Sensor>,
{
    let parent = match NodeId::parse(parent_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad parent: {}", e)),
    };
    match sensors::list_active(parent, list_sensor_ids, get_active_sensor).await {
        Ok(xs) => ok(json!({ "sensors": xs.iter().map(sensor_to_json).collect::<Vec<_>>() })),
        Err(e) => repo_error(e),
    }
}


/// `GET /query/get_user?id=U#email`
pub async fn handle_get_user<FGU, FGUFut>(id_s: &str, get_user: FGU) -> (u16, String)
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
{
    let uid = match UserId::parse(id_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad id: {}", e)),
    };
    let uid_clone = uid.clone();
    match get_user(uid).await {
        Ok(Some(u)) => ok(user_to_json(&u)),
        Ok(None) => repo_error(RepositoryError::NotFoundUser(uid_clone)),
        Err(e) => repo_error(e),
    }
}

/// `GET /query/list_users`
pub async fn handle_list_users<FLU, FLUFut>(list_users_fn: FLU) -> (u16, String)
where
    FLU: FnOnce() -> FLUFut,
    FLUFut: Future<Output = Result<Vec<User>, RepositoryError>>,
{
    match users::list(list_users_fn).await {
        Ok(xs) => ok(json!({ "users": xs.iter().map(user_to_json).collect::<Vec<_>>() })),
        Err(e) => repo_error(e),
    }
}

/// `GET /query/list_blocked_nodes?user=U#email`
pub async fn handle_list_blocked_nodes<FLB, FLBFut>(
    user_s: &str,
    list_blocked: FLB,
) -> (u16, String)
where
    FLB: FnOnce(UserId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
{
    let uid = match UserId::parse(user_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad user: {}", e)),
    };
    match access::list_blocked_nodes(uid, list_blocked).await {
        Ok(ids) => ok(json!({ "nodes": ids.iter().map(|n| n.to_string()).collect::<Vec<_>>() })),
        Err(e) => repo_error(e),
    }
}

/// `GET /query/list_blocked_users?node=HN2#...`
pub async fn handle_list_blocked_users<FLB, FLBFut>(
    node_s: &str,
    list_blocked: FLB,
) -> (u16, String)
where
    FLB: FnOnce(NodeId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<UserId>, RepositoryError>>,
{
    let nid = match NodeId::parse(node_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad node: {}", e)),
    };
    match access::list_blocked_users(nid, list_blocked).await {
        Ok(ids) => ok(json!({ "users": ids.iter().map(|u| u.to_string()).collect::<Vec<_>>() })),
        Err(e) => repo_error(e),
    }
}

/// `GET /query/effective_permission?user=U#...&node=HN2#...`
pub async fn handle_effective_permission<FGU, FGUFut, FGN, FGNFut, FLB, FLBFut, FLA, FLAFut>(
    user_s: &str,
    node_s: &str,
    get_user: FGU,
    get_node: FGN,
    list_blocked: FLB,
    list_access_edges: FLA,
) -> (u16, String)
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FLB: FnOnce(UserId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
    FLA: FnOnce(UserId) -> FLAFut,
    FLAFut: Future<Output = Result<Vec<(NodeId, EdgeKind)>, RepositoryError>>,
{
    let uid = match UserId::parse(user_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad user: {}", e)),
    };
    let nid = match NodeId::parse(node_s) {
        Ok(id) => id,
        Err(e) => return bad_request(&format!("bad node: {}", e)),
    };
    match access::effective_permission(uid, nid, get_user, get_node, list_blocked, list_access_edges).await {
        Ok(Some(g)) => ok(json!({ "capability": g.to_string() })),
        Ok(None) => ok(json!({ "capability": null, "reason": "blocked" })),
        Err(e) => repo_error(e),
    }
}

// ---------------------------------------------------------------------------
// HTML query handlers
// ---------------------------------------------------------------------------

/// `GET /hierarchy/query/nodes?id=...&user=...&path=...&permissions=...`
///
/// Top-level (no id): uses `access::start_nodes` rule — root grant expands
/// to root's children, otherwise the administrated nodes are shown directly.
/// Drilling (id provided): gated by `access::has_admin_access`.
/// Mirrors OCaml `api_html.ml :: render_nodes`.
///
/// `get_node` must be `Fn + Clone` so it can be called once per administrated
/// node when resolving names in the top-level non-root path.
pub async fn handle_nodes<FLA, FLAFut, FLCR, FLCRFut, FGN, FGNFut>(
    id_opt: Option<&str>,
    user_s: &str,
    path_opt: Option<&str>,
    with_permissions: bool,
    list_administrated: FLA,
    list_child_refs: FLCR,
    get_node: FGN,
) -> (u16, String)
where
    FLA: FnOnce(UserId) -> FLAFut,
    FLAFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
    FLCR: Fn(NodeId, Option<EdgeKind>) -> FLCRFut + Clone,
    FLCRFut: Future<Output = Result<Vec<(NodeId, String)>, RepositoryError>>,
    FGN: Fn(NodeId) -> FGNFut + Clone,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
{
    // The hx-get URLs embed the user string as-is (e.g. "U#..." or bare email).
    let user_str = user_s;

    // Build a "U#..." prefix for UserId::parse.
    let normalized = if user_s.starts_with("U#") {
        user_s.to_string()
    } else if user_s.is_empty() {
        String::new()
    } else {
        format!("U#{}", user_s)
    };

    let uid = match UserId::parse(&normalized) {
        Ok(id) => id,
        // Unknown/empty user → render nothing (HTMX "nothing to show").
        Err(_) => return html_ok(maud::html! {}),
    };

    let refs_result: Result<Vec<(NodeId, String)>, RepositoryError> = match id_opt {
        Some(id_s) if !id_s.is_empty() => {
            // Drilling into a specific node — gated by admin access check.
            let parent = match NodeId::parse(id_s) {
                Ok(id) => id,
                Err(e) => return html_error(&format!("bad id: {}", e)),
            };
            let has_access = access::has_admin_access(
                uid.clone(),
                parent.clone(),
                list_administrated,
                {
                    let gn = get_node.clone();
                    move |nid| gn(nid)
                },
            )
            .await
            .unwrap_or(false);
            if has_access {
                hierarchy::list_child_refs(parent, None, list_child_refs).await
            } else {
                Ok(vec![])
            }
        }
        _ => {
            // Top-level — apply start_nodes rule.
            let grants =
                match access::list_administrated_nodes(uid.clone(), list_administrated).await {
                    Ok(g) => g,
                    Err(e) => return html_repo_error(e),
                };

            if grants.iter().any(|n| n.is_root()) {
                // Root grant → list root's direct children.
                hierarchy::list_child_refs(NodeId::root(), None, list_child_refs).await
            } else {
                // Return the administrated nodes as start nodes (with real names).
                // Mirrors OCaml: List.filter_map (fun g -> match Hierarchy.get_node g with ...) grants
                let mut refs = Vec::new();
                for g in grants {
                    if let Ok(Some(n)) = get_node(g.clone()).await {
                        refs.push((g, n.name));
                    }
                }
                Ok(refs)
            }
        }
    };

    match refs_result {
        Err(e) => html_repo_error(e),
        Ok(refs) => {
            // parent_path for child items:
            //   - drilling (id provided) → use the path_opt from the request
            //   - top-level              → "H#root"
            let parent_path = match id_opt {
                Some(s) if !s.is_empty() => path_opt.unwrap_or(""),
                _ => "H#root",
            };
            let markup = tree::render_nodes(&refs, user_str, parent_path, with_permissions);
            html_ok(markup)
        }
    }
}

/// `GET /hierarchy/query/node?id=HN2#...&user=U#...`
///
/// Computes the acting user's effective capability on the node and passes it
/// to `render_node` so the UI is gated correctly.
pub async fn handle_node<FGN, FGNFut, FGU, FGUFut, FLB, FLBFut, FLA, FLAFut>(
    id_s: &str,
    user_s: &str,
    get_node: FGN,
    get_user: FGU,
    list_blocked_nodes: FLB,
    list_access_edges: FLA,
) -> (u16, String)
where
    FGN: FnOnce(NodeId) -> FGNFut + Clone,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FLB: FnOnce(UserId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
    FLA: FnOnce(UserId) -> FLAFut,
    FLAFut: Future<Output = Result<Vec<(NodeId, EdgeKind)>, RepositoryError>>,
{
    let nid = match NodeId::parse(id_s) {
        Ok(id) => id,
        Err(e) => return html_error(&format!("bad id: {}", e)),
    };

    // Compute capability: normalise user string to "U#..." prefix.
    let normalized_user = if user_s.starts_with("U#") {
        user_s.to_string()
    } else if user_s.is_empty() {
        String::new()
    } else {
        format!("U#{}", user_s)
    };

    let capability: Option<CognitoGroup> = match UserId::parse(&normalized_user) {
        Ok(uid) => access::effective_permission(
            uid,
            nid.clone(),
            get_user,
            get_node.clone(),
            list_blocked_nodes,
            list_access_edges,
        )
        .await
        .unwrap_or(None),
        Err(_) => None,
    };

    match hierarchy::get_node(nid, get_node).await {
        Ok(n) => {
            // show_sensors: whether the schema allows sensors at this node's level.
            // The schema is carried on the node itself in the Rust model.
            let level = n.id.level();
            let show_sensors = n
                .schema
                .as_ref()
                .map(|s| s.allows_sensors(level))
                .unwrap_or(false);
            html_ok(html_node::render_node(&n, show_sensors, capability))
        }
        Err(e) => html_repo_error(e),
    }
}


/// `GET /hierarchy/query/company_sensors?nodepath=...`
///
/// Returns `<option>` elements for active sensors under the given node's
/// path prefix. Mirrors OCaml `render_company_sensors`.
pub async fn handle_company_sensors<FLS, FLSFut>(
    nodepath: &str,
    list_under_path: FLS,
) -> (u16, String)
where
    FLS: FnOnce(String) -> FLSFut,
    FLSFut: Future<Output = Result<Vec<Sensor>, RepositoryError>>,
{
    let last = leaf_node_id(nodepath);
    let nid = match NodeId::parse(last) {
        Ok(id) => id,
        Err(e) => return html_error(&format!("bad nodepath: {}", e)),
    };
    let path_prefix = nid.to_string();
    match list_under_path(path_prefix).await {
        Ok(ss) => {
            use maud::html;
            let markup = html! {
                @for s in &ss {
                    @let id_s = s.id.to_string();
                    option value=(id_s) {
                        (s.daq_id) " (" (s.purpose) ")"
                    }
                }
            };
            html_ok(markup)
        }
        Err(e) => html_repo_error(e),
    }
}

/// `GET /hierarchy/query/users` — HTML table rows.
pub async fn handle_html_users<FLU, FLUFut>(list_users_fn: FLU) -> (u16, String)
where
    FLU: FnOnce() -> FLUFut,
    FLUFut: Future<Output = Result<Vec<User>, RepositoryError>>,
{
    use maud::html;
    match users::list(list_users_fn).await {
        Err(e) => html_repo_error(e),
        Ok(us) => {
            let markup = if us.is_empty() {
                html! {
                    tr {
                        td colspan="6"
                            style="text-align: center; color: var(--text-muted); padding: 1rem;"
                        {
                            "No users found"
                        }
                    }
                }
            } else {
                html! {
                    @for u in &us {
                        @let email = u.email.clone();
                        tr {
                            td { (u.name) }
                            td { (email) }
                            td { (u.cognito_group.to_string()) }
                            td { (u.language.to_string()) }
                            td { (u.currency.to_string()) }
                            td {
                                button
                                    class="btn-danger btn-sm"
                                    onclick=(format!(
                                        "window.dispatchEvent(new CustomEvent('delete-user', {{ detail: {{ email: '{}' }} }}))",
                                        email
                                    ))
                                {
                                    "Slet"
                                }
                            }
                        }
                    }
                }
            };
            html_ok(markup)
        }
    }
}

/// `GET /hierarchy/query/add_child_form?parent=HN2#...&level=...`
///
/// Renders the add-child dialog body.  Mirrors OCaml `render_add_child_form`.
pub async fn handle_add_child_form<FGN, FGNFut>(
    parent_s: &str,
    level_s: Option<&str>,
    get_node: FGN,
) -> (u16, String)
where
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
{
    use model::domain::ids::Level;
    use model::domain::schema::EdgeSpec as SchemaEdgeSpec;

    let parent_id = match NodeId::parse(parent_s) {
        Ok(id) => id,
        Err(e) => return html_error(&format!("bad parent: {}", e)),
    };

    let parent_node = match hierarchy::get_node(parent_id.clone(), get_node).await {
        Ok(n) => n,
        Err(e) => return html_repo_error(e),
    };

    let parent_level = parent_node.id.level();

    // Determine allowed child levels — mirrors OCaml `allowed` computation.
    type AllowedEntry = (Level, Vec<SchemaEdgeSpec>);
    let allowed: Vec<AllowedEntry> = match parent_level {
        Level::Hn0 => vec![(
            Level::Hn1,
            vec![SchemaEdgeSpec::builder()
                .label("partner".to_string())
                .build()],
        )],
        Level::Hn1 => vec![(
            Level::Hn2,
            vec![SchemaEdgeSpec::builder()
                .label("company".to_string())
                .build()],
        )],
        _ => {
            // Get allowed children from the schema on this node.
            match &parent_node.schema {
                Some(sch) => {
                    sch.allowed_children(parent_level)
                        .iter()
                        .map(|(child_level, edge_specs)| (*child_level, edge_specs.clone()))
                        .collect()
                }
                None => vec![],
            }
        }
    };

    if allowed.is_empty() {
        use maud::html;
        return html_ok(html! {
            div class="error" {
                "This node type cannot have children according to its schema."
            }
        });
    }

    // Pick the chosen level.
    let requested_level = level_s.and_then(|s| s.parse::<Level>().ok());
    let chosen_level = match requested_level {
        Some(l) if allowed.iter().any(|(al, _)| *al == l) => l,
        _ => allowed[0].0,
    };

    let labels: Vec<String> = allowed
        .iter()
        .find(|(l, _)| *l == chosen_level)
        .map(|(_, specs)| specs.iter().map(|s| s.label.clone()).collect())
        .unwrap_or_default();

    let metadata_fields: Vec<(String, model::domain::schema::FieldSpec)> =
        match &parent_node.schema {
            Some(sch) => sch
                .metadata_for(chosen_level)
                .iter()
                .map(|(n, s)| (n.clone(), s.clone()))
                .collect(),
            None => vec![],
        };

    // Build level option pairs: (value_str, is_selected).
    let chosen_level_s = chosen_level.to_string();
    let level_options_owned: Vec<(String, bool)> = allowed
        .iter()
        .map(|(l, _)| (l.to_string(), *l == chosen_level))
        .collect();
    let level_options: Vec<(&str, bool)> = level_options_owned
        .iter()
        .map(|(s, b)| (s.as_str(), *b))
        .collect();

    let label_options_owned: Vec<(String, bool)> = labels
        .iter()
        .enumerate()
        .map(|(i, s)| (s.clone(), i == 0))
        .collect();
    let label_options: Vec<(&str, bool)> = label_options_owned
        .iter()
        .map(|(s, b)| (s.as_str(), *b))
        .collect();

    let is_multi = allowed.len() > 1;

    // Build metadata inputs.
    let metadata_inputs = build_metadata_inputs(&metadata_fields);

    html_ok(forms::render_add_child_form(
        parent_s,
        &chosen_level_s,
        &level_options,
        &label_options,
        metadata_inputs,
        is_multi,
    ))
}

/// Build metadata form inputs from schema field specs.
fn build_metadata_inputs(
    fields: &[(String, model::domain::schema::FieldSpec)],
) -> Markup {
    use maud::html;
    use model::domain::values::FieldType;

    html! {
        @for (fname, spec) in fields {
            @let req_attr = spec.required;
            div class="form-row" {
                label class="form-label" { (fname) }
                @match &spec.typ {
                    FieldType::String { .. } => {
                        input type="text"
                            name=(format!("data.metadata.{}", fname))
                            class="form-input"
                            required[req_attr];
                    }
                    FieldType::Number { .. } => {
                        input type="number" step="any"
                            name=(format!("data.metadata.{}", fname))
                            class="form-input"
                            required[req_attr];
                    }
                    FieldType::Integer { .. } => {
                        input type="number" step="1"
                            name=(format!("data.metadata.{}", fname))
                            class="form-input"
                            required[req_attr];
                    }
                    FieldType::Boolean => {
                        select
                            name=(format!("data.metadata.{}", fname))
                            class="form-select"
                            required[req_attr]
                        {
                            option value="true" { "true" }
                            option value="false" { "false" }
                        }
                    }
                    FieldType::Timestamp => {
                        input type="datetime-local"
                            name=(format!("data.metadata.{}", fname))
                            class="form-input"
                            required[req_attr];
                    }
                    FieldType::Enum { one_of } => {
                        select
                            name=(format!("data.metadata.{}", fname))
                            class="form-select"
                            required[req_attr]
                        {
                            @for v in one_of {
                                option value=(v) { (v) }
                            }
                        }
                    }
                }
                @if spec.required {
                    span class="required" { "*" }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTML query: static option renderers
// ---------------------------------------------------------------------------

pub fn handle_profiles() -> (u16, String) {
    html_ok(forms::render_profiles())
}

pub fn handle_languages() -> (u16, String) {
    html_ok(forms::render_languages())
}

pub fn handle_currencies() -> (u16, String) {
    html_ok(forms::render_currencies())
}

pub fn handle_permissions() -> (u16, String) {
    html_ok(forms::render_permissions())
}

pub fn handle_timezones() -> (u16, String) {
    html_ok(forms::render_timezones())
}

// ---------------------------------------------------------------------------
// leaf_node_id — mirrors OCaml `api_html.ml :: leaf_node_id`
//
// Extract the last `HN{n}#<id>` segment from a nodepath string.
// The path can be pipe-separated (storage format) or hash-separated (URL).
// ---------------------------------------------------------------------------

pub fn leaf_node_id(nodepath: &str) -> &str {
    let bytes = nodepath.as_bytes();
    let len = bytes.len();
    if len < 2 {
        return nodepath;
    }
    // Scan from the right for the last occurrence of 'H' followed by 'N'.
    let mut i = len;
    while i >= 2 {
        i -= 1;
        if bytes[i] == b'H' && i + 1 < len && bytes[i + 1] == b'N' {
            return &nodepath[i..];
        }
    }
    nodepath
}

// ---------------------------------------------------------------------------
// Prod entry-point: run_query
// ---------------------------------------------------------------------------

/// Dispatch a query action to the real AWS-backed repository.
///
/// `action` is the path component after `/query/` or `/hierarchy/query/`.
/// `params` is the query-string parameter list.
/// Returns `(status_code, body_string)`.
pub async fn run_query(action: &str, params: &[(String, String)]) -> (u16, String) {
    use model::repository::dynamodb::{node as ddb_node, sensor as ddb_sensor, user};

    let ddb = model::get_dynamodb_client().await;
    let table = model::get_table_name();

    fn p<'a>(params: &'a [(String, String)], k: &str) -> Option<&'a str> {
        params.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
    }

    match action {
        // ----- JSON queries -----

        "get_node" => {
            let id_s = match p(params, "id") {
                Some(s) => s,
                None => return bad_request("missing id"),
            };
            handle_get_node(id_s, {
                let t = table.clone();
                move |nid| async move { ddb_node::get_node(ddb, &t, &nid).await }
            })
            .await
        }

        "list_children" => {
            let parent_s = match p(params, "parent") {
                Some(s) => s,
                None => return bad_request("missing parent"),
            };
            let label = p(params, "label").map(String::from);
            let full = matches!(p(params, "full"), Some("true") | Some("1"));
            handle_list_children(
                parent_s,
                label,
                full,
                {
                    let t = table.clone();
                    move |pid, kind| async move {
                        ddb_node::list_children(ddb, &t, &pid, kind.as_ref()).await
                    }
                },
                {
                    let t = table.clone();
                    move |pid, kind| async move {
                        ddb_node::list_child_refs(ddb, &t, &pid, kind.as_ref()).await
                    }
                },
            )
            .await
        }

        "list_sensors" => {
            let parent_s = match p(params, "parent") {
                Some(s) => s,
                None => return bad_request("missing parent"),
            };
            // DDB: list_sensor_ids is async; get_active_sensor is also async but
            // sensors::list_active takes a sync Fn. Use a two-step: fetch IDs
            // asynchronously, then resolve each via async DDB query.
            let parent = match NodeId::parse(parent_s) {
                Ok(id) => id,
                Err(e) => return bad_request(&format!("bad parent: {}", e)),
            };
            let t = table.clone();
            let ids = match ddb_sensor::list_sensor_ids(ddb, &t, &parent).await {
                Ok(ids) => ids,
                Err(e) => return repo_error(e),
            };
            let mut result = Vec::new();
            for sid in ids {
                match ddb_sensor::get_active_sensor(ddb, &t, &sid).await {
                    Ok(Some(s)) => result.push(s),
                    Ok(None) => {}
                    Err(e) => return repo_error(e),
                }
            }
            ok(json!({ "sensors": result.iter().map(sensor_to_json).collect::<Vec<_>>() }))
        }

        "get_sensor" => {
            let id_s = match p(params, "id") {
                Some(s) => s,
                None => return bad_request("missing id"),
            };
            let sid = match SensorId::parse(id_s) {
                Ok(id) => id,
                Err(e) => return bad_request(&format!("bad id: {}", e)),
            };
            let t = table.clone();
            match ddb_sensor::get_active_sensor(ddb, &t, &sid).await {
                Ok(Some(s)) => ok(sensor_to_json(&s)),
                Ok(None) => repo_error(RepositoryError::NotFound(NodeId::make(
                    model::domain::ids::Level::Hn9,
                    sid.id(),
                ))),
                Err(e) => repo_error(e),
            }
        }

        "get_user" => {
            let id_s = match p(params, "id") {
                Some(s) => s,
                None => return bad_request("missing id"),
            };
            handle_get_user(id_s, {
                let t = table.clone();
                move |uid| async move { user::get_user(ddb, &t, &uid).await }
            })
            .await
        }

        "list_users" => {
            handle_list_users({
                let t = table.clone();
                move || async move { user::list_users(ddb, &t).await }
            })
            .await
        }

        "list_blocked_nodes" => {
            let user_s = match p(params, "user") {
                Some(s) => s,
                None => return bad_request("missing user"),
            };
            handle_list_blocked_nodes(user_s, {
                let t = table.clone();
                move |uid| async move { user::list_blocked_nodes(ddb, &t, &uid).await }
            })
            .await
        }

        "list_blocked_users" => {
            let node_s = match p(params, "node") {
                Some(s) => s,
                None => return bad_request("missing node"),
            };
            handle_list_blocked_users(node_s, {
                let t = table.clone();
                move |nid| async move { user::list_blocked_users(ddb, &t, &nid).await }
            })
            .await
        }

        "effective_permission" => {
            let user_s = match p(params, "user") {
                Some(s) => s,
                None => return bad_request("missing user"),
            };
            let node_s = match p(params, "node") {
                Some(s) => s,
                None => return bad_request("missing node"),
            };
            handle_effective_permission(
                user_s,
                node_s,
                {
                    let t = table.clone();
                    move |uid| async move { user::get_user(ddb, &t, &uid).await }
                },
                {
                    let t = table.clone();
                    move |nid| async move { ddb_node::get_node(ddb, &t, &nid).await }
                },
                {
                    let t = table.clone();
                    move |uid| async move { user::list_blocked_nodes(ddb, &t, &uid).await }
                },
                {
                    let t = table.clone();
                    move |uid| async move { user::list_access_edges(ddb, &t, &uid).await }
                },
            )
            .await
        }

        // ----- HTML queries -----

        "nodes" => {
            let id_opt = p(params, "id");
            let user_s = p(params, "user").unwrap_or("");
            let path_opt = p(params, "path");
            let with_perms = matches!(p(params, "permissions"), Some("true"));

            handle_nodes(
                id_opt,
                user_s,
                path_opt,
                with_perms,
                {
                    let t = table.clone();
                    move |uid| async move { user::list_administrated_nodes(ddb, &t, &uid).await }
                },
                {
                    let t = table.clone();
                    move |pid, kind: Option<EdgeKind>| {
                        let t = t.clone();
                        async move {
                            ddb_node::list_child_refs(ddb, &t, &pid, kind.as_ref()).await
                        }
                    }
                },
                {
                    let t = table.clone();
                    move |nid| {
                        let t = t.clone();
                        async move { ddb_node::get_node(ddb, &t, &nid).await }
                    }
                },
            )
            .await
        }

        "node" => {
            let id_s = match p(params, "id") {
                Some(s) => s,
                None => return html_error("missing id"),
            };
            let user_s = p(params, "user").unwrap_or("");
            handle_node(
                id_s,
                user_s,
                {
                    let t = table.clone();
                    move |nid| async move { ddb_node::get_node(ddb, &t, &nid).await }
                },
                {
                    let t = table.clone();
                    move |uid| async move { user::get_user(ddb, &t, &uid).await }
                },
                {
                    let t = table.clone();
                    move |uid| async move { user::list_blocked_nodes(ddb, &t, &uid).await }
                },
                {
                    let t = table.clone();
                    move |uid| async move { user::list_access_edges(ddb, &t, &uid).await }
                },
            )
            .await
        }

        "sensors" => {
            let nodepath = match p(params, "nodepath") {
                Some(s) => s,
                None => return html_error("missing nodepath"),
            };
            // Same async two-step as list_sensors above.
            let last = leaf_node_id(nodepath);
            let nid = match NodeId::parse(last) {
                Ok(id) => id,
                Err(e) => return html_error(&format!("bad nodepath: {}", e)),
            };
            let t = table.clone();
            let ids = match ddb_sensor::list_sensor_ids(ddb, &t, &nid).await {
                Ok(ids) => ids,
                Err(e) => return html_repo_error(e),
            };
            let mut ss = Vec::new();
            for sid in ids {
                if let Ok(Some(s)) = ddb_sensor::get_active_sensor(ddb, &t, &sid).await {
                    ss.push(s);
                }
            }
            html_ok(html_node::render_sensors(&ss))
        }

        "company_sensors" => {
            let nodepath = match p(params, "nodepath") {
                Some(s) => s,
                None => return html_error("missing nodepath"),
            };
            handle_company_sensors(nodepath, {
                let t = table.clone();
                move |prefix| async move {
                    ddb_sensor::list_sensors_under_path(ddb, &t, &prefix).await
                }
            })
            .await
        }

        "users" => {
            handle_html_users({
                let t = table.clone();
                move || async move { user::list_users(ddb, &t).await }
            })
            .await
        }

        "add_child_form" => {
            let parent_s = match p(params, "parent") {
                Some(s) => s,
                None => return html_error("missing parent"),
            };
            let level_s = p(params, "level");
            handle_add_child_form(parent_s, level_s, {
                let t = table.clone();
                move |nid| async move { ddb_node::get_node(ddb, &t, &nid).await }
            })
            .await
        }

        "profiles" => handle_profiles(),
        "languages" => handle_languages(),
        "currencies" => handle_currencies(),
        "permissions" => handle_permissions(),
        "timezones" => handle_timezones(),

        other => bad_request(&format!("unknown query action {:?}", other)),
    }
}

// ---------------------------------------------------------------------------
// Tests — port of `services/hierarchy/test/test_api_query.ml` 1:1
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use serde_json::json;

    use model::domain::ids::{Level, NodeId, UserId};
    use model::domain::node;
    use model::domain::schema::{EdgeSpec as SchemaEdgeSpec, Schema};
    use model::domain::values::{CognitoGroup, EdgeKind};
    use model::errors::RepositoryError;
    use model::repository::memory::Store;
    use model::repository::EdgeSpec as RepoEdgeSpec;

    use super::*;

    // -----------------------------------------------------------------------
    // Closure factories over a shared Rc<Store>
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
    ) -> impl Fn(RepoEdgeSpec) -> std::future::Ready<Result<(), RepositoryError>> + Clone {
        move |spec| {
            s.put_edge(spec);
            std::future::ready(Ok(()))
        }
    }

    fn make_list_administrated(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<Vec<NodeId>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_administrated_nodes(&uid)))
    }

    fn make_list_blocked_nodes_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<Vec<NodeId>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_blocked_nodes(&uid)))
    }

    fn make_list_access_edges(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId)
           -> std::future::Ready<Result<Vec<(NodeId, EdgeKind)>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_access_edges(&uid)))
    }

    fn make_list_blocked_users_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(NodeId) -> std::future::Ready<Result<Vec<UserId>, RepositoryError>> {
        move |nid| std::future::ready(Ok(s.list_blocked_users(&nid)))
    }

    fn make_list_users(
        s: Rc<Store>,
    ) -> impl FnOnce() -> std::future::Ready<Result<Vec<model::domain::user::User>, RepositoryError>>
    {
        move || std::future::ready(Ok(s.list_users()))
    }

    fn make_list_sensor_ids(
        s: Rc<Store>,
    ) -> impl FnOnce(NodeId)
           -> std::future::Ready<Result<Vec<model::domain::ids::SensorId>, RepositoryError>>
    {
        move |nid| std::future::ready(Ok(s.list_sensor_ids(&nid)))
    }

    fn make_list_children(
        s: Rc<Store>,
    ) -> impl FnOnce(
        NodeId,
        Option<EdgeKind>,
    ) -> std::future::Ready<Result<Vec<node::Node>, RepositoryError>> {
        move |nid, kind| std::future::ready(Ok(s.list_children(&nid, kind.as_ref())))
    }

    fn make_list_child_refs(
        s: Rc<Store>,
    ) -> impl Fn(
        NodeId,
        Option<EdgeKind>,
    ) -> std::future::Ready<Result<Vec<(NodeId, String)>, RepositoryError>>
           + Clone {
        move |nid, kind| std::future::ready(Ok(s.list_child_refs(&nid, kind.as_ref())))
    }

    fn make_add_node(
        s: Rc<Store>,
    ) -> impl FnOnce(
        Level,
        Box<dyn Fn(u32) -> (node::Node, RepoEdgeSpec) + Send>,
    ) -> std::future::Ready<Result<node::Node, RepositoryError>> {
        move |level, build| {
            let n = s.add_node(level, build);
            std::future::ready(Ok(n))
        }
    }

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

    /// Seed an HN2 company node with one HN3 property child.
    /// Returns the c2 NodeId.
    async fn seed_with_one_child(store: &Rc<Store>) -> NodeId {
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

        model::logic::hierarchy::add_node(
            c2.clone(),
            Some(Level::Hn3),
            None,
            "P".to_string(),
            json!({}),
            None,
            make_get_node(store.clone()),
            make_list_children(store.clone()),
            make_add_node(store.clone()),
        )
        .await
        .expect("seed add_node");

        c2
    }

    /// Create a user in the store and return their UserId.
    async fn seed_user(store: &Rc<Store>, email: &str, name: &str) -> UserId {
        model::logic::users::create(
            email.to_string(),
            name.to_string(),
            CognitoGroup::Writer,
            None,
            None,
            make_get_user(store.clone()),
            make_put_user(store.clone()),
        )
        .await
        .expect("seed user")
        .id
    }

    // -----------------------------------------------------------------------
    // test: get_node returns node
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_node_returns_node() {
        let store = Rc::new(Store::new());
        let c2 = seed_with_one_child(&store).await;

        let (status, body) = handle_get_node(
            &c2.to_string(),
            {
                let s = store.clone();
                move |nid| std::future::ready(Ok(s.get_node(&nid)))
            },
        )
        .await;

        assert_eq!(status, 200, "status");
        let jval: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(jval["id"].as_str(), Some(c2.to_string().as_str()));
    }

    // -----------------------------------------------------------------------
    // test: list_children returns array
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_children_returns_array() {
        let store = Rc::new(Store::new());
        let c2 = seed_with_one_child(&store).await;

        let (status, body) = handle_list_children(
            &c2.to_string(),
            None,
            false,
            make_list_children(store.clone()),
            make_list_child_refs(store.clone()),
        )
        .await;

        assert_eq!(status, 200, "status");
        let jval: serde_json::Value = serde_json::from_str(&body).unwrap();
        let children = jval["children"].as_array().expect("children array");
        assert_eq!(children.len(), 1, "one child");
    }

    // -----------------------------------------------------------------------
    // test: unknown action → 400
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn unknown_action_is_bad_request() {
        let (status, _body) = bad_request("unknown query action \"does_not_exist\"");
        assert_eq!(status, 400, "status 400");
    }

    // -----------------------------------------------------------------------
    // test: list_sensors empty
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_sensors_empty() {
        let store = Rc::new(Store::new());
        let parent = NodeId::make(Level::Hn4, 10042);

        let (status, body) = handle_list_sensors(
            &parent.to_string(),
            make_list_sensor_ids(store.clone()),
            {
                let s = store.clone();
                move |sid| s.get_active_sensor(&sid)
            },
        )
        .await;

        assert_eq!(status, 200, "status 200");
        let jval: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(jval["sensors"].as_array().map(|a| a.len()), Some(0));
    }

    // -----------------------------------------------------------------------
    // test: get_user happy
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_user_happy() {
        let store = Rc::new(Store::new());
        seed_user(&store, "carol@ex", "Carol").await;

        let (status, _body) = handle_get_user("U#carol@ex", make_get_user(store.clone())).await;
        assert_eq!(status, 200, "status 200");
    }

    // -----------------------------------------------------------------------
    // test: list_users empty
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_users_empty() {
        let store = Rc::new(Store::new());

        let (status, body) = handle_list_users(make_list_users(store.clone())).await;

        assert_eq!(status, 200, "status 200");
        let jval: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            jval["users"].as_array().map(|a| a.len()),
            Some(0),
            "empty list"
        );
    }

    // -----------------------------------------------------------------------
    // Seed: user + blocked node
    // -----------------------------------------------------------------------

    async fn seed_user_and_blocked_node(store: &Rc<Store>) -> NodeId {
        let c2 = seed_with_one_child(store).await;
        seed_user(store, "alice@ex", "Alice").await;

        model::logic::access::block(
            UserId::of_email("alice@ex"),
            c2.clone(),
            make_get_user(store.clone()),
            {
                let s = store.clone();
                move |nid| std::future::ready(Ok(s.get_node(&nid)))
            },
            make_put_edge(store.clone()),
        )
        .await
        .expect("block");

        c2
    }

    // -----------------------------------------------------------------------
    // test: list_blocked_nodes returns nodes
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_blocked_nodes_returns_nodes() {
        let store = Rc::new(Store::new());
        let c2 = seed_user_and_blocked_node(&store).await;

        let (status, body) =
            handle_list_blocked_nodes("U#alice@ex", make_list_blocked_nodes_fn(store.clone()))
                .await;

        assert_eq!(status, 200, "status 200");
        let jval: serde_json::Value = serde_json::from_str(&body).unwrap();
        let nodes = jval["nodes"].as_array().expect("nodes array");
        assert_eq!(nodes.len(), 1, "one node");
        assert_eq!(nodes[0].as_str(), Some(c2.to_string().as_str()));
    }

    // -----------------------------------------------------------------------
    // test: list_blocked_users returns users
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_blocked_users_returns_users() {
        let store = Rc::new(Store::new());
        let c2 = seed_user_and_blocked_node(&store).await;

        let (status, body) =
            handle_list_blocked_users(&c2.to_string(), make_list_blocked_users_fn(store.clone()))
                .await;

        assert_eq!(status, 200, "status 200");
        let jval: serde_json::Value = serde_json::from_str(&body).unwrap();
        let users_arr = jval["users"].as_array().expect("users array");
        assert_eq!(users_arr.len(), 1, "one user");
        assert_eq!(users_arr[0].as_str(), Some("U#alice@ex"));
    }

    // -----------------------------------------------------------------------
    // test: effective_permission before/after block
    //
    // Updated to edge-based semantics: capability comes from the user's
    // access edge kind, not the global cognito group.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn effective_permission_before_and_after_block() {
        let store = Rc::new(Store::new());
        let c2 = seed_with_one_child(&store).await;
        let frank_uid = seed_user(&store, "frank@ex", "Frank").await;

        // Grant a Writes edge on c2 so frank has Writer capability there.
        store.put_edge(RepoEdgeSpec {
            from_: frank_uid.to_string(),
            to_: c2.to_string(),
            kind: EdgeKind::Writes,
            name: String::new(),
        });

        // Before block: Writer capability (from the Writes edge).
        let (status, body) = handle_effective_permission(
            "U#frank@ex",
            &c2.to_string(),
            make_get_user(store.clone()),
            {
                let s = store.clone();
                move |nid| std::future::ready(Ok(s.get_node(&nid)))
            },
            make_list_blocked_nodes_fn(store.clone()),
            make_list_access_edges(store.clone()),
        )
        .await;
        assert_eq!(status, 200, "status 200 before block");
        let jval: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            jval["capability"].as_str(),
            Some("Writer"),
            "writer before block"
        );

        // Block frank@ex on c2.
        model::logic::access::block(
            UserId::of_email("frank@ex"),
            c2.clone(),
            make_get_user(store.clone()),
            {
                let s = store.clone();
                move |nid| std::future::ready(Ok(s.get_node(&nid)))
            },
            make_put_edge(store.clone()),
        )
        .await
        .expect("block frank");

        // After block: capability null + reason "blocked".
        let (status2, body2) = handle_effective_permission(
            "U#frank@ex",
            &c2.to_string(),
            make_get_user(store.clone()),
            {
                let s = store.clone();
                move |nid| std::future::ready(Ok(s.get_node(&nid)))
            },
            make_list_blocked_nodes_fn(store.clone()),
            make_list_access_edges(store.clone()),
        )
        .await;
        assert_eq!(status2, 200, "status 200 after block");
        let jval2: serde_json::Value = serde_json::from_str(&body2).unwrap();
        assert!(jval2["capability"].is_null(), "capability null after block");
        assert_eq!(
            jval2["reason"].as_str(),
            Some("blocked"),
            "reason blocked"
        );
    }

    // -----------------------------------------------------------------------
    // test: top_level_shows_administrated_hn2
    //
    // A user administrating an HN2 node → the rendered HTML tree contains
    // "HN2#10002". Mirrors OCaml `top_level_shows_administrated_hn2`.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn top_level_shows_administrated_hn2() {
        let store = Rc::new(Store::new());
        let c2 = NodeId::make(Level::Hn2, 10002);

        let n2 = node::make(
            10002,
            Level::Hn2,
            "Acme",
            NodeId::root(),
            &format!("{}|HN1#10001", NodeId::root()),
            ts(),
            json!({}),
            None,
        );
        store.put_node(&n2);

        // Create user with Admin group.
        let uid = model::logic::users::create(
            "stel@x".to_string(),
            "Stel".to_string(),
            CognitoGroup::Admin,
            None,
            None,
            make_get_user(store.clone()),
            make_put_user(store.clone()),
        )
        .await
        .expect("create user")
        .id;

        // Grant administrates on c2.
        model::logic::access::grant_administrates(
            uid.clone(),
            c2.clone(),
            make_get_user(store.clone()),
            {
                let s = store.clone();
                move |nid| std::future::ready(Ok(s.get_node(&nid)))
            },
            make_put_edge(store.clone()),
        )
        .await
        .expect("grant administrates");

        // Call handle_nodes with the user's id (no id → top-level).
        let uid_s = uid.to_string();
        let (status, html) = handle_nodes(
            None,
            &uid_s,
            None,
            false,
            make_list_administrated(store.clone()),
            make_list_child_refs(store.clone()),
            make_get_node(store.clone()),
        )
        .await;

        assert_eq!(status, 200, "status 200");
        assert!(
            html.contains("HN2#10002"),
            "tree should show the administrated HN2 node, got html snippet: {:?}",
            &html[..html.len().min(500)]
        );
    }

    // -----------------------------------------------------------------------
    // Helpers for handle_node tests
    // -----------------------------------------------------------------------

    fn make_list_blocked_nodes_once(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<Vec<NodeId>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_blocked_nodes(&uid)))
    }

    fn make_list_access_edges_once(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId)
           -> std::future::Ready<Result<Vec<(NodeId, EdgeKind)>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_access_edges(&uid)))
    }

    /// Seed a minimal HN2 node and return its NodeId.
    fn make_simple_hn2(store: &Rc<Store>) -> NodeId {
        let nid = NodeId::make(Level::Hn2, 10003);
        let n = node::make(
            10003,
            Level::Hn2,
            "TestCo",
            NodeId::root(),
            &format!("{}|HN1#10001", NodeId::root()),
            ts(),
            json!({}),
            None,
        );
        store.put_node(&n);
        nid
    }

    /// Grant an edge of a given kind and return the UserId.
    async fn seed_user_with_edge(
        store: &Rc<Store>,
        email: &str,
        node_id: &NodeId,
        kind: EdgeKind,
    ) -> UserId {
        let uid = seed_user(store, email, "Test").await;
        store.put_edge(RepoEdgeSpec {
            from_: uid.to_string(),
            to_: node_id.to_string(),
            kind,
            name: String::new(),
        });
        uid
    }

    // -----------------------------------------------------------------------
    // test: handle_node with Writes edge → Writer capability → no add-child/sensor
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn handle_node_writer_omits_add_child_and_sensor() {
        let store = Rc::new(Store::new());
        let nid = make_simple_hn2(&store);
        let uid = seed_user_with_edge(&store, "writer@ex", &nid, EdgeKind::Writes).await;

        let (status, html) = handle_node(
            &nid.to_string(),
            &uid.to_string(),
            make_get_node(store.clone()),
            // get_user: FnOnce
            {
                let s = store.clone();
                move |u| std::future::ready(Ok(s.get_user(&u)))
            },
            make_list_blocked_nodes_once(store.clone()),
            make_list_access_edges_once(store.clone()),
        )
        .await;

        assert_eq!(status, 200, "status 200");
        assert!(
            !html.contains("add-child-dialog"),
            "Writer should not see add-child-dialog"
        );
        assert!(
            !html.contains("Add child"),
            "Writer should not see Add child button"
        );
        assert!(
            !html.contains("add-sensor-dialog"),
            "Writer should not see add-sensor-dialog"
        );
    }

    // -----------------------------------------------------------------------
    // test: handle_node with Reads edge → Reader capability → no add-child/sensor, readonly
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn handle_node_reader_omits_add_child_and_sensor_and_is_readonly() {
        let store = Rc::new(Store::new());
        let nid = make_simple_hn2(&store);
        let uid = seed_user_with_edge(&store, "reader@ex", &nid, EdgeKind::Reads).await;

        let (status, html) = handle_node(
            &nid.to_string(),
            &uid.to_string(),
            make_get_node(store.clone()),
            {
                let s = store.clone();
                move |u| std::future::ready(Ok(s.get_user(&u)))
            },
            make_list_blocked_nodes_once(store.clone()),
            make_list_access_edges_once(store.clone()),
        )
        .await;

        assert_eq!(status, 200, "status 200");
        assert!(
            !html.contains("add-child-dialog"),
            "Reader should not see add-child-dialog"
        );
        assert!(
            !html.contains("add-sensor-dialog"),
            "Reader should not see add-sensor-dialog"
        );
        // Name input should be readonly for Reader.
        assert!(
            html.contains(r#"id="name" value="TestCo" readonly"#),
            "Reader: name input should be readonly"
        );
    }
}
