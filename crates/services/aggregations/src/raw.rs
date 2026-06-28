//! `/measurements` route of the aggregations lambda (one lambda — limits the
//! Datadog-instrumented function count). Reads the live `all.raw_data` table via
//! **Amazon Athena** (the iceberg-rust direct path was ~12s; Athena is ~3.5s — it
//! does the dedup `GROUP BY` server-side and returns only the small page we render).
//! Returns an **HTML fragment** of `<tr>` rows for the /measurements (Datatilegnelse)
//! page to swap in with HTMX.
//!
//! GET /measurements?daq_id=<id>&from=<date|rfc3339>&to=<...>&limit=<n>&before=<cursor>
//!   - newest version per timestamp: `max_by(value, ingested_time)` (Athena, server-side).
//!   - newest-first: ORDER BY timestamp DESC, then LIMIT.
//!   - keyset "load more": `before` adds `timestamp < before`; the response emits an
//!     out-of-band `#m-before` input with the next cursor (oldest ts on the page).
use std::collections::HashMap;
use std::time::Duration as StdDuration;

use aws_sdk_athena::types::{QueryExecutionContext, ResultConfiguration};
use aws_sdk_athena::Client as AthenaClient;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use api::{ApiError, ApiResponse, Format};
use serde::Serialize;
use utoipa::ToSchema;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 500;
const POLL_MS: u64 = 300;
const MAX_POLLS: u32 = 90; // ~27s ceiling, under the 30s lambda timeout

struct Cfg {
    workgroup: String,
    output: String,
    catalog: String,
    database: String,
    table: String,
}

fn cfg() -> Cfg {
    let env = |k: &str, d: &str| {
        std::env::var(k)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| d.into())
    };
    Cfg {
        workgroup: env("ATHENA_WORKGROUP", "daq-workgroup"),
        output: env(
            "ATHENA_OUTPUT",
            "s3://daq-athena-query-results-891377204778-eu-central-1/",
        ),
        catalog: env("ATHENA_CATALOG", "s3tablescatalog/measurements"),
        database: env("ATHENA_DATABASE", "all"),
        table: env("ATHENA_TABLE", "raw_data"),
    }
}

/// One raw meter reading: the newest version (`max_by(value, ingested_time)`)
/// for a given timestamp. The JSON shape of the `/measurements` route.
#[derive(Debug, Serialize, ToSchema)]
pub struct Measurement {
    /// Reading timestamp, `"YYYY-MM-DD HH:MM:SS"` (UTC, from Athena).
    pub timestamp: String,
    /// Formatted reading value.
    pub value: String,
    pub unit: String,
}

/// `GET /meterdata/query/get_measurements?...` — newest raw readings for one sensor.
///
/// `?format=html` (default) returns the `<tr>` fragment the Datatilegnelse page
/// swaps in with HTMX; `?format=json` returns the `[Measurement]` array. Errors
/// follow the format: an HTML `<tr>` for `html`, the `{"error":{…}}` envelope for
/// `json`.
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
pub async fn handle_measurements(
    athena: &AthenaClient,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Html);

    let daq = qs.get("daq_id").cloned().unwrap_or_default();
    if daq.is_empty() {
        return err(format, 400, "Bad_request", "daq_id required");
    }
    // Treat empty params (blank date inputs) as absent → default to last 1 day.
    let now = Utc::now();
    let param = |k: &str| qs.get(k).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let to = param("to");
    let from = param("from");
    let before = param("before");
    let limit = qs
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT);

    match query_rows(
        athena,
        &daq,
        from.as_deref(),
        to.as_deref(),
        before.as_deref(),
        limit,
        now,
    )
    .await
    {
        Ok(rows) => Ok(match format {
            Format::Json => ApiResponse::json(&rows),
            Format::Html => ApiResponse::html(200, render_fragment(&rows, limit)),
        }),
        Err(e) => err(format, 500, "Internal", &e.to_string()),
    }
}

/// Format-aware error: a JSON `{"error":{…}}` envelope for `json`, or an HTML
/// `<tr>` row (so an HTMX swap stays well-formed) for `html`.
fn err(format: Format, status: u16, code: &str, message: &str) -> Result<ApiResponse, ApiError> {
    match format {
        Format::Json => Err(ApiError::new(status, code.to_string(), message.to_string())),
        Format::Html => Ok(ApiResponse::html(
            status,
            format!("<tr><td colspan=\"4\">Fejl: {}</td></tr>", esc(message)),
        )),
    }
}

