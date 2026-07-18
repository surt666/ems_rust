//! `query-raw-duck` — the DuckDB arm of the raw_data read spike. Same contract as
//! `query-raw` (GET ?daqid=&fromtime=&totime= → JSON rows), different engine:
//! DuckDB + iceberg extension + S3 Tables ATTACH instead of DataFusion + iceberg-rust.

use std::sync::{Arc, Mutex};

use duckdb::Connection;
use lambda_http::{run, service_fn, Body, Request, RequestExt, Response};

mod measurements;
mod reader_duck;
mod status;
mod types;

use measurements::{Format, MeasurementQuery};
use status::StatusQuery;
use types::{RawQuery, RawRow};

#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
    let arn = std::env::var("TABLE_BUCKET_ARN")
        .unwrap_or_else(|_| "arn:aws:s3tables:eu-central-1:891377204778:bucket/measurements".into());
    let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "eu-central-1".into());

    // Local reproduction: `cargo run -p query-raw-duck -- --selftest` (needs AWS creds
    // + network in env) exercises setup() and one query, printing the real DuckDB error.
    if std::env::args().any(|a| a == "--selftest") {
        return selftest(&arn, &region);
    }

    // The heartbeat Parquet lake (device-liveness). Same S3 credentials/extensions as the
    // iceberg ATTACH — DuckDB reads plain Parquet directly, so no separate secret needed.
    let heartbeat_bucket: Arc<str> =
        Arc::from(std::env::var("HEARTBEAT_BUCKET").unwrap_or_default());

    // Cold start: build the connection (extensions + ATTACH) once. Blocking DuckDB
    // work, but nothing is concurrent yet.
    let conn = reader_duck::setup(&arn, &region)?;
    let conn = Arc::new(Mutex::new(conn));

    run(service_fn(move |req: Request| {
        let conn = conn.clone();
        let heartbeat_bucket = heartbeat_bucket.clone();
        async move { handle_request(req, conn, heartbeat_bucket).await }
    }))
    .await
}

/// Route by path: `/rawdevice/status` is the device-liveness endpoint (heartbeat Parquet
/// lake); `/meterdata/query/get_measurements` is the production Datatilegnelse endpoint
/// (HTML/JSON per the aggregations contract); anything else is the raw JSON benchmark
/// endpoint (`/rawdata/query-duck`).
async fn handle_request(
    req: Request,
    conn: Arc<Mutex<Connection>>,
    heartbeat_bucket: Arc<str>,
) -> Result<Response<Body>, lambda_http::Error> {
    let path = req.uri().path();
    if path.contains("status") {
        handle_status(req, conn, heartbeat_bucket).await
    } else if path.contains("get_measurements") {
        handle_measurements(req, conn).await
    } else {
        handle_raw(req, conn).await
    }
}

/// Device liveness: "is this gateway/meter reporting?" — DuckDB `read_parquet` over the
/// heartbeat lake. Returns the Raw Device MFE HTML fragment (default) or JSON.
async fn handle_status(
    req: Request,
    conn: Arc<Mutex<Connection>>,
    heartbeat_bucket: Arc<str>,
) -> Result<Response<Body>, lambda_http::Error> {
    let qs = req.query_string_parameters();
    let get = |k: &str| qs.first(k).map(str::to_owned);
    let format = Format::resolve(get("format").as_deref());

    if heartbeat_bucket.is_empty() {
        return Ok(status_error(format, 500, "HEARTBEAT_BUCKET not configured"));
    }
    let query = match StatusQuery::parse(get) {
        Ok(q) => q,
        Err(e) => return Ok(status_error(format, 400, &e)),
    };

    let bucket = heartbeat_bucket.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| "connection lock poisoned".to_string())?;
        reader_duck::query_status(&conn, &query, &bucket)
            .map(|rows| (query, rows))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());

    match result {
        Ok(Ok((query, rows))) => Ok(match format {
            Format::Json => json_response(200, &rows),
            Format::Html => html_response(200, status::render_fragment(&query, &rows)),
        }),
        Ok(Err(e)) | Err(e) => Ok(status_error(format, 502, &e)),
    }
}

fn status_error(format: Format, status: u16, msg: &str) -> Response<Body> {
    match format {
        Format::Json => json_response(status, &serde_json::json!({ "error": msg })),
        Format::Html => html_response(
            status,
            format!("<div id=\"rd-result\"><p>\u{26a0} {}</p></div>", status::esc(msg)),
        ),
    }
}

