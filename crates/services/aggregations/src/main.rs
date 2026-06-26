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

mod raw;

// ── pure helpers ──────────────────────────────────────────────────────────────

fn hn_re() -> &'static Regex {
    static HN_RE: OnceLock<Regex> = OnceLock::new();
    HN_RE.get_or_init(|| Regex::new(r"HN\d+#\d+").unwrap())
}

/// Extract HN segments from `level_id`, return `(pk, sk_path)` starting at HN2.
/// pk = "HN2#<id>", sk_path = "|"-joined path from HN2 onwards.
/// Returns error if there is no HN2 segment.
fn parse_node_keys(level_id: &str) -> Result<(String, String)> {
    let segs: Vec<&str> = hn_re().find_iter(level_id).map(|m| m.as_str()).collect();

    let hn2 = segs
        .iter()
        .position(|s| s.starts_with("HN2#"))
        .ok_or_else(|| anyhow!("level_id has no HN2 (company) segment: {:?}", level_id))?;

    let path = &segs[hn2..];
    Ok((path[0].to_string(), path.join("|")))
}

/// Roll-up granularity. Owns the single-char sort-key code and the bucket
/// label ⇄ ISO-instant conversions, so the two directions can't drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gran {
    Hour,
    Day,
}

impl Gran {
    /// `"daily"` → `Day`; anything else (incl. `"hourly"`/empty) → `Hour`.
    fn from_resolution(resolution: &str) -> Gran {
        if resolution == "daily" {
            Gran::Day
        } else {
            Gran::Hour
        }
    }

    /// The single character stored in the sort key (`h`/`d`).
    fn code(self) -> &'static str {
        match self {
            Gran::Hour => "h",
            Gran::Day => "d",
        }
    }

    /// Format an instant as its bucket label: hour → `YYYY-MM-DDThh`, day → `YYYY-MM-DD`.
    fn label(self, t: DateTime<Utc>) -> String {
        match self {
            Gran::Hour => t.format("%Y-%m-%dT%H").to_string(),
            Gran::Day => t.format("%Y-%m-%d").to_string(),
        }
    }

    /// Expand a bucket label back to a full ISO-8601 UTC instant string.
    /// On parse failure returns the bucket unchanged (mirrors Go behaviour).
    fn label_to_iso(self, bucket: &str) -> String {
        let instant = match self {
            // "2024-01-15T10" → append ":00:00" to parse as a full datetime.
            Gran::Hour => {
                chrono::NaiveDateTime::parse_from_str(&format!("{bucket}:00:00"), "%Y-%m-%dT%H:%M:%S")
                    .map(|ndt| ndt.and_utc())
            }
            // "2024-01-15" → midnight UTC.
            Gran::Day => chrono::NaiveDate::parse_from_str(bucket, "%Y-%m-%d")
                .map(|nd| nd.and_time(chrono::NaiveTime::MIN).and_utc()),
        };
        instant.map_or_else(
            |_| bucket.to_string(),
            |dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        )
    }
}

/// Parse an RFC-3339 / ISO-8601 string (trimmed) to UTC.
fn parse_iso(s: &str) -> Result<DateTime<Utc>> {
    Ok(s.trim()
        .parse::<DateTime<chrono::FixedOffset>>()
        .map_err(|e| anyhow!("parse_iso: {}", e))?
        .with_timezone(&Utc))
}

/// Format an ISO-8601 string as the bucket label for querying.
fn bucket_label(iso: &str, gran: Gran) -> Result<String> {
    Ok(gran.label(parse_iso(iso)?))
}

/// Split a sort-key "<node_path>#<purpose>#<gran>#<bucket>" on the last 3 `#`.
/// node_path may itself contain `#`. Mirrors Python's `sk.rsplit("#", 3)`:
/// `rsplitn(4, '#')` caps at 4 pieces, so the leftmost (node_path) keeps any
/// internal `#`. Fewer than 3 `#` → fallback `(sk, "", "", "")`.
fn parse_sk(sk: &str) -> (&str, &str, &str, &str) {
    let mut it = sk.rsplitn(4, '#');
    let bucket = it.next().unwrap_or("");
    match (it.next(), it.next(), it.next()) {
        (Some(gran), Some(purpose), Some(node)) => (node, purpose, gran, bucket),
        _ => (sk, "", "", ""),
    }
}

