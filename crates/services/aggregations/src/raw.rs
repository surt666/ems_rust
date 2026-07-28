//! `/measurements` (Datatilegnelse) HTTP handler.
//!
//! The data access lives in `model::repository::measurements` (the Athena
//! adapter) and is **injected** as `read` — so this module owns only the HTTP
//! concerns: query-string parsing, `?format` negotiation, the HTMX `<tr>`
//! fragment + keyset cursor, and the error envelope. A test (or a future Redshift
//! adapter) substitutes a different `read` without touching this handler.
//!
//! GET /measurements?daq_id=<id>&from=<date|rfc3339>&to=<...>&limit=<n>&before=<cursor>
//!   - newest-first: the reader returns rows ORDER BY timestamp DESC, LIMIT n.
//!   - keyset "load more": `before` → only timestamps < before; the response emits
//!     an out-of-band `#m-before` input with the next cursor (oldest ts on page).
use std::collections::HashMap;
use std::future::Future;

use chrono::Utc;

use api::{ApiError, ApiResponse, Format};
use model::domain::measurement::{Measurement, MeasurementQuery};

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 500;

/// `GET /meterdata/query/get_measurements?...` — newest raw readings for one sensor.
///
/// `?format=html` (default) returns the `<tr>` fragment the Datatilegnelse page
/// swaps in with HTMX; `?format=json` returns the `[Measurement]` array. Errors
/// follow the format: an HTML `<tr>` for `html`, the `{"error":{…}}` envelope for
/// `json`. `read` is the injected data source (Athena in production).
#[utoipa::path(
    get,
    path = "/meterdata/query/get_measurements",
    tag = "measurements",
    params(
        ("daq_id" = String, Query, description = "Sensor DAQ id (required)"),
        ("from" = Option<String>, Query, description = "Start (RFC3339 / 'YYYY-MM-DD[ HH:MM:SS]'); default now-1d"),
        ("to" = Option<String>, Query, description = "End (same formats); default now"),
        ("limit" = Option<usize>, Query, description = "Max rows 1..=500 (default 100)"),
        ("before" = Option<String>, Query, description = "Keyset cursor: only timestamps < before"),
        ("format" = Option<String>, Query, description = "html (default) | json"),
    ),
    responses(
        (status = 200, description = "Readings as [Measurement] (json) or an HTML <tr> fragment (html)", body = Vec<Measurement>),
        (status = 400, description = "daq_id required", body = api::ErrorResponse),
        (status = 500, description = "Athena query failed", body = api::ErrorResponse),
    ),
)]
pub async fn handle_measurements<R, Fut>(
    read: R,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError>
where
    R: FnOnce(MeasurementQuery) -> Fut,
    Fut: Future<Output = anyhow::Result<Vec<Measurement>>>,
{
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Html);

    let daq = qs.get("daq_id").cloned().unwrap_or_default();
    if daq.is_empty() {
        return err(format, 400, "Bad_request", "daq_id required");
    }
    // Treat empty params (blank date inputs) as absent → the adapter defaults the
    // window to the last day.
    let param = |k: &str| qs.get(k).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let limit = qs
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT);
    let query = MeasurementQuery {
        daq_id: daq,
        from: param("from"),
        to: param("to"),
        before: param("before"),
        limit,
        now: Utc::now(),
    };

    match read(query).await {
        Ok(rows) => Ok(match format {
            Format::Json => ApiResponse::json(&rows),
            Format::Html | Format::Chart => ApiResponse::html(200, render_fragment(&rows, limit)),
        }),
        Err(e) => err(format, 500, "Internal", &e.to_string()),
    }
}

/// Format-aware error: a JSON `{"error":{…}}` envelope for `json`, or an HTML
/// `<tr>` row (so an HTMX swap stays well-formed) for `html`.
fn err(format: Format, status: u16, code: &str, message: &str) -> Result<ApiResponse, ApiError> {
    match format {
        Format::Json => Err(ApiError::new(status, code.to_string(), message.to_string())),
        Format::Html | Format::Chart => Ok(ApiResponse::html(
            status,
            format!("<tr><td colspan=\"4\">Fejl: {}</td></tr>", esc(message)),
        )),
    }
}

