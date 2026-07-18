//! DuckDB edge: open a connection, load the iceberg/httpfs/aws extensions, attach the
//! S3 Tables bucket, and query `all.raw_data` with bound params.
//!
//! The connection (with extensions loaded + bucket attached) is built once at cold
//! start and reused across warm invocations — that's the DuckDB-idiomatic warm path.
//! ATTACH resolves the current snapshot at attach time, so warm reuse trades a little
//! snapshot freshness for speed (acceptable for the spike; the DataFusion arm reloads
//! per request, which is part of why its floor is ~5s).

use duckdb::{params, params_from_iter, Connection};

use crate::measurements::{Measurement, MeasurementQuery};
use crate::status::{StatusQuery, StatusRow};
use crate::types::{RawQuery, RawRow};

#[derive(Debug, thiserror::Error)]
pub enum DuckError {
    #[error("duckdb setup: {0}")]
    Setup(String),
    #[error("duckdb query: {0}")]
    Query(String),
}

/// Cold-start: open an in-memory connection, install/load extensions into /tmp,
/// register credential-chain creds, and attach the S3 Tables bucket as `s3t`.
pub fn setup(table_bucket_arn: &str, region: &str) -> Result<Connection, DuckError> {
    let conn = Connection::open_in_memory().map_err(|e| DuckError::Setup(e.to_string()))?;

    // Run each step individually with flushed logging, so a Lambda failure names the
    // exact step (extension INSTALL vs LOAD vs SECRET vs ATTACH) in CloudWatch.
    let secret = format!("CREATE SECRET s3creds (TYPE s3, PROVIDER credential_chain, REGION '{region}');");
    let attach = format!("ATTACH '{table_bucket_arn}' AS s3t (TYPE ICEBERG, ENDPOINT_TYPE S3_TABLES);");
    // Extensions INSTALL at runtime into /tmp (from DuckDB's repo — the lambda is
    // non-VPC, so it has internet). Measured tradeoff (2026-07-17): this ~6s cold beats
    // BUNDLING the 116M of extensions in the package (that gave a ~10s cold — reading
    // them from read-only /var/task is slower than Lambda's fast network download +
    // load from writable /tmp). Warm is ~1.7s either way. The real cold-start lever is
    // provisioned concurrency, not bundling.
    // NB: no `SET TimeZone='UTC'` — that forces an icu autoload (fails in Lambda), and
    // DuckDB's default timezone is already UTC.
    let steps: [&str; 10] = [
        "SET home_directory='/tmp';",
        "SET extension_directory='/tmp/duckdb_extensions';",
        "INSTALL aws;",
        "LOAD aws;",
        "INSTALL httpfs;",
        "LOAD httpfs;",
        "INSTALL iceberg;",
        "LOAD iceberg;",
        &secret,
        &attach,
    ];
    for step in steps {
        step_log(&format!("duck-setup: {step}"));
        conn.execute_batch(step).map_err(|e| {
            step_log(&format!("duck-setup FAILED at [{step}]: {e}"));
            DuckError::Setup(format!("at [{step}]: {e}"))
        })?;
    }
    step_log("duck-setup: complete");
    Ok(conn)
}

/// stdout + flush so the line reaches CloudWatch even if the process exits abruptly.
fn step_log(msg: &str) {
    use std::io::Write;
    println!("{msg}");
    let _ = std::io::stdout().flush();
}

/// Run the raw_data query with bound params (injection-safe by construction).
pub fn query(conn: &Connection, q: &RawQuery) -> Result<Vec<RawRow>, DuckError> {
    let sql = "SELECT CAST(\"timestamp\" AS VARCHAR) AS ts, value, unit \
               FROM s3t.all.raw_data \
               WHERE daq_id = ? \
                 AND \"timestamp\" >= ?::TIMESTAMPTZ \
                 AND \"timestamp\" <= ?::TIMESTAMPTZ \
               ORDER BY \"timestamp\" DESC";

    let mut stmt = conn.prepare(sql).map_err(|e| DuckError::Query(e.to_string()))?;
    let rows = stmt
        .query_map(
            params![
                q.daq_id.as_str(),
                q.from.to_rfc3339(),
                q.to.to_rfc3339()
            ],
            |row| {
                Ok(RawRow {
                    timestamp: row.get::<_, String>(0)?,
                    value: row.get::<_, Option<f64>>(1)?,
                    unit: row.get::<_, Option<String>>(2)?,
                })
            },
        )
        .map_err(|e| DuckError::Query(e.to_string()))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| DuckError::Query(e.to_string()))
}

