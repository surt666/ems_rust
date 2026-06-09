mod command;
mod dispatch;
mod html;
mod json;
mod query;

use lambda_http::{http::Method, run, service_fn, Body, Error, Request, Response};

// ---------------------------------------------------------------------------
// Lambda HTTP handler — mirrors handler.ml routing
//
// Routes:
//   GET  /query/<action>          → query::run_query (JSON)
//   GET  /hierarchy/query/<action> → query::run_query (HTML)
//   POST /command                 → dispatch::run (command)
//   POST /hierarchy/command       → dispatch::run (command)
//   _                             → 400 Bad_request
// ---------------------------------------------------------------------------

async fn handler(event: Request) -> Result<Response<Body>, Error> {
    let method = event.method().clone();
    let path = event.uri().path().to_string();

    // Extract query-string params into a Vec<(String, String)>.
    let params: Vec<(String, String)> = event
        .uri()
        .query()
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
        .unwrap_or_default();

    // ---------------------------------------------------------------------------
    // Routing — matches handler.ml `handler` match
    // ---------------------------------------------------------------------------

    fn strip(prefix: &str, path: &str) -> Option<String> {
        path.strip_prefix(prefix).map(|s| s.to_string())
    }

    let (status, body, content_type) = match method {
        Method::GET => {
            if let Some(action) = strip("/query/", &path)
                .or_else(|| strip("/hierarchy/query/", &path))
            {
                let (s, b) = query::run_query(&action, &params).await;
                // Determine content-type: HTML actions vs JSON actions.
                let ct = if is_html_action(&action) {
                    "text/html; charset=utf-8"
                } else {
                    "application/json"
                };
                (s, b, ct)
            } else {
                (
                    400u16,
                    r#"{"error":{"code":"Bad_request","message":"no matching route"}}"#
                        .to_string(),
                    "application/json",
                )
            }
        }

        Method::POST
            if path == "/command" || path == "/hierarchy/command" =>
        {
            // Parse the raw body as a command.
            let raw_body = body_string(&event);
            let ct_header = event
                .headers()
                .get("content-type")
                .or_else(|| event.headers().get("Content-Type"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_lowercase();

            let ct_opt: Option<&str> = if ct_header.is_empty() {
                None
            } else {
                Some(&ct_header)
            };

            match command::parse_command(ct_opt, &raw_body) {
                Ok(cmd) => {
                    let resp = dispatch::run(cmd).await;
                    // dispatch::run returns a serde_json::Value with statusCode +
                    // body fields (mirrors OCaml Lambda V2 envelope).
                    let status = resp["statusCode"]
                        .as_u64()
                        .unwrap_or(200) as u16;
                    let b = resp["body"]
                        .as_str()
                        .unwrap_or("{}")
                        .to_string();
                    (status, b, "application/json")
                }
                Err(e) => (
                    400u16,
                    format!(
                        r#"{{"error":{{"code":"Bad_request","message":"{}"}}}}"#,
                        e.to_string().replace('"', "\\\"")
                    ),
                    "application/json",
                ),
            }
        }

        _ => (
            400u16,
            r#"{"error":{"code":"Bad_request","message":"no matching route"}}"#.to_string(),
            "application/json",
        ),
    };

    // Build the HTTP response with CORS headers.
    let response = Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header(
            "Access-Control-Allow-Headers",
            "Content-Type,Authorization,X-Requested-With",
        )
        .header(
            "Access-Control-Allow-Methods",
            "GET,POST,OPTIONS",
        )
        .body(Body::from(body))?;

    Ok(response)
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
    run(service_fn(handler)).await
}
