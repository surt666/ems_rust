// Rust port of infra/daq/data_pipeline/lambda/aggregations-go/main.go
//
// GET /aggregations?level_id=<hierarchy path>&resolution=<hourly|daily>
//                  &purpose=<opt>&start=<ISO>&end=<ISO>
//
// Response: JSON array, one row per (purpose, bucket), sorted by purpose then time.

use anyhow::{anyhow, Result};
use aws_sdk_dynamodb::{types::AttributeValue, Client};
use chrono::{DateTime, Utc};
use futures::future::try_join_all;
use lambda_http::{http::Method, run, service_fn, Body, Error, Request, Response};
use regex::Regex;
use serde::{Deserialize, Serialize};

use api::{ApiError, ApiResponse, Cors, Format};
use utoipa::{OpenApi, ToSchema};
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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
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

/// Sum a dimension's rows across resources into one series — one `Row` per bucket,
/// `purpose` = the dimension (energy / volume). `BTreeMap` keeps buckets in order.
fn to_rows_dimension(
    items: Vec<AggItem>,
    level_id: &str,
    resolution: &str,
    gran: Gran,
    dimension: &str,
) -> Vec<Row> {
    use std::collections::BTreeMap;
    // bucket -> (summed value, summed count, unit-of-the-dimension)
    let mut by_bucket: BTreeMap<String, (f64, i64, String)> = BTreeMap::new();
    for it in items {
        let (_, _resource, g, bucket) = parse_sk(&it.sk);
        if g != gran.code() {
            continue;
        }
        let slot = by_bucket
            .entry(bucket.to_string())
            .or_insert((0.0, 0, String::new()));
        slot.0 += it.sum;
        slot.1 += it.count;
        if slot.2.is_empty() {
            slot.2 = it.unit;
        }
    }

    by_bucket
        .into_iter()
        .map(|(bucket, (sum, count, unit))| Row {
            level_id: level_id.to_string(),
            purpose: dimension.to_string(),
            unit,
            resolution: resolution.to_string(),
            timestamp: gran.label_to_iso(&bucket),
            value: sum,
            contributor_count: count,
        })
        .collect()
}

// ── DynamoDB query ────────────────────────────────────────────────────────────

/// The meter type / energy form a sensor measures (the EMS "Målertype"), keyed
/// into the rollup sort key. A node's "all" query fans out over `Resource::ALL`,
/// so adding a resource means adding a variant here; `as_str` must match the value
/// the Glue rollup writes into the sort key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resource {
    Electricity,
    DistrictHeating,
    DistrictCooling,
    Gas,
    Water,
    Heat,
}

impl Resource {
    /// Every resource — the per-resource fan-out for a node's series.
    const ALL: &'static [Resource] = &[
        Resource::Electricity,
        Resource::DistrictHeating,
        Resource::DistrictCooling,
        Resource::Gas,
        Resource::Water,
        Resource::Heat,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Resource::Electricity => "electricity",
            Resource::DistrictHeating => "district_heating",
            Resource::DistrictCooling => "district_cooling",
            Resource::Gas => "gas",
            Resource::Water => "water",
            Resource::Heat => "heat",
        }
    }
}

struct QueryParams<'a> {
    table: &'a str,
    pk: &'a str,
    sk_path: &'a str,
    gran: Gran,
    start_bucket: &'a str,
    end_bucket: &'a str,
    /// A single requested resource, or empty for "all resources".
    resource: &'a str,
}

/// Query a node's rollup rows. A specific `resource` runs one key-range query; an
/// empty `resource` ("all") fans out over every `Resource::ALL` **concurrently** —
/// each is its own `sk BETWEEN` key-range, so every query reads only its own
/// window (no `begins_with` + post-read filter, no cross-granularity reads). The
/// sort key is `<node_path>#<resource>#<gran>#<date>` with the date last, so once
/// the resource is fixed the date window is a pure key-condition range.
async fn query_node(client: &Client, p: QueryParams<'_>) -> Result<Vec<AggItem>> {
    let resources: Vec<&str> = if p.resource.is_empty() {
        Resource::ALL.iter().map(|r| r.as_str()).collect()
    } else {
        vec![p.resource]
    };

    let per_resource =
        try_join_all(resources.into_iter().map(|resource| query_one_resource(client, &p, resource)))
            .await?;
    Ok(per_resource.into_iter().flatten().collect())
}

