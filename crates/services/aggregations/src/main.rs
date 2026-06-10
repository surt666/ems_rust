// Rust port of infra/daq/data_pipeline/lambda/aggregations-go/main.go
//
// GET /aggregations?level_id=<hierarchy path>&resolution=<hourly|daily>
//                  &purpose=<opt>&start=<ISO>&end=<ISO>
//
// Response: JSON array, one row per (purpose, bucket), sorted by purpose then time.

use anyhow::{anyhow, Result};
use aws_sdk_dynamodb::{types::AttributeValue, Client};
use chrono::{DateTime, Utc};
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

// ── pure helpers ──────────────────────────────────────────────────────────────

static HN_RE: OnceLock<Regex> = OnceLock::new();

fn hn_re() -> &'static Regex {
    HN_RE.get_or_init(|| Regex::new(r"HN(\d+)#(\d+)").unwrap())
}

/// Extract HN segments from `level_id`, return `(pk, sk_path)` starting at HN2.
/// pk = "HN2#<id>", sk_path = "|"-joined path from HN2 onwards.
/// Returns error if there is no HN2 segment.
fn parse_node_keys(level_id: &str) -> Result<(String, String)> {
    let segs: Vec<String> = hn_re()
        .captures_iter(level_id)
        .map(|cap| format!("HN{}#{}", &cap[1], &cap[2]))
        .collect();

    let hn2 = segs
        .iter()
        .position(|s| s.starts_with("HN2#"))
        .ok_or_else(|| anyhow!("level_id has no HN2 (company) segment: {:?}", level_id))?;

    let path = &segs[hn2..];
    Ok((path[0].clone(), path.join("|")))
}

/// Map resolution string to granularity character.
fn gran_of(resolution: &str) -> &'static str {
    if resolution == "daily" {
        "d"
    } else {
        "h"
    }
}

/// Parse an RFC-3339 / ISO-8601 string (trimmed) to UTC.
fn parse_iso(s: &str) -> Result<DateTime<Utc>> {
    let t: DateTime<Utc> = s
        .trim()
        .parse::<DateTime<chrono::FixedOffset>>()
        .map_err(|e| anyhow!("parse_iso: {}", e))?
        .with_timezone(&Utc);
    Ok(t)
}

/// Format a timestamp as the bucket label for querying.
/// hour → "YYYY-MM-DDThh", day → "YYYY-MM-DD".
fn bucket_label(iso: &str, gran: &str) -> Result<String> {
    let t = parse_iso(iso)?;
    if gran == "h" {
        Ok(t.format("%Y-%m-%dT%H").to_string())
    } else {
        Ok(t.format("%Y-%m-%d").to_string())
    }
}

/// Expand a bucket label back to a full ISO-8601 UTC instant string.
/// On parse failure returns the bucket unchanged (mirrors Go behaviour).
fn bucket_to_iso(bucket: &str, gran: &str) -> String {
    if gran == "h" {
        // bucket looks like "2024-01-15T10" — parse as NaiveDateTime by appending ":00:00"
        match chrono::NaiveDateTime::parse_from_str(
            &format!("{}:00:00", bucket),
            "%Y-%m-%dT%H:%M:%S",
        ) {
            Ok(ndt) => ndt.and_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            Err(_) => bucket.to_string(),
        }
    } else {
        // bucket looks like "2024-01-15" — parse as NaiveDate then convert to midnight UTC
        match chrono::NaiveDate::parse_from_str(bucket, "%Y-%m-%d") {
            Ok(nd) => nd
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
            Err(_) => bucket.to_string(),
        }
    }
}

/// Split a sort-key "<node_path>#<purpose>#<gran>#<bucket>" on the last 3 `#`.
/// node_path may itself contain `#`. Mirrors Python's `sk.rsplit("#", 3)`.
fn parse_sk(sk: &str) -> (&str, &str, &str, &str) {
    let bytes = sk.as_bytes();
    let mut cuts: Vec<usize> = Vec::with_capacity(3);
    let mut i = bytes.len();
    while i > 0 && cuts.len() < 3 {
        i -= 1;
        if bytes[i] == b'#' {
            cuts.push(i);
        }
    }
    if cuts.len() < 3 {
        return (sk, "", "", "");
    }
    let (b, g, p) = (cuts[0], cuts[1], cuts[2]);
    (&sk[..p], &sk[p + 1..g], &sk[g + 1..b], &sk[b + 1..])
}

