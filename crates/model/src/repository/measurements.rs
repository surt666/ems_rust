//! Measurements repository — reads raw meter readings (`all.raw_data`) for the
//! `/measurements` (Datatilegnelse) view.
//!
//! This is the **Athena** adapter: one injectable implementation of "read a page
//! of raw readings for a sensor". The aggregations service injects [`query`] into
//! its HTTP handler, so a test — or a future Redshift adapter — can substitute a
//! different reader without touching the handler. It does the dedup
//! `max_by(value, ingested_time)` + `GROUP BY` server-side and returns only the
//! small page rendered, with **no result reuse** (always the freshest data).
//!
//! Feature-gated (`athena`) so services that never read measurements don't
//! compile the Athena SDK.

use std::time::Duration as StdDuration;

use aws_sdk_athena::types::{QueryExecutionContext, ResultConfiguration};
use aws_sdk_athena::Client as AthenaClient;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use crate::domain::measurement::{Measurement, MeasurementQuery};

const POLL_MS: u64 = 300;
const MAX_POLLS: u32 = 90; // ~27s ceiling, under the 30s lambda timeout

/// Athena workgroup / output / catalog / db / table, from env with defaults.
struct AthenaConfig {
    workgroup: String,
    output: String,
    catalog: String,
    database: String,
    table: String,
}

impl AthenaConfig {
    fn from_env() -> Self {
        let env = |k: &str, d: &str| {
            std::env::var(k)
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| d.into())
        };
        AthenaConfig {
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
}

/// Read the newest raw reading per timestamp for one sensor
/// (`max_by(value, ingested_time)`), newest-first, from Athena over
/// `all.raw_data`. The injectable read adapter for the `/measurements` route.
pub async fn query(athena: &AthenaClient, q: MeasurementQuery) -> anyhow::Result<Vec<Measurement>> {
    let c = AthenaConfig::from_env();
    let from_lit = match q.from.as_deref() {
        Some(s) => ts_literal(s, false)?,
        None => (q.now - Duration::days(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    };
    let to_lit = match q.to.as_deref() {
        Some(s) => ts_literal(s, true)?,
        None => q.now.format("%Y-%m-%d %H:%M:%S").to_string(),
    };
    let before_clause = match q.before.as_deref() {
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
        daq = q.daq_id.replace('\'', "''"),
        from = from_lit,
        to = to_lit,
        before = before_clause,
        limit = q.limit,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_literal_accepts_the_three_formats() {
        assert_eq!(ts_literal("2026-06-12T09:04:31Z", false).unwrap(), "2026-06-12 09:04:31");
        assert_eq!(ts_literal("2026-06-12 09:04:31", false).unwrap(), "2026-06-12 09:04:31");
        assert_eq!(ts_literal("2026-06-12", false).unwrap(), "2026-06-12 00:00:00");
        assert_eq!(ts_literal("2026-06-12", true).unwrap(), "2026-06-12 23:59:59");
    }

    #[test]
    fn norm_ts_drops_sub_second() {
        assert_eq!(norm_ts("2026-06-12 09:04:31.000"), "2026-06-12 09:04:31");
    }
}