/// One resource's window: `pk = :pk AND sk BETWEEN prefix+start AND prefix+end`,
/// where `prefix = <node_path>#<resource>#<gran>#` — a pure key-condition range.
async fn query_one_resource(
    client: &Client,
    p: &QueryParams<'_>,
    resource: &str,
) -> Result<Vec<AggItem>> {
    let prefix = format!("{}#{}#{}#", p.sk_path, resource, p.gran.code());
    // The `.items()` paginator threads `exclusive_start_key`/`last_evaluated_key`
    // and `.collect()` gathers every page, short-circuiting on the first SDK error.
    let raw = client
        .query()
        .table_name(p.table)
        .key_condition_expression("pk = :pk AND sk BETWEEN :sk_start AND :sk_end")
        .expression_attribute_values(":pk", AttributeValue::S(p.pk.to_string()))
        .expression_attribute_values(":sk_start", AttributeValue::S(format!("{prefix}{}", p.start_bucket)))
        .expression_attribute_values(":sk_end", AttributeValue::S(format!("{prefix}{}", p.end_bucket)))
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

/// A node's rows for one aggregation dimension (energy / volume), across every
/// resource, via `gsi1`: `gsi1pk = "<pk>#<dimension>"` and
/// `gsi1sk BETWEEN prefix+start AND prefix+end` where `prefix = <node_path>#<gran>#`
/// (the GSI sort key omits the resource, so this single range spans electricity +
/// heat + gas …). The caller sums per bucket.
async fn query_dimension(
    client: &Client,
    p: &QueryParams<'_>,
    dimension: &str,
) -> Result<Vec<AggItem>> {
    let gsi1pk = format!("{}#{}", p.pk, dimension);
    let prefix = format!("{}#{}#", p.sk_path, p.gran.code());
    let raw = client
        .query()
        .table_name(p.table)
        .index_name("gsi1")
        .key_condition_expression("gsi1pk = :pk AND gsi1sk BETWEEN :sk_start AND :sk_end")
        .expression_attribute_values(":pk", AttributeValue::S(gsi1pk))
        .expression_attribute_values(":sk_start", AttributeValue::S(format!("{prefix}{}", p.start_bucket)))
        .expression_attribute_values(":sk_end", AttributeValue::S(format!("{prefix}{}", p.end_bucket)))
        .into_paginator()
        .items()
        .send()
        .collect::<Result<Vec<_>, _>>()
        .await
        .map_err(|e| anyhow!("DynamoDB gsi1 query: {:?}", e))?;

    Ok(raw
        .into_iter()
        .filter_map(|it| serde_dynamo::from_item(it).ok())
        .collect())
}

// ── Lambda handler ────────────────────────────────────────────────────────────

/// A resolved route. This lambda is **read-only**, so it's the query side of
/// CQRS only (mirrors the hierarchy service's `GET /query/{action}`); there is no
/// command side — meter data is written by the Flink/Glue pipeline, not here.
enum Route {
    Query(String),
    OpenApi,
    Docs,
    NotFound,
}

/// Map `(method, path)` to a route. The gateway only forwards `GET` under
/// `/meterdata/…`, but the bare forms are accepted too (defensive / local):
///   GET `/meterdata/query/<action>` | `/query/<action>` → `Query(action)`
///   GET `/meterdata/openapi.json`   | `/openapi.json`    → `OpenApi`
///   GET `/meterdata/docs`           | `/docs`            → `Docs`
fn resolve_route(method: &Method, path: &str) -> Route {
    if *method != Method::GET {
        return Route::NotFound;
    }
    match path {
        "/meterdata/openapi.json" | "/openapi.json" => Route::OpenApi,
        "/meterdata/docs" | "/docs" => Route::Docs,
        _ => match path
            .strip_prefix("/meterdata/query/")
            .or_else(|| path.strip_prefix("/query/"))
        {
            Some(action) if !action.is_empty() => Route::Query(action.to_string()),
            _ => Route::NotFound,
        },
    }
}

/// Top-level router (CQRS query side). `?format=html|json` picks the representation:
///   GET /meterdata/query/get_aggregations → rollup rows  (json default; html table)
///   GET /meterdata/query/get_measurements → raw readings (html default; json array)
///   GET /meterdata/openapi.json           → the OpenAPI 3.1 spec for the JSON surface
///   GET /meterdata/docs                   → Swagger UI rendering of the spec
///
/// CORS is added by the API Gateway `CorsPreflight`, so we emit none here (`Cors::None`).
async fn handler(
    event: Request,
    client: &Client,
    table: &str,
    athena: &aws_sdk_athena::Client,
) -> Result<Response<Body>, Error> {
    let qs: HashMap<String, String> = event
        .uri()
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default();

    let resp = match resolve_route(event.method(), event.uri().path()) {
        Route::Query(action) => match action.as_str() {
            "get_aggregations" => {
                api::finish(handle_aggregations(client, table, &qs).await, Cors::None)
            }
            "get_measurements" => {
                api::finish(raw::handle_measurements(athena, &qs).await, Cors::None)
            }
            "get_cost" => api::finish(handle_cost(client, table, &qs).await, Cors::None),
            "get_benchmark" => api::finish(handle_benchmark(client, table, &qs).await, Cors::None),
            "get_alarms" => api::finish(handle_alarms(client, table, &qs).await, Cors::None),
            other => api::to_http(
                ApiError::not_found(format!("unknown query action {other:?}")).into_response(),
                Cors::None,
            ),
        },
        Route::OpenApi => api::to_http(openapi_response(), Cors::None),
        Route::Docs => {
            api::to_http(ApiResponse::html(200, api::swagger_ui_html("openapi.json")), Cors::None)
        }
        Route::NotFound => {
            api::to_http(ApiError::not_found("no matching route").into_response(), Cors::None)
        }
    };
    Ok(resp)
}

/// `GET /meterdata/query/get_aggregations` — query the DynamoDB rollup table.
/// `?format=json` (default) returns `[Row]`; `?format=html` returns a `<tr>` table fragment.
#[utoipa::path(
    get,
    path = "/meterdata/query/get_aggregations",
    tag = "aggregations",
    params(
        ("level_id" = String, Query, description = "Hierarchy node path (…|HN2#..|HN3#..)"),
        ("resolution" = Option<String>, Query, description = "hourly (default) | daily"),
        ("resource" = Option<String>, Query, description = "Filter to one resource (electricity, water, …); empty = all"),
        ("dimension" = Option<String>, Query, description = "Aggregate across resources in a dimension (energy | volume) → one summed series; overrides resource"),
        ("start" = String, Query, description = "ISO-8601 start (required)"),
        ("end" = String, Query, description = "ISO-8601 end (required)"),
        ("format" = Option<String>, Query, description = "json (default) | html"),
    ),
    responses(
        (status = 200, description = "Rollup rows as [Row] (json) or an HTML table fragment (html)", body = Vec<Row>),
        (status = 400, description = "Missing/invalid start or end", body = api::ErrorResponse),
        (status = 500, description = "DynamoDB query failed", body = api::ErrorResponse),
    ),
)]
async fn handle_aggregations(
    client: &Client,
    table: &str,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Json);

    let level_id = qs.get("level_id").cloned().unwrap_or_default();
    let resolution = qs
        .get("resolution")
        .cloned()
        .unwrap_or_else(|| "hourly".to_string());
    // A single resource filter (electricity / water / …); empty = all resources.
    // Accept `resource=` (new) or `purpose=` (legacy) during the transition.
    let resource = qs
        .get("resource")
        .or_else(|| qs.get("purpose"))
        .cloned()
        .unwrap_or_default();
    // An aggregation *dimension* (energy / volume): when set, sum across every
    // resource in that dimension via the GSI → one series (a node's total energy).
    // Mutually exclusive with `resource`; `dimension` wins.
    let dimension = qs.get("dimension").cloned().unwrap_or_default();
    let start = qs.get("start").cloned().unwrap_or_default();
    let end = qs.get("end").cloned().unwrap_or_default();

    if start.is_empty() || end.is_empty() {
        return Err(ApiError::bad_request("start and end are required (ISO-8601)"));
    }

    let gran = Gran::from_resolution(&resolution);

    let (pk, sk_path) = match parse_node_keys(&level_id) {
        Ok(keys) => keys,
        // node above company level (HN0/HN1) — nothing to aggregate at a single partition
        Err(_) => return Ok(rows_response(&[], format)),
    };

    let (start_bucket, end_bucket) = match (bucket_label(&start, gran), bucket_label(&end, gran)) {
        (Ok(s), Ok(e)) => (s, e),
        _ => return Err(ApiError::bad_request("start/end must be ISO-8601 timestamps")),
    };

    let params = QueryParams {
        table,
        pk: &pk,
        sk_path: &sk_path,
        gran,
        start_bucket: &start_bucket,
        end_bucket: &end_bucket,
        resource: &resource,
    };

    let rows = if dimension.is_empty() {
        match query_node(client, params).await {
            Ok(items) => to_rows(items, &level_id, &resolution, gran),
            Err(e) => return Err(ApiError::internal(e.to_string())),
        }
    } else {
        match query_dimension(client, &params, &dimension).await {
            Ok(items) => to_rows_dimension(items, &level_id, &resolution, gran, &dimension),
            Err(e) => return Err(ApiError::internal(e.to_string())),
        }
    };
    Ok(rows_response(&rows, format))
}

