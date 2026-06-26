//! `/measurements` route of the aggregations lambda (kept to one Lambda to limit
//! Datadog-instrumented function count). Reads the live `all.raw_data` Apache
//! Iceberg table (S3 Tables) **directly** via the iceberg-rust S3Tables catalog
//! (no Athena) and returns an **HTML fragment** of `<tr>` rows for the
//! /measurements (Datatilegnelse) page to swap in with HTMX.
//!
//! GET /measurements?daq_id=<id>&from=<rfc3339>&to=<rfc3339>&limit=<n>&before=<cursor>
//!   - newest version of each reading: dedup by `timestamp`, keep max `ingested_time`.
//!   - newest-first: sort by timestamp desc, then LIMIT.
//!   - keyset "load more": `before` narrows to timestamp < before; the response emits
//!     an out-of-band `#m-before` input holding the next cursor (oldest ts on the page).
use std::collections::HashMap;

use arrow_array::{Array, Float64Array, StringArray, TimestampMicrosecondArray};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use iceberg::expr::Reference;
use iceberg::spec::Datum;
use iceberg::{Catalog, CatalogBuilder, TableIdent};
use iceberg_catalog_s3tables::{S3TablesCatalogBuilder, S3TABLES_CATALOG_PROP_TABLE_BUCKET_ARN};
use lambda_http::{Body, Error, Response};

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 500;

struct Row {
    ts: i64,       // micros since epoch (UTC)
    value: f64,
    unit: String,
    ingested: i64, // micros since epoch (UTC)
}

/// Handle `GET /measurements?...`. Always returns text/html (a fragment).
pub async fn handle_measurements(qs: &HashMap<String, String>) -> Result<Response<Body>, Error> {
    let daq = qs.get("daq_id").cloned().unwrap_or_default();
    if daq.is_empty() {
        return html(400, "<tr><td colspan=\"4\">daq_id required</td></tr>".into());
    }
    // Treat empty params (blank date inputs) as absent → sensible defaults.
    let now = Utc::now();
    let to = qs
        .get("to")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| now.to_rfc3339());
    let from = qs
        .get("from")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| (now - Duration::days(30)).to_rfc3339());
    let before = qs
        .get("before")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let limit = qs
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT);

    match query_rows(&daq, &from, &to, before.as_deref(), limit).await {
        Ok(rows) => html(200, render_fragment(&rows, limit)),
        Err(e) => html(
            500,
            format!("<tr><td colspan=\"4\">Fejl: {}</td></tr>", esc(&e.to_string())),
        ),
    }
}