async fn query_rows(
    athena: &AthenaClient,
    daq: &str,
    from: Option<&str>,
    to: Option<&str>,
    before: Option<&str>,
    limit: usize,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<Measurement>> {
    let c = cfg();
    let from_lit = match from {
        Some(s) => ts_literal(s, false)?,
        None => (now - Duration::days(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    };
    let to_lit = match to {
        Some(s) => ts_literal(s, true)?,
        None => now.format("%Y-%m-%d %H:%M:%S").to_string(),
    };
    let before_clause = match before {
        Some(s) => format!(" AND timestamp < timestamp '{}'", ts_literal(s, false)?),
        None => String::new(),
    };
    // daq_id inlined with single-quote escaping (Presto string literal); timestamps
    // are reformatted from parsed datetimes and the limit is a validated int — all
    // injection-safe.
    let sql = format!(
        "SELECT timestamp, max_by(value, ingested_time) AS value, max_by(unit, ingested_time) AS unit \
         FROM \"{db}\".\"{tbl}\" \
         WHERE daq_id = '{daq}' \
           AND timestamp >= timestamp '{from}' AND timestamp <= timestamp '{to}'{before} \
         GROUP BY daq_id, timestamp ORDER BY timestamp DESC LIMIT {limit}",
        db = c.database,
        tbl = c.table,
        daq = daq.replace('\'', "''"),
        from = from_lit,
        to = to_lit,
        before = before_clause,
        limit = limit,
    );

    // Start → poll → fetch. No result-reuse — always read the freshest raw_data.
    let start = athena
        .start_query_execution()
        .query_string(sql)
        .work_group(&c.workgroup)
        .query_execution_context(
            QueryExecutionContext::builder()
                .database(&c.database)
                .catalog(&c.catalog)
                .build(),
        )
        .result_configuration(
            ResultConfiguration::builder()
                .output_location(&c.output)
                .build(),
        )
        // No result-reuse cache: this is a live raw-data viewer, so every load
        // runs fresh against raw_data (the latest readings always show). Trade-off
        // is the full Athena latency (~7-10s) on every load.
        .send()
        .await?;
    let qid = start
        .query_execution_id()
        .ok_or_else(|| anyhow::anyhow!("no query execution id"))?
        .to_string();

    let mut polls = 0;
    loop {
        let ge = athena
            .get_query_execution()
            .query_execution_id(&qid)
            .send()
            .await?;
        let status = ge.query_execution().and_then(|q| q.status());
        let state = status
            .and_then(|s| s.state())
            .map(|s| s.as_str().to_string());
        match state.as_deref() {
            Some("SUCCEEDED") => break,
            Some("FAILED") | Some("CANCELLED") => {
                let reason = status.and_then(|s| s.state_change_reason()).unwrap_or("");
                return Err(anyhow::anyhow!(
                    "athena {}: {}",
                    state.unwrap_or_default(),
                    reason
                ));
            }
            _ => {
                polls += 1;
                if polls >= MAX_POLLS {
                    return Err(anyhow::anyhow!(
                        "athena query timed out after {} polls",
                        polls
                    ));
                }
                tokio::time::sleep(StdDuration::from_millis(POLL_MS)).await;
            }
        }
    }

    let results = athena
        .get_query_results()
        .query_execution_id(&qid)
        .send()
        .await?;
    let mut rows = Vec::new();
    // First row is the column header → skip it.
    for r in results
        .result_set()
        .map(|rs| rs.rows())
        .unwrap_or_default()
        .iter()
        .skip(1)
    {
        let cols = r.data();
        let cell = |i: usize| {
            cols.get(i)
                .and_then(|d| d.var_char_value())
                .unwrap_or_default()
        };
        let raw_ts = cell(0);
        let value = cell(1)
            .parse::<f64>()
            .map(|v| format!("{:.3}", v))
            .unwrap_or_else(|_| cell(1).to_string());
        rows.push(Measurement {
            timestamp: norm_ts(raw_ts),
            value,
            unit: cell(2).to_string(),
        });
    }
    Ok(rows)
}

/// Validate + reformat a from/to/cursor value into a Presto `timestamp` literal
/// body. Accepts RFC3339, `YYYY-MM-DD HH:MM:SS[.fff]`, or date-only `YYYY-MM-DD`.
fn ts_literal(s: &str, end_of_day: bool) -> anyhow::Result<String> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt
            .with_timezone(&Utc)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string());
    }
    for f in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, f) {
            return Ok(ndt.format("%Y-%m-%d %H:%M:%S").to_string());
        }
    }
    let d = NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")?;
    let t = if end_of_day {
        NaiveTime::from_hms_opt(23, 59, 59)
    } else {
        NaiveTime::from_hms_opt(0, 0, 0)
    }
    .expect("valid time");
    Ok(d.and_time(t).format("%Y-%m-%d %H:%M:%S").to_string())
}

/// Athena returns e.g. "2026-06-12 09:04:31.000"; normalize to seconds precision.
fn norm_ts(s: &str) -> String {
    s.split('.').next().unwrap_or(s).trim().to_string()
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