/// Render rollup rows in the requested representation.
fn rows_response(rows: &[Row], format: Format) -> ApiResponse {
    match format {
        Format::Json => ApiResponse::json(&rows),
        Format::Html => ApiResponse::html(200, rows_to_html(rows)),
    }
}

/// A `<tr>` table fragment of rollup rows (purpose / time / value / unit / count).
fn rows_to_html(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "<tr><td colspan=\"5\" class=\"muted\">Ingen data.</td></tr>".to_string();
    }
    let mut out = String::new();
    for r in rows {
        out.push_str(&format!(
            "<tr><td>{}</td><td class=\"mono\">{}</td><td class=\"mono\" style=\"text-align:right\">{:.3}</td><td>{}</td><td class=\"mono\">{}</td></tr>",
            esc(&r.purpose),
            esc(&r.timestamp),
            r.value,
            esc(&r.unit),
            r.contributor_count,
        ));
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ── Derived read models (cost / benchmark / alarms) ────────────────────────────
//
// All three are pure functions of the existing rollup (`measurements_aggregate`)
// plus a small config (tariffs, spike factor). No new data source, no new lambda
// — they ride the same CQRS query surface as get_aggregations. Defaults are
// representative Danish unit prices; override per deploy if a real price model
// lands.

/// Fetch per-(resource, bucket) consumption rows for a node window — the shared
/// core of get_aggregations, reused by the derived models. A `level_id` above
/// company level (no HN2) yields no rows rather than an error.
async fn fetch_node_rows(
    client: &Client,
    table: &str,
    level_id: &str,
    resolution: &str,
    start: &str,
    end: &str,
) -> Result<Vec<Row>, ApiError> {
    let gran = Gran::from_resolution(resolution);
    let (pk, sk_path) = match parse_node_keys(level_id) {
        Ok(keys) => keys,
        Err(_) => return Ok(Vec::new()),
    };
    let (start_bucket, end_bucket) = match (bucket_label(start, gran), bucket_label(end, gran)) {
        (Ok(s), Ok(e)) => (s, e),
        _ => return Err(ApiError::bad_request("start/end must be ISO-8601 timestamps")),
    };
    let params = QueryParams {
        table,
        pk: &pk,
        sk_path: &sk_path,
        gran,
        start_bucket: &start_bucket,
        end_bucket: &end_bucket,
        resource: "",
    };
    let items = query_node(client, params)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(to_rows(items, level_id, resolution, gran))
}

/// Representative unit price (DKK) for a resource, given the rollup's stored unit.
/// Energy rolls up in Wh (tariff is quoted per kWh → divide by 1000); volumes in m³.
fn tariff_dkk_per_unit(resource: &str, unit: &str) -> f64 {
    let u = unit.to_ascii_lowercase();
    let (per_kwh, per_m3) = match resource {
        "electricity" => (2.50, 0.0),
        "district_heating" | "heat" => (0.90, 0.0),
        "district_cooling" => (0.50, 0.0),
        "gas" => (0.0, 8.0),
        "water" => (0.0, 50.0),
        _ => (0.0, 0.0),
    };
    if u.contains("kwh") {
        per_kwh
    } else if u.contains("wh") {
        per_kwh / 1000.0
    } else if is_cubic_metre(&u) {
        per_m3
    } else {
        0.0
    }
}

/// True for the various ways a cubic-metre unit is written in the data
/// (`m3`, `m³`, `m^3`). The rollup carries whatever the raw meter reported.
fn is_cubic_metre(u: &str) -> bool {
    u.contains("m3") || u.contains("m³") || u.contains("m^3")
}

/// Energy expressed in kWh (rollup stores Wh); non-energy units contribute 0.
fn energy_kwh(value: f64, unit: &str) -> f64 {
    let u = unit.to_ascii_lowercase();
    if u.contains("kwh") {
        value
    } else if u.contains("wh") {
        value / 1000.0
    } else {
        0.0
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// `GET /meterdata/query/get_cost` — consumption × per-resource tariff, one row
/// per (resource, bucket) with the value in DKK. Same shape as get_aggregations;
/// the client sums across resources for a node total.
#[utoipa::path(
    get,
    path = "/meterdata/query/get_cost",
    tag = "aggregations",
    params(
        ("level_id" = String, Query, description = "Hierarchy node path (…|HN2#..)"),
        ("resolution" = Option<String>, Query, description = "hourly | daily (default daily)"),
        ("start" = String, Query, description = "ISO-8601 start (required)"),
        ("end" = String, Query, description = "ISO-8601 end (required)"),
        ("format" = Option<String>, Query, description = "json (default) | html"),
    ),
    responses(
        (status = 200, description = "Cost rows ([Row], value in DKK)", body = Vec<Row>),
        (status = 400, description = "Missing/invalid start or end", body = api::ErrorResponse),
    ),
)]
async fn handle_cost(
    client: &Client,
    table: &str,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Json);
    let level_id = qs.get("level_id").cloned().unwrap_or_default();
    let resolution = qs.get("resolution").cloned().unwrap_or_else(|| "daily".to_string());
    let start = qs.get("start").cloned().unwrap_or_default();
    let end = qs.get("end").cloned().unwrap_or_default();
    if start.is_empty() || end.is_empty() {
        return Err(ApiError::bad_request("start and end are required (ISO-8601)"));
    }

    let rows = fetch_node_rows(client, table, &level_id, &resolution, &start, &end).await?;
    let cost_rows: Vec<Row> = rows
        .into_iter()
        .map(|r| {
            let cost = round2(r.value * tariff_dkk_per_unit(&r.purpose, &r.unit));
            Row { unit: "DKK".to_string(), value: cost, ..r }
        })
        .collect();
    Ok(rows_response(&cost_rows, format))
}

