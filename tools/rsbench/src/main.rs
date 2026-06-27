//! Redshift Spectrum query endpoint for the /measurements (Datatilegnelse) page —
//! an ALTERNATIVE backend to the Athena route, selectable via a frontend checkbox.
//! VPC lambda (tier2) connecting to Redshift Serverless over tokio-postgres (TLS),
//! querying `spectrum_rl.raw_data` (S3 Tables). Returns the SAME HTML fragment as
//! the Athena `/measurements` route so the page swap logic is identical.
use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};
use tokio_postgres::SimpleQueryMessage;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 500;

#[derive(Debug)]
struct NoVerify;
impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(&self, _e: &CertificateDer<'_>, _i: &[CertificateDer<'_>], _n: &ServerName<'_>, _o: &[u8], _t: UnixTime) -> Result<ServerCertVerified, TlsError> { Ok(ServerCertVerified::assertion()) }
    fn verify_tls12_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, TlsError> { Ok(HandshakeSignatureValid::assertion()) }
    fn verify_tls13_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, TlsError> { Ok(HandshakeSignatureValid::assertion()) }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::RSA_PKCS1_SHA256, SignatureScheme::RSA_PKCS1_SHA384, SignatureScheme::RSA_PKCS1_SHA512,
             SignatureScheme::ECDSA_NISTP256_SHA256, SignatureScheme::ECDSA_NISTP384_SHA384,
             SignatureScheme::RSA_PSS_SHA256, SignatureScheme::RSA_PSS_SHA384, SignatureScheme::RSA_PSS_SHA512, SignatureScheme::ED25519]
    }
}

fn ts_lit(s: &str, end_of_day: bool) -> Option<String> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) { return Some(dt.with_timezone(&Utc).format("%Y-%m-%d %H:%M:%S").to_string()); }
    for f in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, f) { return Some(ndt.format("%Y-%m-%d %H:%M:%S").to_string()); }
    }
    let d = NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()?;
    let t = if end_of_day { NaiveTime::from_hms_opt(23,59,59) } else { NaiveTime::from_hms_opt(0,0,0) }.unwrap();
    Some(d.and_time(t).format("%Y-%m-%d %H:%M:%S").to_string())
}

fn esc(s: &str) -> String { s.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;") }
fn fmt_date(s: &str) -> String { if s.len() >= 16 { s[..16].to_string() } else { s.to_string() } }
fn html(status: u16, body: String) -> Result<Response<Body>, Error> {
    Ok(Response::builder().status(status).header("Content-Type","text/html; charset=utf-8").body(Body::Text(body)).expect("resp"))
}

async fn handler(event: Request) -> Result<Response<Body>, Error> {
    let qs: HashMap<String,String> = event.uri().query()
        .map(|q| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()).unwrap_or_default();
    let daq = qs.get("daq_id").cloned().unwrap_or_default();
    if daq.is_empty() { return html(400, "<tr><td colspan=\"4\">daq_id required</td></tr>".into()); }
    let now = Utc::now();
    let from = qs.get("from").map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        .and_then(|s| ts_lit(&s, false)).unwrap_or_else(|| (now - Duration::days(1)).format("%Y-%m-%d %H:%M:%S").to_string());
    let to = qs.get("to").map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        .and_then(|s| ts_lit(&s, true)).unwrap_or_else(|| now.format("%Y-%m-%d %H:%M:%S").to_string());
    let before = qs.get("before").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).and_then(|s| ts_lit(&s, false));
    let limit = qs.get("limit").and_then(|s| s.parse::<usize>().ok()).unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let before_clause = before.map(|b| format!(" AND \"timestamp\" < '{}'::timestamptz", b)).unwrap_or_default();
    let sql = format!(
        "SELECT \"timestamp\",\"value\",\"unit\" FROM (SELECT \"timestamp\",\"value\",\"unit\", ROW_NUMBER() OVER (PARTITION BY \"timestamp\" ORDER BY \"ingested_time\" DESC) rn FROM spectrum_rl.raw_data WHERE \"daq_id\"='{}' AND \"timestamp\" >= '{}'::timestamptz AND \"timestamp\" <= '{}'::timestamptz{}) WHERE rn=1 ORDER BY \"timestamp\" DESC LIMIT {}",
        daq.replace('\'', "''"), from, to, before_clause, limit);

    match query(&sql).await {
        Ok(rows) => html(200, render(&rows, limit)),
        Err(e) => html(500, format!("<tr><td colspan=\"4\">Redshift fejl: {}</td></tr>", esc(&e.to_string()))),
    }
}

async fn query(sql: &str) -> Result<Vec<(String,String,String)>, Error> {
    let host = std::env::var("REDSHIFT_HOST")?;
    let user = std::env::var("REDSHIFT_USER").unwrap_or_else(|_| "spikeadmin".into());
    let pw = std::env::var("REDSHIFT_PASSWORD")?;
    let db = std::env::var("REDSHIFT_DB").unwrap_or_else(|_| "dev".into());
    let conn_str = format!("host={host} port=5439 user={user} password={pw} dbname={db} sslmode=require connect_timeout=60");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = rustls::ClientConfig::builder().dangerous().with_custom_certificate_verifier(Arc::new(NoVerify)).with_no_client_auth();
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(config);
    let (client, connection) = tokio_postgres::connect(&conn_str, tls).await?;
    let h = tokio::spawn(async move { let _ = connection.await; });
    let msgs = client.simple_query(sql).await?;
    let mut out = Vec::new();
    for m in &msgs {
        if let SimpleQueryMessage::Row(r) = m {
            out.push((r.get(0).unwrap_or("").to_string(), r.get(1).unwrap_or("").to_string(), r.get(2).unwrap_or("").to_string()));
        }
    }
    drop(client); let _ = h.await;
    Ok(out)
}

fn render(rows: &[(String,String,String)], limit: usize) -> String {
    let mut out = String::new();
    if rows.is_empty() { out.push_str("<tr><td colspan=\"4\" class=\"muted\">Ingen aflæsninger i perioden.</td></tr>"); }
    for (ts, val, unit) in rows {
        let v = val.parse::<f64>().map(|x| format!("{:.3}", x)).unwrap_or_else(|_| val.clone());
        out.push_str(&format!(
            "<tr><td class=\"mono\">{}</td><td class=\"mono\" style=\"text-align:right\">{}</td><td>{}</td><td class=\"mono muted\">{}</td></tr>",
            fmt_date(ts), esc(&v), esc(unit), fmt_date(ts)));
    }
    let next = if rows.len() == limit { rows.last().map(|r| r.0.split('.').next().unwrap_or(&r.0).to_string()).unwrap_or_default() } else { String::new() };
    out.push_str(&format!("<input id=\"m-before\" name=\"before\" type=\"hidden\" value=\"{}\" hx-swap-oob=\"true\">", esc(&next)));
    out
}

#[tokio::main]
async fn main() -> Result<(), Error> { run(service_fn(handler)).await }