// ── DynamoDB item shape ───────────────────────────────────────────────────────

#[derive(Debug)]
struct AggItem {
    sk: String,
    sum: f64,
    count: i64,
    unit: String,
}

/// Extract a String attribute value (returns "" on missing/wrong type).
fn attr_s(item: &HashMap<String, AttributeValue>, key: &str) -> String {
    item.get(key)
        .and_then(|v| v.as_s().ok())
        .cloned()
        .unwrap_or_default()
}

/// Extract a Number attribute value as f64 (0.0 on missing/parse error).
fn attr_n_f64(item: &HashMap<String, AttributeValue>, key: &str) -> f64 {
    item.get(key)
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}

/// Extract a Number attribute value as i64 (0 on missing/parse error).
fn attr_n_i64(item: &HashMap<String, AttributeValue>, key: &str) -> i64 {
    item.get(key)
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn decode_item(item: HashMap<String, AttributeValue>) -> AggItem {
    AggItem {
        sk: attr_s(&item, "sk"),
        sum: attr_n_f64(&item, "sum"),
        count: attr_n_i64(&item, "count"),
        unit: attr_s(&item, "unit"),
    }
}

// ── JSON response row (field names match Go json tags exactly) ────────────────

#[derive(Debug, Serialize, Deserialize)]
struct Row {
    level_id: String,
    purpose: String,
    unit: String,
    resolution: String,
    timestamp: String,
    value: f64,
    contributor_count: i64,
}

/// Group items by purpose (keeping only those matching `gran`), sort, build rows.
fn to_rows(items: Vec<AggItem>, level_id: &str, resolution: &str, gran: &str) -> Vec<Row> {
    // Group by purpose, filter on gran
    let mut by_purpose: HashMap<String, Vec<(String, AggItem)>> = HashMap::new();
    for it in items {
        let (_, purpose, g, bucket) = parse_sk(&it.sk);
        if g != gran {
            continue;
        }
        by_purpose
            .entry(purpose.to_string())
            .or_default()
            .push((bucket.to_string(), it));
    }

    // Sort purposes
    let mut purposes: Vec<String> = by_purpose.keys().cloned().collect();
    purposes.sort();

    let mut rows = Vec::new();
    for purpose in purposes {
        let mut entries = by_purpose.remove(&purpose).unwrap();
        // Sort by bucket (lexicographic — same as Go's string comparison)
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (bucket, it) in entries {
            rows.push(Row {
                level_id: level_id.to_string(),
                purpose: purpose.clone(),
                unit: it.unit,
                resolution: resolution.to_string(),
                timestamp: bucket_to_iso(&bucket, gran),
                value: it.sum,
                contributor_count: it.count,
            });
        }
    }
    rows
}

// ── DynamoDB query ────────────────────────────────────────────────────────────

struct QueryParams<'a> {
    table: &'a str,
    pk: &'a str,
    sk_path: &'a str,
    gran: &'a str,
    start_bucket: &'a str,
    end_bucket: &'a str,
    purpose: &'a str,
}

async fn query_node(client: &Client, p: QueryParams<'_>) -> Result<Vec<AggItem>> {
    let QueryParams {
        table,
        pk,
        sk_path,
        gran,
        start_bucket,
        end_bucket,
        purpose,
    } = p;
    let mut items: Vec<AggItem> = Vec::new();
    let mut last_key: Option<HashMap<String, AttributeValue>> = None;

    loop {
        let mut req = if !purpose.is_empty() {
            // Efficient range: pk = :pk AND sk BETWEEN prefix+start AND prefix+end
            let prefix = format!("{}#{}#{}#", sk_path, purpose, gran);
            let start_val = format!("{}{}", prefix, start_bucket);
            let end_val = format!("{}{}", prefix, end_bucket);
            client
                .query()
                .table_name(table)
                .key_condition_expression("pk = :pk AND sk BETWEEN :sk_start AND :sk_end")
                .expression_attribute_values(":pk", AttributeValue::S(pk.to_string()))
                .expression_attribute_values(":sk_start", AttributeValue::S(start_val))
                .expression_attribute_values(":sk_end", AttributeValue::S(end_val))
        } else {
            // All purposes: pk = :pk AND begins_with(sk, sk_path#), filter bucket range
            client
                .query()
                .table_name(table)
                .key_condition_expression("pk = :pk AND begins_with(sk, :sk_prefix)")
                // `bucket` is a DynamoDB reserved word — escape it with an attribute name
                // (the aws-sdk-go expression builder did this automatically in the Go version).
                .filter_expression("#bk BETWEEN :b_start AND :b_end")
                .expression_attribute_names("#bk", "bucket")
                .expression_attribute_values(":pk", AttributeValue::S(pk.to_string()))
                .expression_attribute_values(
                    ":sk_prefix",
                    AttributeValue::S(format!("{}#", sk_path)),
                )
                .expression_attribute_values(":b_start", AttributeValue::S(start_bucket.to_string()))
                .expression_attribute_values(":b_end", AttributeValue::S(end_bucket.to_string()))
        };

        if let Some(lk) = last_key {
            req = req.set_exclusive_start_key(Some(lk));
        }

        let page = req.send().await.map_err(|e| anyhow!("DynamoDB query: {:?}", e))?;
        let page_items: Vec<AggItem> = page.items.unwrap_or_default().into_iter().map(decode_item).collect();
        items.extend(page_items);

        last_key = page.last_evaluated_key;
        if last_key.is_none() {
            break;
        }
    }

    Ok(items)
}