/// A node's consumption + cost this period vs the preceding equal-length period.
#[derive(Debug, Serialize, ToSchema)]
struct Benchmark {
    level_id: String,
    period_days: f64,
    energy_kwh: f64,
    energy_prev_kwh: f64,
    energy_deviation_pct: f64,
    cost_dkk: f64,
    cost_prev_dkk: f64,
    cost_deviation_pct: f64,
}

fn pct_change(cur: f64, prev: f64) -> f64 {
    if prev.abs() < f64::EPSILON {
        0.0
    } else {
        round2((cur - prev) / prev * 100.0)
    }
}

/// Sum a window's rows into (energy kWh, cost DKK).
fn energy_and_cost(rows: &[Row]) -> (f64, f64) {
    let mut e = 0.0;
    let mut c = 0.0;
    for r in rows {
        e += energy_kwh(r.value, &r.unit);
        c += r.value * tariff_dkk_per_unit(&r.purpose, &r.unit);
    }
    (e, c)
}

/// `GET /meterdata/query/get_benchmark` — this node vs its own preceding period.
/// Returns current/previous energy + cost and the % deviation (lower = better).
#[utoipa::path(
    get,
    path = "/meterdata/query/get_benchmark",
    tag = "aggregations",
    params(
        ("level_id" = String, Query, description = "Hierarchy node path (…|HN2#..)"),
        ("resolution" = Option<String>, Query, description = "hourly | daily (default daily)"),
        ("start" = String, Query, description = "ISO-8601 start (required)"),
        ("end" = String, Query, description = "ISO-8601 end (required)"),
    ),
    responses(
        (status = 200, description = "Benchmark vs the preceding equal period", body = Benchmark),
        (status = 400, description = "Missing/invalid start or end", body = api::ErrorResponse),
    ),
)]
async fn handle_benchmark(
    client: &Client,
    table: &str,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let level_id = qs.get("level_id").cloned().unwrap_or_default();
    let resolution = qs.get("resolution").cloned().unwrap_or_else(|| "daily".to_string());
    let start = qs.get("start").cloned().unwrap_or_default();
    let end = qs.get("end").cloned().unwrap_or_default();
    if start.is_empty() || end.is_empty() {
        return Err(ApiError::bad_request("start and end are required (ISO-8601)"));
    }
    let s = parse_iso(&start).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let e = parse_iso(&end).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let dur = e - s;
    let prev_start = (s - dur).to_rfc3339();
    let prev_end = s.to_rfc3339();

    let cur = fetch_node_rows(client, table, &level_id, &resolution, &start, &end).await?;
    let prev = fetch_node_rows(client, table, &level_id, &resolution, &prev_start, &prev_end).await?;
    let (e_cur, c_cur) = energy_and_cost(&cur);
    let (e_prev, c_prev) = energy_and_cost(&prev);

    let bench = Benchmark {
        level_id,
        period_days: round2(dur.num_seconds() as f64 / 86_400.0),
        energy_kwh: round2(e_cur),
        energy_prev_kwh: round2(e_prev),
        energy_deviation_pct: pct_change(e_cur, e_prev),
        cost_dkk: round2(c_cur),
        cost_prev_dkk: round2(c_prev),
        cost_deviation_pct: pct_change(c_cur, c_prev),
    };
    Ok(ApiResponse::json(&bench))
}