fn fmt_date(ts: &str) -> String {
    // "YYYY-MM-DD HH:MM:SS" → "YYYY-MM-DD HH:MM"
    if ts.len() >= 16 {
        ts[..16].to_string()
    } else {
        ts.to_string()
    }
}

/// Rows + an out-of-band `#m-before` cursor (oldest ts on the page, or empty when
/// the page wasn't full → exhausted).
fn render_fragment(rows: &[Measurement], limit: usize) -> String {
    let mut out = String::new();
    if rows.is_empty() {
        out.push_str(
            "<tr><td colspan=\"4\" class=\"muted\">Ingen aflæsninger i perioden.</td></tr>",
        );
    }
    for r in rows {
        out.push_str(&format!(
            "<tr><td class=\"mono\">{}</td><td class=\"mono\" style=\"text-align:right\">{}</td><td>{}</td><td class=\"mono muted\">{}</td></tr>",
            fmt_date(&r.timestamp),
            esc(&r.value),
            esc(&r.unit),
            fmt_date(&r.timestamp),
        ));
    }
    let next_before = if rows.len() == limit {
        rows.last().map(|r| r.timestamp.clone()).unwrap_or_default()
    } else {
        String::new()
    };
    out.push_str(&format!(
        "<input id=\"m-before\" name=\"before\" type=\"hidden\" value=\"{}\" hx-swap-oob=\"true\">",
        esc(&next_before)
    ));
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::ApiBody;

    fn qs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn body_of(resp: &ApiResponse) -> &str {
        match &resp.body {
            ApiBody::Json(s) | ApiBody::Html(s) => s,
        }
    }

    fn sample() -> Vec<Measurement> {
        vec![
            Measurement { timestamp: "2026-06-12 09:04:31".into(), value: "1.500".into(), unit: "kWh".into() },
            Measurement { timestamp: "2026-06-12 08:04:31".into(), value: "1.250".into(), unit: "kWh".into() },
        ]
    }

    /// The handler renders the injected rows — no real Athena needed.
    #[tokio::test]
    async fn injected_reader_renders_html_fragment() {
        let resp = handle_measurements(
            |_q| async { Ok(sample()) },
            &qs(&[("daq_id", "daq:x"), ("format", "html")]),
        )
        .await
        .expect("html ok");
        let body = body_of(&resp);
        assert!(body.contains("2026-06-12 09:04"), "renders a reading: {body}");
        assert!(body.contains("id=\"m-before\""), "emits the keyset cursor");
    }

    /// `?format=json` serializes the injected rows.
    #[tokio::test]
    async fn injected_reader_serializes_json() {
        let resp = handle_measurements(
            |_q| async { Ok(sample()) },
            &qs(&[("daq_id", "daq:x"), ("format", "json")]),
        )
        .await
        .expect("json ok");
        let body = body_of(&resp);
        assert!(body.contains("\"value\":\"1.500\""), "json rows: {body}");
    }

    /// The parsed query reaches the reader (daq id + defaulted limit).
    #[tokio::test]
    async fn passes_query_to_reader() {
        let resp = handle_measurements(
            |q: MeasurementQuery| async move {
                assert_eq!(q.daq_id, "daq:probe");
                assert_eq!(q.limit, 100);
                Ok(vec![])
            },
            &qs(&[("daq_id", "daq:probe")]),
        )
        .await
        .expect("ok");
        assert!(body_of(&resp).contains("Ingen aflæsninger"));
    }

    /// Missing daq_id short-circuits to 400 without calling the reader.
    #[tokio::test]
    async fn missing_daq_id_is_400_before_read() {
        let resp = handle_measurements(
            |_q| async { panic!("reader must not run") },
            &qs(&[("format", "json")]),
        )
        .await;
        assert!(matches!(resp, Err(e) if e.status == 400));
    }
}
