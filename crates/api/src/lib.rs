//! Shared HTTP plumbing for the service lambdas.
//!
//! Handlers return a typed [`ApiResponse`] / [`ApiError`] instead of hand-building
//! a `lambda_http::Response`, and [`to_http`]/[`finish`] are the single place that
//! sets status, `Content-Type` and CORS headers. [`Format`] implements the
//! `?format=html|json` content negotiation, and the JSON side is exactly what
//! `utoipa` documents (the [`ErrorResponse`] envelope is `ToSchema`).

use lambda_http::{Body, Response};
use serde::Serialize;
use utoipa::ToSchema;

// ---------------------------------------------------------------------------
// Content negotiation
// ---------------------------------------------------------------------------

/// Requested representation, parsed from `?format=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Html,
}

impl Format {
    /// Resolve a raw `?format=` value against a route's natural default.
    /// Unknown/absent values fall back to `default`.
    pub fn resolve(raw: Option<&str>, default: Format) -> Format {
        match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("json") => Format::Json,
            Some("html") => Format::Html,
            _ => default,
        }
    }
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

/// The body of a successful response — a JSON document or an HTML fragment.
pub enum ApiBody {
    Json(String),
    Html(String),
}

/// A typed response: an HTTP status plus a JSON-or-HTML body. Build via the
/// constructors so handlers never touch `Response`/headers directly.
pub struct ApiResponse {
    pub status: u16,
    pub body: ApiBody,
}

impl ApiResponse {
    /// `200` with `value` serialized as JSON.
    pub fn json(value: &impl Serialize) -> Self {
        Self::json_status(200, value)
    }

    /// `status` with `value` serialized as JSON.
    pub fn json_status(status: u16, value: &impl Serialize) -> Self {
        let body = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
        Self { status, body: ApiBody::Json(body) }
    }

    /// A response whose JSON body string is already built (e.g. a dispatch envelope).
    pub const fn json_raw(status: u16, body: String) -> Self {
        Self { status, body: ApiBody::Json(body) }
    }

    /// `status` with an HTML-fragment body.
    pub const fn html(status: u16, body: String) -> Self {
        Self { status, body: ApiBody::Html(body) }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A typed error: an HTTP status plus the machine-readable `code`/`message` that
/// become the `{"error":{…}}` wire envelope (see [`ErrorResponse`]).
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
}

/// The error wire shape, `{"error":{"code","message"}}`. Lives here as the single
/// source of truth so the serialized body and the OpenAPI schema can't drift.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ErrorDetail {
    /// Machine-readable code, e.g. `"Bad_request"`.
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { status, code: code.into(), message: message.into() }
    }
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "Bad_request", message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "Not_found", message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, "Internal", message)
    }

    /// Render to the `{"error":{…}}` envelope response.
    pub fn into_response(self) -> ApiResponse {
        let env = ErrorResponse {
            error: ErrorDetail { code: self.code, message: self.message },
        };
        ApiResponse::json_status(self.status, &env)
    }
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// Whether to attach permissive CORS headers. Function-URL lambdas let the URL
/// config add CORS (so we must NOT duplicate it → [`Cors::None`]); API-Gateway /
/// direct lambdas add it here ([`Cors::AllowAll`]).
#[derive(Clone, Copy)]
pub enum Cors {
    None,
    AllowAll,
}

/// Turn a typed [`ApiResponse`] into the `lambda_http` envelope — the single place
/// status, `Content-Type` and CORS headers are set.
pub fn to_http(resp: ApiResponse, cors: Cors) -> Response<Body> {
    let (content_type, body) = match resp.body {
        ApiBody::Json(s) => ("application/json", s),
        ApiBody::Html(s) => ("text/html; charset=utf-8", s),
    };
    let builder = Response::builder()
        .status(resp.status)
        .header("Content-Type", content_type);
    let builder = match cors {
        Cors::AllowAll => builder
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Allow-Headers", "Content-Type,Authorization,X-Requested-With")
            .header("Access-Control-Allow-Methods", "GET,POST,OPTIONS"),
        Cors::None => builder,
    };
    builder
        .body(Body::from(body))
        .expect("failed to build response")
}

/// Resolve a handler's `Result<ApiResponse, ApiError>` into an HTTP response,
/// rendering the error as the `{"error":{…}}` envelope.
pub fn finish(result: Result<ApiResponse, ApiError>, cors: Cors) -> Response<Body> {
    to_http(result.unwrap_or_else(ApiError::into_response), cors)
}

/// A minimal Swagger UI page (assets from the unpkg CDN) that loads the spec at
/// `spec_path`, resolved **relative to the docs page** — so `"openapi.json"` works
/// whether the page is served at `/docs` (→ `/openapi.json`) or, behind a path
/// prefix, at `/hierarchy/docs` (→ `/hierarchy/openapi.json`).
pub fn swagger_ui_html(spec_path: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>API docs</title>
<link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
</head>
<body>
<div id="swagger-ui"></div>
<script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js" crossorigin></script>
<script>
window.ui = SwaggerUIBundle({{ url: '{spec_path}', dom_id: '#swagger-ui', deepLinking: true }});
</script>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_resolves_with_default() {
        assert_eq!(Format::resolve(Some("json"), Format::Html), Format::Json);
        assert_eq!(Format::resolve(Some("HTML"), Format::Json), Format::Html);
        assert_eq!(Format::resolve(Some("  json "), Format::Html), Format::Json);
        assert_eq!(Format::resolve(None, Format::Html), Format::Html);
        assert_eq!(Format::resolve(Some("xml"), Format::Json), Format::Json);
    }

    #[test]
    fn error_envelope_shape() {
        let resp = ApiError::bad_request("nope").into_response();
        assert_eq!(resp.status, 400);
        match resp.body {
            ApiBody::Json(s) => {
                assert_eq!(s, r#"{"error":{"code":"Bad_request","message":"nope"}}"#);
            }
            _ => panic!("expected json"),
        }
    }
}