/// One flagged consumption anomaly (a bucket far above the resource's median).
#[derive(Debug, Serialize, ToSchema)]
struct Alarm {
    resource: String,
    timestamp: String,
    value: f64,
    median: f64,
    ratio: f64,
    unit: String,
}

#[derive(Debug, Serialize, ToSchema)]
struct AlarmsResponse {
    level_id: String,
    count: usize,
    alarms: Vec<Alarm>,
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Spike-detection over per-resource daily series: flag buckets above
/// `spike_factor × median` (median > 0). Pure given the rows + factor.
fn detect_spikes(rows: &[Row], spike_factor: f64) -> Vec<Alarm> {
    use std::collections::BTreeMap;
    let mut by_resource: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        by_resource.entry(r.purpose.clone()).or_default().push(r);
    }
    let mut alarms = Vec::new();
    for (resource, rs) in by_resource {
        let med = median(&rs.iter().map(|r| r.value).collect::<Vec<_>>());
        if med <= 0.0 {
            continue;
        }
        for r in rs {
            if r.value > spike_factor * med {
                alarms.push(Alarm {
                    resource: resource.clone(),
                    timestamp: r.timestamp.clone(),
                    value: round2(r.value),
                    median: round2(med),
                    ratio: round2(r.value / med),
                    unit: r.unit.clone(),
                });
            }
        }
    }
    alarms.sort_by(|a, b| b.ratio.partial_cmp(&a.ratio).unwrap_or(std::cmp::Ordering::Equal));
    alarms
}

