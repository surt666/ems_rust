//! OpenAPI 3.1 document for the JSON surface of the hierarchy lambda.
//!
//! The write API is the typed [`Command`] union (POST /command); the read API is
//! the `/query/{action}` family — JSON actions return JSON, HTML actions return an
//! HTMX fragment, so they're documented as one parameterised path. The two
//! `*_doc` stubs exist only to carry `#[utoipa::path]` annotations.

use utoipa::OpenApi;

use crate::command::Command;

/// `POST /command` (or `/hierarchy/command`) — execute a write command. The body
/// is the `Command` union, serde-tagged on `action`. Accepts JSON or
/// `application/x-www-form-urlencoded` (the GUI posts forms).
#[utoipa::path(
    post,
    path = "/command",
    tag = "command",
    request_body = Command,
    responses(
        (status = 200, description = "Command applied; body shape depends on the action"),
        (status = 400, description = "Bad request / validation", body = api::ErrorResponse),
        (status = 404, description = "Not found", body = api::ErrorResponse),
        (status = 409, description = "Conflict", body = api::ErrorResponse),
    ),
)]
#[allow(dead_code)]
fn command_doc() {}

/// `GET /query/{action}` (or `/hierarchy/query/{action}`) — read queries. JSON
/// actions (`get_node`, `list_children`, `list_users`, `effective_permission`, …)
/// return `application/json`; HTML actions (`nodes`, `node`, `sensors`, `users`,
/// `add_child_form`, …) return a `text/html` fragment for HTMX. Action-specific
/// parameters (e.g. `id`, `parent`, `user`) are passed as query string.
#[utoipa::path(
    get,
    path = "/query/{action}",
    tag = "query",
    params(
        ("action" = String, Path, description = "Query name, e.g. get_node | list_children | nodes | node | sensors"),
    ),
    responses(
        (status = 200, description = "Query result (application/json or text/html, by action)"),
        (status = 400, description = "Unknown action / bad params", body = api::ErrorResponse),
    ),
)]
#[allow(dead_code)]
fn query_doc() {}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "EMS Hierarchy API",
        description = "Hierarchy/topology read queries (/query/{action}, JSON or HTMX fragments) and \
                       write commands (POST /command, the Command union).",
        version = "0.1.0",
    ),
    paths(command_doc, query_doc),
    components(schemas(Command, api::ErrorResponse, api::ErrorDetail)),
    tags(
        (name = "command", description = "Hierarchy write commands"),
        (name = "query", description = "Hierarchy read queries"),
    ),
)]
pub struct ApiDoc;

/// The serialized OpenAPI document (served at `GET /openapi.json`).
pub fn openapi_json() -> String {
    ApiDoc::openapi()
        .to_json()
        .unwrap_or_else(|_| "{}".to_string())
}

/// Pretty-printed OpenAPI document (for the `--openapi` CLI dump).
pub fn openapi_pretty() -> String {
    ApiDoc::openapi()
        .to_pretty_json()
        .unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_doc_generates() {
        let doc = openapi_json();
        assert!(doc.contains("/command"), "missing /command path");
        assert!(doc.contains("/query/{action}"), "missing /query path");
        assert!(doc.contains("Command"), "missing Command schema");
        assert!(doc.contains("attach_sensor"), "Command variants should be enumerated");
    }
}
