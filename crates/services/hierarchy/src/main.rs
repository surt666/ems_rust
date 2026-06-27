mod command;
mod dispatch;
mod html;
mod json;
mod openapi;
mod query;

use lambda_http::{http::Method, run, service_fn, Body, Error, Request, Response};

use api::{ApiError, ApiResponse, Cors};

// ---------------------------------------------------------------------------
// Lambda HTTP handler — routing
//
// Routes:
//   GET  /query/<action>          → query::run_query (JSON)
//   GET  /hierarchy/query/<action> → query::run_query (HTML)
//   POST /command                 → dispatch::run (command)
//   POST /hierarchy/command       → dispatch::run (command)
//   GET  /openapi.json            → the OpenAPI 3.1 spec
//   GET  /docs                    → Swagger UI rendering of the spec
//   _                             → 400 Bad_request
// ---------------------------------------------------------------------------

async fn handler(event: Request) -> Result<Response<Body>, Error> {
    let params = parse_query_params(event.uri().query());

    // One match, one arm per route, each producing a typed ApiResponse. The query
    // action comes from the URL path (json vs html by `is_html_action`); the
    // command action comes from the request body (resolved in run_command).
    let resp: ApiResponse = match resolve_route(event.method(), event.uri().path()) {
        Route::Query(action) => {
            let (status, body) = query::run_query(&action, &params).await;
            if is_html_action(&action) {
                ApiResponse::html(status, body)
            } else {
                ApiResponse::json_raw(status, body)
            }
        }
        Route::Command => run_command(&event).await,
        Route::OpenApi => ApiResponse::json_raw(200, openapi::openapi_json()),
        Route::Docs => ApiResponse::html(200, api::swagger_ui_html("openapi.json")),
        // Preserve the prior contract: unmatched route → 400 Bad_request.
        Route::NotFound => ApiError::bad_request("no matching route").into_response(),
    };

    // Single transport step: status + Content-Type + permissive CORS headers.
    Ok(api::to_http(resp, Cors::AllowAll))
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

/// A resolved route. `Query` carries the action parsed from the URL path; for
/// `Command` the action lives in the request body (the serde `"action"` tag) and
/// is matched later by `dispatch::run`.
enum Route {
    Query(String),
    Command,
    OpenApi,
    Docs,
    NotFound,
}

/// Map `(method, path)` to a route:
///   GET  `/query/<action>`  | `/hierarchy/query/<action>`  → `Query(action)`
///   POST `/command`         | `/hierarchy/command`         → `Command`
///   GET  `/openapi.json`    | `/hierarchy/openapi.json`    → `OpenApi`
///   _                                                      → `NotFound`
fn resolve_route(method: &Method, path: &str) -> Route {
    match *method {
        Method::GET if matches!(path, "/openapi.json" | "/hierarchy/openapi.json") => {
            Route::OpenApi
        }
        Method::GET if matches!(path, "/docs" | "/hierarchy/docs") => Route::Docs,
        Method::GET => match path
            .strip_prefix("/hierarchy/query/")
            .or_else(|| path.strip_prefix("/query/"))
        {
            Some(action) if !action.is_empty() => Route::Query(action.to_string()),
            _ => Route::NotFound,
        },
        Method::POST if matches!(path, "/command" | "/hierarchy/command") => Route::Command,
        _ => Route::NotFound,
    }
}

/// Parse the URL query string into decoded `(key, value)` pairs (empty keys skipped).
fn parse_query_params(query: Option<&str>) -> Vec<(String, String)> {
    query
        .map(|q| {
            q.split('&')
                .filter_map(|pair| {
                    let mut it = pair.splitn(2, '=');
                    let k = it.next()?;
                    if k.is_empty() {
                        return None;
                    }
                    let v = it.next().unwrap_or("");
                    Some((url_decode(k), url_decode(v)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse the request body as a command and dispatch it. The command's `"action"`
/// tag selects the concrete handler inside `dispatch::run`.
async fn run_command(event: &Request) -> ApiResponse {
    let raw_body = body_string(event);
    let ct_header = event
        .headers()
        .get("content-type")
        .or_else(|| event.headers().get("Content-Type"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let ct_opt = (!ct_header.is_empty()).then_some(ct_header.as_str());

    match command::parse_command(ct_opt, &raw_body) {
        Ok(cmd) => {
            // dispatch::run returns the Lambda V2 envelope { statusCode, body } —
            // body is an already-serialized JSON string.
            let resp = dispatch::run(cmd).await;
            let status = resp["statusCode"].as_u64().unwrap_or(200) as u16;
            let body = resp["body"].as_str().unwrap_or("{}").to_string();
            ApiResponse::json_raw(status, body)
        }
        Err(e) => ApiError::bad_request(e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// HTML query actions (vs JSON ones).
fn is_html_action(action: &str) -> bool {
    matches!(
        action,
        "nodes"
            | "node"
            | "sensors"
            | "company_sensors"
            | "users"
            | "add_child_form"
            | "profiles"
            | "languages"
            | "currencies"
            | "permissions"
            | "timezones"
    )
}

/// Extract request body as a String (handles both text and base64-encoded).
fn body_string(req: &Request) -> String {
    match req.body() {
        Body::Text(s) => s.clone(),
        Body::Binary(b) => {
            // Try UTF-8; fall back to empty string.
            String::from_utf8(b.clone()).unwrap_or_default()
        }
        Body::Empty => String::new(),
    }
}

/// Percent-decode a URL-encoded string (+ → space, %XX → char).
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut result = Vec::with_capacity(len);
    let mut i = 0;
    while i < len {
        match bytes[i] {
            b'+' => {
                result.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < len => {
                if let Some(byte) = decode_hex(bytes[i + 1], bytes[i + 2]) {
                    result.push(byte);
                    i += 3;
                } else {
                    result.push(b'%');
                    i += 1;
                }
            }
            c => {
                result.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(result).unwrap_or_default()
}

fn decode_hex(hi: u8, lo: u8) -> Option<u8> {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    Some(hex(hi)? << 4 | hex(lo)?)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Error> {
    // `cargo run -p hierarchy -- --openapi` prints the spec and exits (the Lambda
    // runtime never passes args, so this is inert in production).
    if std::env::args().any(|a| a == "--openapi") {
        println!("{}", openapi::openapi_pretty());
        return Ok(());
    }

    run(service_fn(handler)).await
}