/// Datatilegnelse query: newest reading per timestamp (dedup via `max_by(_, ingested_time)`,
/// mirroring the Athena path), keyset-paginated by `before`, limited. Values pre-formatted.
pub fn query_measurements(conn: &Connection, q: &MeasurementQuery) -> Result<Vec<Measurement>, DuckError> {
    // `before` clause is added only when present; `limit` is a validated 1..=500 int, so
    // it is safe to inline. daq/from/to/before are bound (`?`) — injection-safe.
    let mut sql = String::from(
        "SELECT strftime(\"timestamp\"::TIMESTAMP, '%Y-%m-%d %H:%M:%S') AS ts, \
                max_by(value, ingested_time) AS value, \
                max_by(unit, ingested_time)  AS unit \
         FROM s3t.all.raw_data \
         WHERE daq_id = ? \
           AND \"timestamp\" >= ?::TIMESTAMPTZ \
           AND \"timestamp\" <= ?::TIMESTAMPTZ",
    );
    let mut binds: Vec<&str> = vec![q.daq_id.as_str(), q.from.as_str(), q.to.as_str()];
    if let Some(b) = q.before.as_deref() {
        sql.push_str(" AND \"timestamp\" < ?::TIMESTAMPTZ");
        binds.push(b);
    }
    sql.push_str(&format!(
        " GROUP BY daq_id, \"timestamp\" ORDER BY \"timestamp\" DESC LIMIT {}",
        q.limit
    ));

    let mut stmt = conn.prepare(&sql).map_err(|e| DuckError::Query(e.to_string()))?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            let ts: String = row.get(0)?;
            let value: Option<f64> = row.get(1)?;
            let unit: Option<String> = row.get(2)?;
            Ok(Measurement {
                timestamp: ts,
                value: value.map(|v| format!("{v:.3}")).unwrap_or_default(),
                unit: unit.unwrap_or_default(),
            })
        })
        .map_err(|e| DuckError::Query(e.to_string()))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| DuckError::Query(e.to_string()))
}

/// Device-liveness status from the heartbeat Parquet lake (plain S3 Parquet — DuckDB
/// reads it directly, no Iceberg/Athena). Filters by gateway_id and/or device_id over
/// the recent `dt` partitions. `bucket` is env-provided (trusted); ids are bound params.
pub fn query_status(conn: &Connection, q: &StatusQuery, bucket: &str) -> Result<Vec<StatusRow>, DuckError> {
    let mut sql = format!(
        "SELECT device_id, gateway_id, schematype, transport, \
                max(ingest_time) AS last_seen, max(event_time) AS last_event, count(*) AS msgs \
         FROM read_parquet('s3://{bucket}/heartbeat/**/*.parquet', hive_partitioning=true, union_by_name=true) \
         WHERE dt >= '{}'",
        q.since
    );
    let mut binds: Vec<&str> = Vec::new();
    if let Some(g) = q.gatewayid.as_deref() {
        sql.push_str(" AND gateway_id = ?");
        binds.push(g);
    }
    if let Some(m) = q.meterid.as_deref() {
        sql.push_str(" AND device_id = ?");
        binds.push(m);
    }
    sql.push_str(" GROUP BY device_id, gateway_id, schematype, transport ORDER BY last_seen DESC LIMIT 200");

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        // An empty lake (no Parquet written yet) makes read_parquet raise "No files found".
        // That's not an error for a liveness check — it means the device hasn't reported,
        // so return zero rows and let the caller render the "no data" answer.
        Err(e) if is_no_files(&e) => return Ok(Vec::new()),
        Err(e) => return Err(DuckError::Query(e.to_string())),
    };
    let rows = match stmt.query_map(params_from_iter(binds.iter()), |row| {
        Ok(StatusRow {
            device_id: row.get::<_, Option<String>>(0)?,
            gateway_id: row.get::<_, Option<String>>(1)?,
            schematype: row.get::<_, Option<String>>(2)?,
            transport: row.get::<_, Option<String>>(3)?,
            last_seen: row.get::<_, Option<String>>(4)?,
            last_event: row.get::<_, Option<String>>(5)?,
            msgs: row.get::<_, i64>(6)?,
        })
    }) {
        Ok(r) => r,
        Err(e) if is_no_files(&e) => return Ok(Vec::new()),
        Err(e) => return Err(DuckError::Query(e.to_string())),
    };
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| DuckError::Query(e.to_string()))
}

/// DuckDB raises this IO error when a read_parquet glob matches no objects — expected
/// while the heartbeat lake is still empty (or the partition window has no data).
fn is_no_files(e: &duckdb::Error) -> bool {
    e.to_string().contains("No files found")
}