/// `GET /meterdata/query/get_alarms` — derived consumption-spike alarms for a node
/// (buckets above `spike_factor × resource median`). A real, config-driven v1 over
/// the rollup until a dedicated alarm engine exists.
#[utoipa::path(
    get,
    path = "/meterdata/query/get_alarms",
    tag = "aggregations",
    params(
        ("level_id" = String, Query, description = "Hierarchy node path (…|HN2#..)"),
        ("resolution" = Option<String>, Query, description = "hourly | daily (default daily)"),
        ("start" = String, Query, description = "ISO-8601 start (required)"),
        ("end" = String, Query, description = "ISO-8601 end (required)"),
        ("spike_factor" = Option<f64>, Query, description = "Multiple of the median that counts as a spike (default 2.0)"),
    ),
    responses(
        (status = 200, description = "Triggered spike alarms", body = AlarmsResponse),
        (status = 400, description = "Missing/invalid start or end", body = api::ErrorResponse),
    ),
)]
async fn handle_alarms(
    client: &Client,
    table: &str,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let level_id = qs.get("level_id").cloned().unwrap_or_default();
    let resolution = qs.get("resolution").cloned().unwrap_or_else(|| "daily".to_string());
    let start = qs.get("start").cloned().unwrap_or_default();
    let end = qs.get("end").cloned().unwrap_or_default();
    if start.is_empty() || end.is_empty() {
        return Err(ApiError::bad_request("start and end are required (ISO-8601)"));
    }
    let spike_factor = qs
        .get("spike_factor")
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|f| *f > 1.0)
        .unwrap_or(2.0);

    let rows = fetch_node_rows(client, table, &level_id, &resolution, &start, &end).await?;
    let alarms = detect_spikes(&rows, spike_factor);
    Ok(ApiResponse::json(&AlarmsResponse {
        level_id,
        count: alarms.len(),
        alarms,
    }))
}

