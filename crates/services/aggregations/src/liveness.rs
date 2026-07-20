//! Device-liveness read: "is this device sending data?" — reads the `all.heartbeat`
//! Iceberg table (S3 Tables), written append-only by Firehose from the meter-heartbeat
//! lambda. Aggregates to latest-per-device (`max_by(_, last_seen)` + `GROUP BY`), keyed
//! by customerid (+ optional meterid). Serves the Raw Device MFE: HTML fragment or JSON.

use std::collections::HashMap;
use std::time::Duration as StdDuration;

use aws_sdk_athena::types::{QueryExecutionContext, ResultConfiguration};
use aws_sdk_athena::Client as AthenaClient;
use serde::Serialize;

use api::{ApiError, ApiResponse, Format};

const POLL_MS: u64 = 300;
const MAX_POLLS: u32 = 90; // ~27s ceiling, under the 30s lambda timeout

/// One device's latest-seen state (all cells come back from Athena as strings).
#[derive(Debug, Default, Serialize)]
pub struct LivenessRow {
    pub customerid: String,
    pub meterid: String,
    pub last_seen: String,
    pub event_time: String,
    pub gatewayid: String,
    pub schematype: String,
    pub transport: String,
}

/// `GET …/query/get_liveness?customerid=&meterid=&format=html|json`. `customerid` is
/// required (the read filter); `meterid` narrows to one device. Values are single-quote
/// escaped for the Presto literal (injection-safe).
pub async fn handle_liveness(
    athena: &AthenaClient,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Html);

    let clean = |k: &str| qs.get(k).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let Some(customerid) = clean("customerid") else {
        return Err(ApiError::bad_request("customerid is required"));
    };
    let meterid = clean("meterid");

    let esc = |s: &str| s.replace('\'', "''");
    let meter_clause = match &meterid {
        Some(m) => format!(" AND meterid = '{}'", esc(m)),
        None => String::new(),
    };
    // Aggregate the append log to the latest row per device.
    let sql = format!(
        "SELECT customerid, meterid, \
                CAST(max(last_seen) AS VARCHAR) AS last_seen, \
                max_by(event_time, last_seen) AS event_time, \
                max_by(gatewayid, last_seen)  AS gatewayid, \
                max_by(schematype, last_seen) AS schematype, \
                max_by(transport, last_seen)  AS transport \
         FROM \"all\".\"heartbeat\" \
         WHERE customerid = '{cust}'{meter} \
         GROUP BY customerid, meterid ORDER BY 3 DESC LIMIT 500",
        cust = esc(&customerid),
        meter = meter_clause,
    );

    let rows = run_query(athena, &sql).await?;

    Ok(match format {
        Format::Json => ApiResponse::json(&rows),
        Format::Html => ApiResponse::html(200, render_fragment(&customerid, meterid.as_deref(), &rows)),
    })
}

/// Athena config from env (shared with the measurements route), table fixed to `heartbeat`.
fn athena_env(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| d.into())
}

/// Start → poll → fetch an Athena query, returning `LivenessRow`s (header skipped).
async fn run_query(athena: &AthenaClient, sql: &str) -> Result<Vec<LivenessRow>, ApiError> {
    let workgroup = athena_env("ATHENA_WORKGROUP", "daq-workgroup");
    let output = athena_env("ATHENA_OUTPUT", "s3://daq-athena-query-results-891377204778-eu-central-1/");
    let catalog = athena_env("ATHENA_CATALOG", "s3tablescatalog/measurements");
    let database = athena_env("ATHENA_DATABASE", "all");

    let start = athena
        .start_query_execution()
        .query_string(sql)
        .work_group(&workgroup)
        .query_execution_context(QueryExecutionContext::builder().database(&database).catalog(&catalog).build())
        .result_configuration(ResultConfiguration::builder().output_location(&output).build())
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("athena start: {e}")))?;
    let qid = start
        .query_execution_id()
        .ok_or_else(|| ApiError::internal("no athena query id"))?
        .to_string();

    let mut polls = 0;
    loop {
        let ge = athena
            .get_query_execution()
            .query_execution_id(&qid)
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("athena poll: {e}")))?;
        let status = ge.query_execution().and_then(|q| q.status());
        match status.and_then(|s| s.state()).map(|s| s.as_str()) {
            Some("SUCCEEDED") => break,
            Some("FAILED") | Some("CANCELLED") => {
                let reason = status.and_then(|s| s.state_change_reason()).unwrap_or("");
                return Err(ApiError::internal(format!("athena failed: {reason}")));
            }
            _ => {
                polls += 1;
                if polls >= MAX_POLLS {
                    return Err(ApiError::internal("athena query timed out"));
                }
                tokio::time::sleep(StdDuration::from_millis(POLL_MS)).await;
            }
        }
    }

    let results = athena
        .get_query_results()
        .query_execution_id(&qid)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("athena results: {e}")))?;
    let cell = |r: &aws_sdk_athena::types::Row, i: usize| {
        r.data().get(i).and_then(|d| d.var_char_value()).unwrap_or_default().to_string()
    };
    let rows = results
        .result_set()
        .map(|rs| rs.rows())
        .unwrap_or_default()
        .iter()
        .skip(1) // header
        .map(|r| LivenessRow {
            customerid: cell(r, 0),
            meterid: cell(r, 1),
            last_seen: cell(r, 2),
            event_time: cell(r, 3),
            gatewayid: cell(r, 4),
            schematype: cell(r, 5),
            transport: cell(r, 6),
        })
        .collect();
    Ok(rows)
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The `#rd-result` fragment the Raw Device MFE swaps in.
fn render_fragment(customerid: &str, meterid: Option<&str>, rows: &[LivenessRow]) -> String {
    let target = match meterid {
        Some(m) => format!("meter <b>{}</b> (customer <b>{}</b>)", esc_html(m), esc_html(customerid)),
        None => format!("customer <b>{}</b>", esc_html(customerid)),
    };
    let mut out = String::from("<div id=\"rd-result\">");
    if rows.is_empty() {
        out.push_str(&format!("<p>\u{26d4} No data — {} has not reported.</p>", target));
        out.push_str("</div>");
        return out;
    }
    let last = rows.iter().map(|r| r.last_seen.as_str()).max().unwrap_or("");
    out.push_str(&format!(
        "<p>\u{2705} {} is reporting — {} meter(s), last seen <b>{}</b>.</p>",
        target,
        rows.len(),
        esc_html(last),
    ));
    out.push_str("<table><thead><tr><th>Meter</th><th>Gateway</th><th>Type</th><th>Transport</th><th>Last seen</th></tr></thead><tbody>");
    for r in rows {
        let c = |s: &str| if s.is_empty() { "—".to_string() } else { esc_html(s) };
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            c(&r.meterid), c(&r.gatewayid), c(&r.schematype), c(&r.transport), c(&r.last_seen),
        ));
    }
    out.push_str("</tbody></table></div>");
    out
}