async fn query_rows(
    daq: &str,
    from: &str,
    to: &str,
    before: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<Row>> {
    let table_bucket_arn = std::env::var("TABLE_BUCKET_ARN")
        .unwrap_or_else(|_| "arn:aws:s3tables:eu-central-1:891377204778:bucket/measurements".into());
    let endpoint = std::env::var("S3TABLES_ENDPOINT").ok().filter(|s| !s.is_empty());
    let namespace = std::env::var("ICEBERG_NAMESPACE").unwrap_or_else(|_| "all".into());
    let table_name = std::env::var("ICEBERG_TABLE").unwrap_or_else(|_| "raw_data".into());

    let mut props = HashMap::new();
    props.insert(
        S3TABLES_CATALOG_PROP_TABLE_BUCKET_ARN.to_string(),
        table_bucket_arn,
    );
    let mut builder = S3TablesCatalogBuilder::default();
    if let Some(ep) = &endpoint {
        builder = builder.with_endpoint_url(ep);
    }
    let catalog = builder.load("s3tables", props).await?;
    let ident = TableIdent::from_strs([namespace.as_str(), table_name.as_str()])?;
    let table = catalog.load_table(&ident).await?;

    // daq_id == :daq AND from <= timestamp < to [ AND timestamp < before ]
    let from_dt = parse_flexible(from, false)?;
    let to_dt = parse_flexible(to, true)?;
    let mut pred = Reference::new("daq_id")
        .equal_to(Datum::string(daq))
        .and(Reference::new("timestamp").greater_than_or_equal_to(Datum::timestamptz_from_datetime(from_dt)))
        .and(Reference::new("timestamp").less_than(Datum::timestamptz_from_datetime(to_dt)));
    if let Some(b) = before {
        let b_dt = parse_flexible(b, false)?;
        pred = pred.and(Reference::new("timestamp").less_than(Datum::timestamptz_from_datetime(b_dt)));
    }

    let scan = table
        .scan()
        .select(["timestamp", "value", "unit", "ingested_time"])
        .with_filter(pred)
        .build()?;
    let mut stream = scan.to_arrow().await?;

    // Dedup by timestamp keeping the row with the greatest ingested_time.
    let mut newest: HashMap<i64, (f64, String, i64)> = HashMap::new();
    while let Some(batch) = stream.try_next().await? {
        let ts = col_ts(&batch, "timestamp")?;
        let ing = col_ts(&batch, "ingested_time")?;
        let val = batch
            .column_by_name("value")
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
            .ok_or_else(|| anyhow::anyhow!("value column not Float64"))?;
        let unit = batch
            .column_by_name("unit")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .ok_or_else(|| anyhow::anyhow!("unit column not Utf8"))?;
        for i in 0..batch.num_rows() {
            let t = ts.value(i);
            let g = ing.value(i);
            let e = newest.entry(t).or_insert((0.0, String::new(), i64::MIN));
            if g >= e.2 {
                *e = (val.value(i), unit.value(i).to_string(), g);
            }
        }
    }

    let mut rows: Vec<Row> = newest
        .into_iter()
        .map(|(ts, (value, unit, ingested))| Row { ts, value, unit, ingested })
        .collect();
    rows.sort_by(|a, b| b.ts.cmp(&a.ts)); // newest first
    rows.truncate(limit);
    Ok(rows)
}

fn col_ts<'a>(
    batch: &'a arrow_array::RecordBatch,
    name: &str,
) -> anyhow::Result<&'a TimestampMicrosecondArray> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<TimestampMicrosecondArray>())
        .ok_or_else(|| anyhow::anyhow!("{name} column not Timestamp(Microsecond)"))
}

/// Accept RFC3339, or the `YYYY-MM-DD` a date input emits (start/end of day UTC).
fn parse_flexible(s: &str, end_of_day: bool) -> anyhow::Result<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    let d = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")?;
    let t = if end_of_day {
        chrono::NaiveTime::from_hms_opt(23, 59, 59)
    } else {
        chrono::NaiveTime::from_hms_opt(0, 0, 0)
    }
    .expect("valid time");
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(d.and_time(t), Utc))
}

fn fmt_ts(micros: i64) -> String {
    DateTime::<Utc>::from_timestamp_micros(micros)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Rows + an out-of-band `#m-before` cursor (oldest ts on the page, or empty when
/// the page wasn't full → exhausted). The page includes `#m-before` on its
/// "Hent flere" button so each click advances the keyset.
fn render_fragment(rows: &[Row], limit: usize) -> String {
    let mut out = String::new();
    if rows.is_empty() {
        out.push_str("<tr><td colspan=\"4\" class=\"muted\">Ingen aflæsninger i perioden.</td></tr>");
    }
    for r in rows {
        out.push_str(&format!(
            "<tr><td class=\"mono\">{}</td><td class=\"mono\" style=\"text-align:right\">{:.3}</td><td>{}</td><td class=\"mono muted\">{}</td></tr>",
            fmt_ts(r.ts),
            r.value,
            esc(&r.unit),
            fmt_ts(r.ingested),
        ));
    }
    let next_before = if rows.len() == limit {
        rows.last()
            .and_then(|r| DateTime::<Utc>::from_timestamp_micros(r.ts))
            .map(|d| d.to_rfc3339())
            .unwrap_or_default()
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
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn html(status: u16, body: String) -> Result<Response<Body>, Error> {
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Body::Text(body))
        .expect("response"))
}