/// The generated OpenAPI 3.1 document for the JSON surface of this lambda.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "EMS Measurements & Aggregations API",
        description = "Hierarchy consumption rollups (Resource-Insights chart, over DynamoDB) and raw \
                       meter readings (Datatilegnelse, over Athena). Data routes accept ?format=html|json.",
        version = "0.1.0",
    ),
    paths(handle_aggregations, handle_cost, handle_benchmark, handle_alarms, raw::handle_measurements),
    components(schemas(Row, Benchmark, Alarm, AlarmsResponse, raw::Measurement, api::ErrorResponse, api::ErrorDetail)),
    tags(
        (name = "aggregations", description = "Hierarchy consumption rollup (Resource-Insights chart)"),
        (name = "measurements", description = "Raw meter readings (Datatilegnelse)"),
    ),
)]
struct ApiDoc;

fn openapi_response() -> ApiResponse {
    let json = ApiDoc::openapi().to_json().unwrap_or_else(|_| "{}".to_string());
    ApiResponse::json_raw(200, json)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // `cargo run -p aggregations -- --openapi` prints the spec and exits (the
    // Lambda runtime never passes args, so this is inert in production).
    if std::env::args().any(|a| a == "--openapi") {
        println!("{}", ApiDoc::openapi().to_pretty_json().expect("openapi"));
        return Ok(());
    }

    let cfg = aws_config::load_from_env().await;
    let client = Client::new(&cfg);
    let athena = aws_sdk_athena::Client::new(&cfg);
    let table =
        std::env::var("ROLLUP_TABLE").unwrap_or_else(|_| "measurements_aggregate".to_string());

    run(service_fn(|req| handler(req, &client, &table, &athena))).await
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // OpenAPI ───────────────────────────────────────────────────────────────────

    #[test]
    fn openapi_doc_generates_with_both_paths() {
        // utoipa wires paths/schemas at call time, so generating the doc is the
        // real validation that the #[utoipa::path] annotations resolve.
        let doc = ApiDoc::openapi().to_json().expect("openapi serializes");
        assert!(doc.contains("/meterdata/query/get_aggregations"), "missing get_aggregations path");
        assert!(doc.contains("/meterdata/query/get_measurements"), "missing get_measurements path");
        assert!(doc.contains("/meterdata/query/get_cost"), "missing get_cost path");
        assert!(doc.contains("/meterdata/query/get_benchmark"), "missing get_benchmark path");
        assert!(doc.contains("/meterdata/query/get_alarms"), "missing get_alarms path");
        assert!(doc.contains("Measurement"), "missing Measurement schema");
        assert!(doc.contains("ErrorResponse"), "missing ErrorResponse schema");
    }

    // Derived models ─────────────────────────────────────────────────────────────

    fn row(purpose: &str, unit: &str, value: f64, ts: &str) -> Row {
        Row {
            level_id: "HN2#1".into(),
            purpose: purpose.into(),
            unit: unit.into(),
            resolution: "daily".into(),
            timestamp: ts.into(),
            value,
            contributor_count: 1,
        }
    }

    #[test]
    fn tariff_is_unit_aware() {
        // electricity quoted per kWh; rollup stores Wh → 1/1000 of the kWh price.
        assert!((tariff_dkk_per_unit("electricity", "kWh") - 2.50).abs() < 1e-9);
        assert!((tariff_dkk_per_unit("electricity", "Wh") - 0.0025).abs() < 1e-9);
        assert!((tariff_dkk_per_unit("water", "m3") - 50.0).abs() < 1e-9);
        // The rollup writes volumes as "m^3" — must price the same as "m3"/"m³".
        assert!((tariff_dkk_per_unit("water", "m^3") - 50.0).abs() < 1e-9);
        assert!((tariff_dkk_per_unit("water", "m³") - 50.0).abs() < 1e-9);
        assert_eq!(tariff_dkk_per_unit("electricity", "°C"), 0.0, "non-priced unit → 0");
        assert_eq!(tariff_dkk_per_unit("unknown", "kWh"), 0.0, "unknown resource → 0");
    }

    #[test]
    fn energy_kwh_normalizes_wh() {
        assert!((energy_kwh(1500.0, "Wh") - 1.5).abs() < 1e-9);
        assert!((energy_kwh(2.0, "kWh") - 2.0).abs() < 1e-9);
        assert_eq!(energy_kwh(10.0, "m3"), 0.0, "volume isn't energy");
    }

    #[test]
    fn pct_change_guards_zero_baseline() {
        assert_eq!(pct_change(10.0, 0.0), 0.0);
        assert_eq!(pct_change(110.0, 100.0), 10.0);
        assert_eq!(pct_change(90.0, 100.0), -10.0);
    }

    #[test]
    fn median_odd_even() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn detect_spikes_flags_only_above_factor() {
        // electricity: median of [10,10,10,40] = 10; 40 > 2×10 → one spike.
        // water: flat 5,5,5 → median 5, nothing above 2×.
        let rows = vec![
            row("electricity", "Wh", 10.0, "d1"),
            row("electricity", "Wh", 10.0, "d2"),
            row("electricity", "Wh", 10.0, "d3"),
            row("electricity", "Wh", 40.0, "d4"),
            row("water", "m3", 5.0, "d1"),
            row("water", "m3", 5.0, "d2"),
            row("water", "m3", 5.0, "d3"),
        ];
        let alarms = detect_spikes(&rows, 2.0);
        assert_eq!(alarms.len(), 1, "only the 40 electricity bucket spikes");
        assert_eq!(alarms[0].resource, "electricity");
        assert_eq!(alarms[0].timestamp, "d4");
        assert!((alarms[0].ratio - 4.0).abs() < 1e-9);
    }

    #[test]
    fn detect_spikes_ignores_zero_median_resource() {
        // median([0,0,0,1]) = 0 → resource skipped despite the non-zero bucket.
        let rows = vec![
            row("gas", "m3", 0.0, "d1"),
            row("gas", "m3", 0.0, "d2"),
            row("gas", "m3", 0.0, "d3"),
            row("gas", "m3", 1.0, "d4"),
        ];
        assert!(detect_spikes(&rows, 2.0).is_empty(), "median 0 → skip resource");
    }

    // Purpose fan-out ────────────────────────────────────────────────────────────

    #[test]
    fn resource_all_is_authoritative() {
        // The all-resources query fans out over exactly these — must be non-empty,
        // and each `as_str` must match what the Glue rollup writes into the SK.
        assert!(!Resource::ALL.is_empty(), "fan-out needs at least one resource");
        assert_eq!(Resource::Electricity.as_str(), "electricity");
        assert_eq!(Resource::Water.as_str(), "water");
    }

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
    fn test_to_rows_dimension_sums_across_resources_per_bucket() {
        // Two resources in the same bucket sum into one energy row; a second
        // bucket stays separate; wrong-granularity rows are dropped.
        let items = vec![
            make_item("HN2#1#electricity#h#2024-01-01T10", 100.0, 5, "kWh"),
            make_item("HN2#1#heat#h#2024-01-01T10", 40.0, 2, "kWh"),
            make_item("HN2#1#electricity#h#2024-01-01T11", 20.0, 1, "kWh"),
            make_item("HN2#1#electricity#d#2024-01-01", 999.0, 9, "kWh"), // wrong gran
        ];
        let rows = to_rows_dimension(items, "HN2#1", "hourly", Gran::Hour, "energy");
        assert_eq!(rows.len(), 2, "two hourly buckets");
        assert!(rows.iter().all(|r| r.purpose == "energy"));
        // sorted by bucket; first bucket = electricity + heat
        assert_eq!(rows[0].timestamp, "2024-01-01T10:00:00Z");
        assert_eq!(rows[0].value, 140.0);
        assert_eq!(rows[0].contributor_count, 7);
        assert_eq!(rows[1].value, 20.0);
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