// ── DynamoDB item shape ───────────────────────────────────────────────────────

/// A row from the rollup table. `serde_dynamo` decodes the DynamoDB item
/// directly; missing fields fall back to their defaults.
#[derive(Debug, Default, Deserialize)]
struct AggItem {
    #[serde(default)]
    sk: String,
    #[serde(default)]
    sum: f64,
    #[serde(default)]
    count: i64,
    #[serde(default)]
    unit: String,
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

/// Keep items matching `gran`, sort by (purpose, bucket), build rows.
/// Lexicographic bucket order matches Go's string comparison.
fn to_rows(items: Vec<AggItem>, level_id: &str, resolution: &str, gran: Gran) -> Vec<Row> {
    let mut kept: Vec<(String, String, AggItem)> = items
        .into_iter()
        .filter_map(|it| {
            let (_, purpose, g, bucket) = parse_sk(&it.sk);
            if g != gran.code() {
                return None;
            }
            let (purpose, bucket) = (purpose.to_string(), bucket.to_string());
            Some((purpose, bucket, it))
        })
        .collect();

    kept.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    kept.into_iter()
        .map(|(purpose, bucket, it)| Row {
            level_id: level_id.to_string(),
            purpose,
            unit: it.unit,
            resolution: resolution.to_string(),
            timestamp: gran.label_to_iso(&bucket),
            value: it.sum,
            contributor_count: it.count,
        })
        .collect()
}

// ── DynamoDB query ────────────────────────────────────────────────────────────

struct QueryParams<'a> {
    table: &'a str,
    pk: &'a str,
    sk_path: &'a str,
    gran: Gran,
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
    let req = if !purpose.is_empty() {
        // Efficient range: pk = :pk AND sk BETWEEN prefix+start AND prefix+end
        let prefix = format!("{}#{}#{}#", sk_path, purpose, gran.code());
        client
            .query()
            .table_name(table)
            .key_condition_expression("pk = :pk AND sk BETWEEN :sk_start AND :sk_end")
            .expression_attribute_values(":pk", AttributeValue::S(pk.to_string()))
            .expression_attribute_values(":sk_start", AttributeValue::S(format!("{prefix}{start_bucket}")))
            .expression_attribute_values(":sk_end", AttributeValue::S(format!("{prefix}{end_bucket}")))
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
            .expression_attribute_values(":sk_prefix", AttributeValue::S(format!("{sk_path}#")))
            .expression_attribute_values(":b_start", AttributeValue::S(start_bucket.to_string()))
            .expression_attribute_values(":b_end", AttributeValue::S(end_bucket.to_string()))
    };

    // The `.items()` paginator threads `exclusive_start_key`/`last_evaluated_key`
    // and `.collect()` gathers every page, short-circuiting on the first SDK error.
    let raw = req
        .into_paginator()
        .items()
        .send()
        .collect::<Result<Vec<_>, _>>()
        .await
        .map_err(|e| anyhow!("DynamoDB query: {:?}", e))?;