// ── Lambda handler ────────────────────────────────────────────────────────────

fn json_response(status: u16, body: impl Serialize) -> Result<Response<Body>, Error> {
    // CORS is added by the Function URL config — do NOT set Access-Control-Allow-Origin here
    // too, or the browser sees duplicate headers.
    let json = serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string());
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::Text(json))
        .expect("failed to build response"))
}

async fn handler(event: Request) -> Result<Response<Body>, Error> {
    // Lazy-init DynamoDB client (captured via closure in main, threaded through via OnceLock)
    let client = DDB_CLIENT.get().expect("DDB client not initialized");
    let table = ROLLUP_TABLE.get().expect("ROLLUP_TABLE not set");

    // Parse query params from URI
    let uri = event.uri();
    let qs: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default();

    let level_id = qs.get("level_id").cloned().unwrap_or_default();
    let resolution = qs
        .get("resolution")
        .cloned()
        .unwrap_or_else(|| "hourly".to_string());
    let purpose = qs.get("purpose").cloned().unwrap_or_default();
    let start = qs.get("start").cloned().unwrap_or_default();
    let end = qs.get("end").cloned().unwrap_or_default();

    if start.is_empty() || end.is_empty() {
        return json_response(
            400,
            HashMap::from([("error", "start and end are required (ISO-8601)")]),
        );
    }

    let gran = gran_of(&resolution);

    let (pk, sk_path) = match parse_node_keys(&level_id) {
        Ok(keys) => keys,
        Err(_) => {
            // node above company level (HN0/HN1) — nothing to aggregate at a single partition
            let empty: Vec<Row> = vec![];
            return json_response(200, empty);
        }
    };

    let start_bucket = match bucket_label(&start, gran) {
        Ok(b) => b,
        Err(_) => {
            return json_response(
                400,
                HashMap::from([("error", "start/end must be ISO-8601 timestamps")]),
            );
        }
    };
    let end_bucket = match bucket_label(&end, gran) {
        Ok(b) => b,
        Err(_) => {
            return json_response(
                400,
                HashMap::from([("error", "start/end must be ISO-8601 timestamps")]),
            );
        }
    };

    match query_node(
        client,
        QueryParams {
            table,
            pk: &pk,
            sk_path: &sk_path,
            gran,
            start_bucket: &start_bucket,
            end_bucket: &end_bucket,
            purpose: &purpose,
        },
    )
    .await
    {
        Ok(items) => json_response(200, to_rows(items, &level_id, &resolution, gran)),
        Err(e) => json_response(500, HashMap::from([("error", e.to_string())])),
    }
}

// ── global state (initialized in main, read in handler) ──────────────────────

static DDB_CLIENT: OnceLock<Client> = OnceLock::new();
static ROLLUP_TABLE: OnceLock<String> = OnceLock::new();