/// Datatilegnelse: same contract as aggregations `get_measurements` — HTML `<tr>`
/// fragment (default) or bare `[Measurement]` JSON.
async fn handle_measurements(
    req: Request,
    conn: Arc<Mutex<Connection>>,
) -> Result<Response<Body>, lambda_http::Error> {
    let qs = req.query_string_parameters();
    let get = |k: &str| qs.first(k).map(str::to_owned);
    let format = Format::resolve(get("format").as_deref());

    let query = match MeasurementQuery::parse(get) {
        Ok(q) => q,
        Err(e) => return Ok(measurements_error(format, 400, &e)),
    };
    let limit = query.limit;

    let result = tokio::task::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| "connection lock poisoned".to_string())?;
        reader_duck::query_measurements(&conn, &query).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());

    match result {
        Ok(Ok(rows)) => Ok(match format {
            Format::Json => json_response(200, &rows),
            Format::Html => html_response(200, measurements::render_fragment(&rows, limit)),
        }),
        Ok(Err(e)) | Err(e) => Ok(measurements_error(format, 502, &e)),
    }
}

async fn handle_raw(
    req: Request,
    conn: Arc<Mutex<Connection>>,
) -> Result<Response<Body>, lambda_http::Error> {
    let query = match parse_query(&req) {
        Ok(q) => q,
        Err(e) => return Ok(json_response(400, &serde_json::json!({ "error": e }))),
    };

    // DuckDB is synchronous C — run it off the async reactor.
    let result = tokio::task::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| "connection lock poisoned".to_string())?;
        reader_duck::query(&conn, &query).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string());

    match result {
        Ok(Ok(rows)) => Ok(json_response(200, &RowsPayload { count: rows.len(), rows })),
        Ok(Err(e)) | Err(e) => Ok(json_response(502, &serde_json::json!({ "error": e }))),
    }
}

fn html_response(status: u16, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/html; charset=utf-8")
        .body(Body::from(body))
        .expect("well-formed response")
}

fn measurements_error(format: Format, status: u16, msg: &str) -> Response<Body> {
    match format {
        Format::Json => json_response(status, &serde_json::json!({ "error": msg })),
        Format::Html => html_response(status, measurements::error_fragment(msg)),
    }
}

#[derive(serde::Serialize)]
struct RowsPayload {
    count: usize,
    rows: Vec<RawRow>,
}

fn parse_query(req: &Request) -> Result<RawQuery, String> {
    let params = req.query_string_parameters();
    let get = |k: &str| params.first(k).map(str::to_owned);
    let daqid = get("daqid").ok_or("missing query param: daqid")?;
    let from = get("fromtime").ok_or("missing query param: fromtime")?;
    let to = get("totime").ok_or("missing query param: totime")?;
    RawQuery::parse(&daqid, &from, &to)
}

fn selftest(arn: &str, region: &str) -> Result<(), lambda_http::Error> {
    use std::time::Instant;
    let daq = "daq:std_json_v1:klepierre:60098154:energy";
    let t0 = Instant::now();
    let conn = reader_duck::setup(arn, region)?;
    eprintln!("setup (cold, extensions + ATTACH): {:.3}s", t0.elapsed().as_secs_f64());

    // Same windows as the DataFusion arm benchmark, warm (connection reused).
    let windows = [
        ("1-day", "2026-07-16T00:00:00Z", "2026-07-17T00:00:00Z"),
        ("7-day", "2026-07-10T00:00:00Z", "2026-07-17T00:00:00Z"),
        ("30-day", "2026-06-17T00:00:00Z", "2026-07-17T00:00:00Z"),
        ("90-day", "2026-04-18T00:00:00Z", "2026-07-17T00:00:00Z"),
    ];
    for (label, from, to) in windows {
        let q = RawQuery::parse(daq, from, to).map_err(|e| e.to_string())?;
        let t = Instant::now();
        let rows = reader_duck::query(&conn, &q)?;
        eprintln!("{label:>7}: {:>6.3}s  rows={}", t.elapsed().as_secs_f64(), rows.len());
    }
    Ok(())
}

fn json_response(status: u16, body: &impl serde::Serialize) -> Response<Body> {
    let payload = serde_json::to_string(body).unwrap_or_else(|_| "{}".into());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .expect("well-formed response")
}