    Ok(raw
        .into_iter()
        .filter_map(|it| serde_dynamo::from_item(it).ok())
        .collect())
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

/// A `{"error": "<message>"}` body at the given status (matches the Go contract).
fn json_error(status: u16, message: impl Into<String>) -> Result<Response<Body>, Error> {
    json_response(status, HashMap::from([("error", message.into())]))
}

async fn handler(event: Request, client: &Client, table: &str) -> Result<Response<Body>, Error> {
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

    // Route: /measurements → raw_data viewer (HTML fragment, Datatilegnelse page);
    // everything else → the aggregations rollup (JSON, Resource-Insights chart).
    if uri.path().trim_end_matches('/').ends_with("/measurements") {
        return raw::handle_measurements(&qs).await;
    }

    let level_id = qs.get("level_id").cloned().unwrap_or_default();
    let resolution = qs
        .get("resolution")
        .cloned()
        .unwrap_or_else(|| "hourly".to_string());
    let purpose = qs.get("purpose").cloned().unwrap_or_default();
    let start = qs.get("start").cloned().unwrap_or_default();
    let end = qs.get("end").cloned().unwrap_or_default();

    if start.is_empty() || end.is_empty() {
        return json_error(400, "start and end are required (ISO-8601)");
    }

    let gran = Gran::from_resolution(&resolution);

    let (pk, sk_path) = match parse_node_keys(&level_id) {
        Ok(keys) => keys,
        Err(_) => {
            // node above company level (HN0/HN1) — nothing to aggregate at a single partition
            let empty: Vec<Row> = vec![];
            return json_response(200, empty);
        }
    };

    let (start_bucket, end_bucket) = match (bucket_label(&start, gran), bucket_label(&end, gran)) {
        (Ok(s), Ok(e)) => (s, e),
        _ => return json_error(400, "start/end must be ISO-8601 timestamps"),
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
        Err(e) => json_error(500, e.to_string()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let cfg = aws_config::load_from_env().await;
    let client = Client::new(&cfg);
    let table =
        std::env::var("ROLLUP_TABLE").unwrap_or_else(|_| "measurements_aggregate".to_string());

    run(service_fn(|req| handler(req, &client, &table))).await
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
        let label = bucket_label("2024-01-15T10:30:00Z", Gran::Hour).unwrap();
        assert_eq!(label, "2024-01-15T10");
    }

    #[test]
    fn test_bucket_label_day() {
        let label = bucket_label("2024-01-15T10:30:00Z", Gran::Day).unwrap();
        assert_eq!(label, "2024-01-15");
    }

    #[test]
    fn test_bucket_to_iso_hour() {
        assert_eq!(Gran::Hour.label_to_iso("2024-01-15T10"), "2024-01-15T10:00:00Z");
    }

    #[test]
    fn test_bucket_to_iso_day() {
        assert_eq!(Gran::Day.label_to_iso("2024-01-15"), "2024-01-15T00:00:00Z");
    }

    #[test]
    fn test_bucket_to_iso_invalid_returns_unchanged() {
        // On parse failure, return the bucket string unchanged (matches Go)
        assert_eq!(Gran::Hour.label_to_iso("not-a-date"), "not-a-date");
    }

    #[test]
    fn test_bucket_label_to_iso_round_trip_hour() {
        let original = "2024-06-07T14:00:00Z";
        let label = bucket_label(original, Gran::Hour).unwrap();
        assert_eq!(Gran::Hour.label_to_iso(&label), "2024-06-07T14:00:00Z");
    }

    #[test]
    fn test_bucket_label_to_iso_round_trip_day() {
        let original = "2024-06-07T00:00:00Z";
        let label = bucket_label(original, Gran::Day).unwrap();
        assert_eq!(Gran::Day.label_to_iso(&label), "2024-06-07T00:00:00Z");
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
        let rows = to_rows(items, "HN2#1", "hourly", Gran::Hour);
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
        let rows = to_rows(items, "HN2#1", "hourly", Gran::Hour);
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
        let rows = to_rows(items, "HN2#1", "hourly", Gran::Hour);
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
        let rows = to_rows(items, "HN2#10", "hourly", Gran::Hour);
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
        let rows = to_rows(vec![], "HN2#1", "hourly", Gran::Hour);
        assert!(rows.is_empty());
    }

    #[test]
    fn test_to_rows_daily() {
        let items = vec![
            make_item("HN2#5#heat#d#2024-03-02", 800.0, 10, "kWh"),
            make_item("HN2#5#heat#d#2024-03-01", 900.0, 10, "kWh"),
        ];
        let rows = to_rows(items, "HN2#5", "daily", Gran::Day);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].timestamp, "2024-03-01T00:00:00Z");
        assert_eq!(rows[1].timestamp, "2024-03-02T00:00:00Z");
    }

    // Gran::from_resolution ──────────────────────────────────────────────────────

    #[test]
    fn test_gran_from_resolution() {
        assert_eq!(Gran::from_resolution("daily"), Gran::Day);
        assert_eq!(Gran::from_resolution("hourly"), Gran::Hour);
        assert_eq!(Gran::from_resolution(""), Gran::Hour); // default
        assert_eq!(Gran::from_resolution("anything-else"), Gran::Hour);
    }
}