#[tokio::main]
async fn main() -> Result<(), Error> {
    let cfg = aws_config::load_from_env().await;
    DDB_CLIENT.set(Client::new(&cfg)).ok();

    let table = std::env::var("ROLLUP_TABLE")
        .unwrap_or_else(|_| "measurements_aggregate".to_string());
    ROLLUP_TABLE.set(table).ok();

    run(service_fn(handler)).await
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // parse_node_keys ──────────────────────────────────────────────────────────

    #[test]
    fn test_parse_node_keys_simple() {
        // HN2 is the first segment
        let (pk, sk_path) = parse_node_keys("HN2#42").unwrap();
        assert_eq!(pk, "HN2#42");
        assert_eq!(sk_path, "HN2#42");
    }

    #[test]
    fn test_parse_node_keys_with_prefix() {
        // HN0/HN1 prefix before HN2 — should be dropped
        let (pk, sk_path) = parse_node_keys("HN0#1|HN1#7|HN2#42|HN3#99").unwrap();
        assert_eq!(pk, "HN2#42");
        assert_eq!(sk_path, "HN2#42|HN3#99");
    }

    #[test]
    fn test_parse_node_keys_multi_segment_path() {
        let (pk, sk_path) = parse_node_keys("HN2#10|HN3#20|HN4#30").unwrap();
        assert_eq!(pk, "HN2#10");
        assert_eq!(sk_path, "HN2#10|HN3#20|HN4#30");
    }

    #[test]
    fn test_parse_node_keys_no_hn2_errors() {
        assert!(parse_node_keys("HN0#1|HN1#7").is_err());
    }

    #[test]
    fn test_parse_node_keys_empty_errors() {
        assert!(parse_node_keys("").is_err());
    }

    // parse_sk ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_sk_simple() {
        // node_path contains no '#'
        let (node, purpose, gran, bucket) = parse_sk("HN2#42|HN3#99#electricity#h#2024-01-15T10");
        assert_eq!(node, "HN2#42|HN3#99");
        assert_eq!(purpose, "electricity");
        assert_eq!(gran, "h");
        assert_eq!(bucket, "2024-01-15T10");
    }

    #[test]
    fn test_parse_sk_node_path_with_hash() {
        // node_path itself contains '#' — the last 3 splits should give the correct fields
        // sk = "HN2#10|HN3#20#gas#d#2024-06-01"
        let (node, purpose, gran, bucket) = parse_sk("HN2#10|HN3#20#gas#d#2024-06-01");
        assert_eq!(node, "HN2#10|HN3#20");
        assert_eq!(purpose, "gas");
        assert_eq!(gran, "d");
        assert_eq!(bucket, "2024-06-01");
    }

    #[test]
    fn test_parse_sk_too_few_hashes() {
        let (node, purpose, gran, bucket) = parse_sk("only#two#hashes");
        // 2 '#' < 3, so fallback: node = full sk, rest empty
        assert_eq!(node, "only#two#hashes");
        assert_eq!(purpose, "");
        assert_eq!(gran, "");
        assert_eq!(bucket, "");
    }

    // bucket_label / bucket_to_iso round-trips ──────────────────────────────────

    #[test]
    fn test_bucket_label_hour() {
        let label = bucket_label("2024-01-15T10:30:00Z", "h").unwrap();
        assert_eq!(label, "2024-01-15T10");
    }

    #[test]
    fn test_bucket_label_day() {
        let label = bucket_label("2024-01-15T10:30:00Z", "d").unwrap();
        assert_eq!(label, "2024-01-15");
    }

    #[test]
    fn test_bucket_to_iso_hour() {
        let iso = bucket_to_iso("2024-01-15T10", "h");
        assert_eq!(iso, "2024-01-15T10:00:00Z");
    }

    #[test]
    fn test_bucket_to_iso_day() {
        let iso = bucket_to_iso("2024-01-15", "d");
        assert_eq!(iso, "2024-01-15T00:00:00Z");
    }

    #[test]
    fn test_bucket_to_iso_invalid_returns_unchanged() {
        // On parse failure, return the bucket string unchanged (matches Go)
        let iso = bucket_to_iso("not-a-date", "h");
        assert_eq!(iso, "not-a-date");
    }

    #[test]
    fn test_bucket_label_to_iso_round_trip_hour() {
        let original = "2024-06-07T14:00:00Z";
        let label = bucket_label(original, "h").unwrap();
        let back = bucket_to_iso(&label, "h");
        assert_eq!(back, "2024-06-07T14:00:00Z");
    }

    #[test]
    fn test_bucket_label_to_iso_round_trip_day() {
        let original = "2024-06-07T00:00:00Z";
        let label = bucket_label(original, "d").unwrap();
        let back = bucket_to_iso(&label, "d");
        assert_eq!(back, "2024-06-07T00:00:00Z");
    }

    // to_rows ─────────────────────────────────────────────────────────────────

    fn make_item(sk: &str, sum: f64, count: i64, unit: &str) -> AggItem {
        AggItem {
            sk: sk.to_string(),
            sum,
            count,
            unit: unit.to_string(),
        }
    }

    #[test]
    fn test_to_rows_groups_by_purpose() {
        let items = vec![
            make_item("HN2#1#electricity#h#2024-01-01T10", 100.0, 5, "kWh"),
            make_item("HN2#1#gas#h#2024-01-01T10", 50.0, 3, "m3"),
        ];
        let rows = to_rows(items, "HN2#1", "hourly", "h");
        assert_eq!(rows.len(), 2);
        // sorted by purpose: electricity < gas
        assert_eq!(rows[0].purpose, "electricity");
        assert_eq!(rows[1].purpose, "gas");
    }

    #[test]
    fn test_to_rows_filters_wrong_gran() {
        let items = vec![
            make_item("HN2#1#electricity#h#2024-01-01T10", 100.0, 5, "kWh"),
            make_item("HN2#1#electricity#d#2024-01-01", 2400.0, 5, "kWh"), // daily, should be filtered
        ];
        // request hourly
        let rows = to_rows(items, "HN2#1", "hourly", "h");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].resolution, "hourly");
        assert_eq!(rows[0].timestamp, "2024-01-01T10:00:00Z");
    }

    #[test]
    fn test_to_rows_sorted_by_bucket_within_purpose() {
        let items = vec![
            make_item("HN2#1#electricity#h#2024-01-01T12", 30.0, 1, "kWh"),
            make_item("HN2#1#electricity#h#2024-01-01T10", 10.0, 1, "kWh"),
            make_item("HN2#1#electricity#h#2024-01-01T11", 20.0, 1, "kWh"),
        ];
        let rows = to_rows(items, "HN2#1", "hourly", "h");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].timestamp, "2024-01-01T10:00:00Z");
        assert_eq!(rows[1].timestamp, "2024-01-01T11:00:00Z");
        assert_eq!(rows[2].timestamp, "2024-01-01T12:00:00Z");
    }

    #[test]
    fn test_to_rows_json_field_names() {
        // Verify the exact JSON field names match the Go json tags
        let items = vec![make_item(
            "HN2#10#electricity#h#2024-01-01T08",
            42.5,
            7,
            "kWh",
        )];
        let rows = to_rows(items, "HN2#10", "hourly", "h");
        assert_eq!(rows.len(), 1);
        let val = serde_json::to_value(&rows[0]).unwrap();
        assert!(val.get("level_id").is_some(), "missing level_id");
        assert!(val.get("purpose").is_some(), "missing purpose");
        assert!(val.get("unit").is_some(), "missing unit");
        assert!(val.get("resolution").is_some(), "missing resolution");
        assert!(val.get("timestamp").is_some(), "missing timestamp");
        assert!(val.get("value").is_some(), "missing value");
        assert!(
            val.get("contributor_count").is_some(),
            "missing contributor_count"
        );
        assert_eq!(val["level_id"], "HN2#10");
        assert_eq!(val["purpose"], "electricity");
        assert_eq!(val["unit"], "kWh");
        assert_eq!(val["resolution"], "hourly");
        assert_eq!(val["timestamp"], "2024-01-01T08:00:00Z");
        assert_eq!(val["value"], 42.5);
        assert_eq!(val["contributor_count"], 7);
    }

    #[test]
    fn test_to_rows_empty_items() {
        let rows = to_rows(vec![], "HN2#1", "hourly", "h");
        assert!(rows.is_empty());
    }

    #[test]
    fn test_to_rows_daily() {
        let items = vec![
            make_item("HN2#5#heat#d#2024-03-02", 800.0, 10, "kWh"),
            make_item("HN2#5#heat#d#2024-03-01", 900.0, 10, "kWh"),
        ];
        let rows = to_rows(items, "HN2#5", "daily", "d");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].timestamp, "2024-03-01T00:00:00Z");
        assert_eq!(rows[1].timestamp, "2024-03-02T00:00:00Z");
    }

    // gran_of ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_gran_of() {
        assert_eq!(gran_of("daily"), "d");
        assert_eq!(gran_of("hourly"), "h");
        assert_eq!(gran_of(""), "h"); // default
        assert_eq!(gran_of("anything-else"), "h");
    }
}
