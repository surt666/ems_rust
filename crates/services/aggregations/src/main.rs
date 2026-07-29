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

mod liveness;
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
    const fn code(self) -> &'static str {
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

/// Split a sort-key "<node_path>#<energy_type>#<purpose>#<gran>#<bucket>" on the
/// last 4 `#`. node_path may itself contain `#`, so `rsplitn(5, '#')` caps at 5
/// pieces and the leftmost (node_path) keeps any internal `#`.
/// Fewer than 4 `#` → fallback `(sk, "", "", "", "")`.
fn parse_sk(sk: &str) -> (&str, &str, &str, &str, &str) {
    let mut it = sk.rsplitn(5, '#');
    let bucket = it.next().unwrap_or("");
    match (it.next(), it.next(), it.next(), it.next()) {
        (Some(gran), Some(purpose), Some(energy_type), Some(node)) => {
            (node, energy_type, purpose, gran, bucket)
        }
        _ => (sk, "", "", "", ""),
    }
}

/// `<node_path>#<energy_type>#<purpose>#<gran>#` — the caller appends the bucket to
/// form a pure `BETWEEN` range. An empty `purpose` means `total`, so the default
/// query keeps returning a node's whole consumption rather than nothing.
fn sk_prefix(sk_path: &str, energy_type: &str, purpose: &str, gran: Gran) -> String {
    let purpose = if purpose.is_empty() { "total" } else { purpose };
    format!("{sk_path}#{energy_type}#{purpose}#{}#", gran.code())
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
    /// What the sensor measures (electricity / water / district_heating …).
    energy_type: String,
    /// What the energy is spent on (total / lighting / space_heating …).
    purpose: String,
    unit: String,
    resolution: String,
    timestamp: String,
    value: f64,
    contributor_count: i64,
}

/// Keep items matching `gran`, sort by (energy_type, purpose, bucket), build rows.
/// Lexicographic bucket order matches Go's string comparison.
fn to_rows(items: Vec<AggItem>, level_id: &str, resolution: &str, gran: Gran) -> Vec<Row> {
    let mut kept: Vec<(String, String, String, AggItem)> = items
        .into_iter()
        .filter_map(|it| {
            let (_, energy_type, purpose, g, bucket) = parse_sk(&it.sk);
            if g != gran.code() {
                return None;
            }
            Some((energy_type.to_string(), purpose.to_string(), bucket.to_string(), it))
        })
        .collect();

    kept.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.cmp(&b.2)));

    kept.into_iter()
        .map(|(energy_type, purpose, bucket, it)| Row {
            level_id: level_id.to_string(),
            energy_type,
            purpose,
            unit: it.unit,
            resolution: resolution.to_string(),
            timestamp: gran.label_to_iso(&bucket),
            value: it.sum,
            contributor_count: it.count,
        })
        .collect()
}

/// Sum a dimension's rows across energy types into one series — one `Row` per bucket,
/// `energy_type` = the dimension (energy / volume), `purpose` = the one being queried.
/// `BTreeMap` keeps buckets in order.
fn to_rows_dimension(
    items: Vec<AggItem>,
    level_id: &str,
    resolution: &str,
    gran: Gran,
    dimension: &str,
    purpose: &str,
) -> Vec<Row> {
    use std::collections::BTreeMap;
    // bucket -> (summed value, summed count, unit-of-the-dimension)
    let mut by_bucket: BTreeMap<String, (f64, i64, String)> = BTreeMap::new();
    for it in items {
        let (_, _energy_type, _purpose, g, bucket) = parse_sk(&it.sk);
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
            energy_type: dimension.to_string(),
            purpose: if purpose.is_empty() { "total" } else { purpose }.to_string(),
            unit,
            resolution: resolution.to_string(),
            timestamp: gran.label_to_iso(&bucket),
            value: sum,
            contributor_count: count,
        })
        .collect()
}

// ── DynamoDB query ────────────────────────────────────────────────────────────

// The resource a sensor measures (the EMS "Målertype") is shared domain
// vocabulary — `model::domain::values::EnergyType`. The meter-data context keys
// rollup rows by `EnergyType::as_str` and fans a node's "all" query out over
// `EnergyType::all()`; the hierarchy context writes the same token into the
// sensor's `purpose`. Same ubiquitous language, so the type is shared, not
// re-declared here.
use model::domain::values::{EnergyType, Purpose};

struct QueryParams<'a> {
    table: &'a str,
    pk: &'a str,
    sk_path: &'a str,
    gran: Gran,
    start_bucket: &'a str,
    end_bucket: &'a str,
    /// A single requested energy type, or empty for "all energy types".
    energy_type: &'a str,
    /// A single requested purpose, or empty for `total` (see `sk_prefix`).
    purpose: &'a str,
}

/// Query a node's rollup rows. A specific `energy_type` runs one key-range query; an
/// empty `energy_type` ("all") fans out over every `EnergyType::ALL` **concurrently** —
/// each is its own `sk BETWEEN` key-range, so every query reads only its own
/// window (no `begins_with` + post-read filter, no cross-granularity reads). The
/// sort key is `<node_path>#<energy_type>#<purpose>#<gran>#<date>` with the date
/// last, so once energy type and purpose are fixed the date window is a pure
/// key-condition range.
async fn query_node(client: &Client, p: QueryParams<'_>) -> Result<Vec<AggItem>> {
    let energy_types: Vec<&str> = if p.energy_type.is_empty() {
        EnergyType::all().map(|r| r.as_str()).collect()
    } else {
        vec![p.energy_type]
    };

    let per_type = try_join_all(
        energy_types.into_iter().map(|et| query_one_energy_type(client, &p, et)),
    )
    .await?;
    Ok(per_type.into_iter().flatten().collect())
}

/// One energy type's window: `pk = :pk AND sk BETWEEN prefix+start AND prefix+end`,
/// where `prefix = <node_path>#<energy_type>#<purpose>#<gran>#` — a pure
/// key-condition range.
async fn query_one_energy_type(
    client: &Client,
    p: &QueryParams<'_>,
    energy_type: &str,
) -> Result<Vec<AggItem>> {
    let prefix = sk_prefix(p.sk_path, energy_type, p.purpose, p.gran);
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
/// energy type, via `gsi1`: `gsi1pk = "<pk>#<dimension>#<purpose>"` and
/// `gsi1sk BETWEEN prefix+start AND prefix+end` where `prefix = <node_path>#<gran>#`
/// (the GSI sort key omits the energy type, so this single range spans electricity +
/// heat + gas …). The purpose is in the partition key, not the sort key, so an
/// "all energy" query cannot sum a node's total together with its own breakdown.
/// The caller sums per bucket.
async fn query_dimension(
    client: &Client,
    p: &QueryParams<'_>,
    dimension: &str,
) -> Result<Vec<AggItem>> {
    let purpose = if p.purpose.is_empty() { "total" } else { p.purpose };
    let gsi1pk = format!("{}#{}#{}", p.pk, dimension, purpose);
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
                // Inject the Athena reader (model::repository::measurements) — the
                // handler owns only the HTTP/HTML shape, not the data source.
                let read = |q| model::repository::measurements::query(athena, q);
                api::finish(raw::handle_measurements(read, &qs).await, Cors::None)
            }
            "get_purpose_split" => {
                api::finish(handle_purpose_split(client, table, &qs).await, Cors::None)
            }
            "get_cost" => api::finish(handle_cost(client, table, &qs).await, Cors::None),
            "get_emissions" => api::finish(handle_emissions(client, table, &qs).await, Cors::None),
            "get_benchmark" => api::finish(handle_benchmark(client, table, &qs).await, Cors::None),
            "get_alarms" => api::finish(handle_alarms(client, table, &qs).await, Cors::None),
            "get_liveness" => {
                api::finish(liveness::handle_liveness(athena, &qs).await, Cors::None)
            }
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
        ("energy_type" = Option<String>, Query, description = "Filter to one energy type (electricity, water, …); empty = all. `resource` is accepted as an alias"),
        ("purpose" = Option<String>, Query, description = "Filter to one purpose (total, lighting, space_heating, …); empty = total"),
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
    // A single energy-type filter (electricity / water / …); empty = all types.
    //
    // `purpose=` used to be a legacy alias for this and is NOT accepted any more:
    // purpose is now a real, orthogonal axis (total / lighting / space_heating …),
    // so honouring the alias would silently filter energy types by a purpose name
    // and return nothing. `energy_type=` is the name; `resource=` stays accepted
    // because the frontend still sends it.
    let energy_type = qs
        .get("energy_type")
        .or_else(|| qs.get("resource"))
        .cloned()
        .unwrap_or_default();
    // What the energy is spent on. Empty = `total`, so the default answer is still
    // a node's whole consumption.
    let purpose = qs.get("purpose").cloned().unwrap_or_default();
    // An aggregation *dimension* (energy / volume): when set, sum across every
    // energy type in that dimension via the GSI → one series (a node's total energy).
    // Mutually exclusive with `energy_type`; `dimension` wins.
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
        Err(_) => return Ok(rows_response(&[], format, Render::Table)),
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
        energy_type: &energy_type,
        purpose: &purpose,
    };

    let render = Render::resolve(qs.get("view").map(String::as_str));
    let rows = if dimension.is_empty() {
        match query_node(client, params).await {
            Ok(items) => to_rows(items, &level_id, &resolution, gran),
            Err(e) => return Err(ApiError::internal(e.to_string())),
        }
    } else {
        match query_dimension(client, &params, &dimension).await {
            Ok(items) => {
                to_rows_dimension(items, &level_id, &resolution, gran, &dimension, &purpose)
            }
            Err(e) => return Err(ApiError::internal(e.to_string())),
        }
    };
    Ok(rows_response(&rows, format, render))
}

/// Which HTML rendering of the rows to produce.
///
/// This is a per-route concern, not a media type: `api::Format` answers
/// "JSON or HTML", and every one of these is HTML. Keeping it local means the
/// routes with a single rendering (liveness, raw measurements, benchmark,
/// alarms) never have to mention the others.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Render {
    /// A `<tr>` table — the default.
    #[default]
    Table,
    /// A chart config for `mountDeclaredCharts`.
    Chart,
    /// One card per energy type, each embedding its own sparkline.
    Cards,
}

impl Render {
    fn resolve(raw: Option<&str>) -> Render {
        match raw.map(str::trim) {
            Some("chart") => Render::Chart,
            Some("cards") => Render::Cards,
            _ => Render::Table,
        }
    }
}

/// Render rollup rows in the requested representation.
fn rows_response(rows: &[Row], format: Format, render: Render) -> ApiResponse {
    match (format, render) {
        (Format::Json, _) => ApiResponse::json(&rows),
        (Format::Html, Render::Table) => ApiResponse::html(200, rows_to_html(rows)),
        (Format::Html, Render::Chart) => {
            ApiResponse::html(200, rows_to_chart(rows, ChartKind::PerEnergyType))
        }
        (Format::Html, Render::Cards) => ApiResponse::html(200, rows_to_cards(rows)),
    }
}

/// `?format=chart&view=cards` — one card per energy type: period total, daily
/// average, and a sparkline.
///
/// Each card embeds its own `data-chart` block, so a single fragment mounts N
/// charts. The totals are computed here for the same reason the pivot is: it is
/// the last arithmetic the browser was doing to data it had fetched as JSON.
fn rows_to_cards(rows: &[Row]) -> String {
    use std::collections::BTreeMap;

    if rows.is_empty() {
        return "<p class=\"muted chart-state\">Ingen forbrugsdata for perioden.</p>".to_string();
    }

    // Energy carriers first, then volumes — a stable, human order.
    const ORDER: [&str; 6] = [
        "electricity", "district_heating", "heat", "gas", "district_cooling", "water",
    ];

    let mut by_type: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        by_type.entry(r.energy_type.as_str()).or_default().push(r);
    }

    let mut types: Vec<&str> = by_type.keys().copied().collect();
    types.sort_by_key(|t| ORDER.iter().position(|o| o == t).unwrap_or(usize::MAX));

    let mut out = String::new();
    for t in types {
        let mut rs = by_type.remove(t).unwrap_or_default();
        rs.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        let total: f64 = rs.iter().map(|r| r.value).sum();
        let avg = if rs.is_empty() { 0.0 } else { total / rs.len() as f64 };
        let unit = rs.iter().find(|r| !r.unit.is_empty()).map_or("", |r| r.unit.as_str());
        let decimals = if total < 100.0 { 2 } else { 0 };

        let spark = chart_block(&serde_json::json!({
            "categories": rs.iter()
                .map(|r| r.timestamp.get(..10).unwrap_or(&r.timestamp))
                .collect::<Vec<_>>(),
            "series": [{
                "name": energy_label(t),
                "key": t,
                "type": "bar",
                "data": rs.iter().map(|r| r.value).collect::<Vec<_>>(),
            }],
            "unit": unit,
            "zoom": false,
        }));

        out.push_str(&format!(
            "<div class=\"card rc-card\" data-type=\"{ty}\" role=\"button\" tabindex=\"0\" \
               title=\"Klik for detaljeret analyse\">\
               <div class=\"rc-card__head\"><span class=\"rc-dot rc-dot--{ty}\"></span>\
                 <strong>{label}</strong></div>\
               <div class=\"rc-card__metrics\">\
                 <div><span class=\"muted\">Periode</span><strong class=\"mono\">{total} {unit}</strong></div>\
                 <div><span class=\"muted\">Gnm./dag</span><strong class=\"mono\">{avg} {unit}</strong></div>\
               </div>\
               <div class=\"rc-spark\">{spark}</div>\
             </div>",
            ty = esc(t),
            label = esc(energy_label(t)),
            total = da(total, decimals),
            avg = da(avg, decimals),
            unit = esc(unit),
        ));
    }
    out
}

/// How to fold rows into series, and how to draw them.
///
/// The series type is per-widget and matches what each one looked like before the
/// rendering moved server-side: only the consumption trend was ever a line;
/// cost, emissions and the per-carrier sparklines were columns.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ChartKind {
    /// One line per energy type — the consumption trend.
    PerEnergyType,
    /// One column per energy type — emissions.
    PerEnergyTypeBars,
    /// Everything summed per bucket into a single column series — cost, where
    /// separate carriers would be meaningless once priced in the same currency.
    Total,
    /// One stacked column per bucket, split by purpose — the end-use breakdown.
    /// Stacked because the parts sum to the node's total by construction, so the
    /// column height is meaningful in its own right.
    PerPurposeStacked,
}

impl ChartKind {
    /// ECharts series type.
    fn series_type(self) -> &'static str {
        match self {
            ChartKind::PerEnergyType => "line",
            _ => "bar",
        }
    }

    /// Stacked series share a bucket's column.
    fn stacked(self) -> bool {
        self == ChartKind::PerPurposeStacked
    }

}

/// Danish labels for the purposes (formål).
fn purpose_label(p: &str) -> &str {
    match p {
        "total" => "I alt",
        "unallocated" => "Ikke fordelt",
        "space_heating" => "Rumvarme",
        "dhw" => "Varmt brugsvand",
        "ventilation" => "Ventilation",
        "cooling" => "Køling",
        "lighting" => "Belysning",
        "plug_loads" => "Apparater",
        "ev_charging" => "Elbilopladning",
        "process" => "Proces",
        "common" => "Fælles",
        "generation" => "Produktion",
        other => other,
    }
}

/// Danish labels for the energy types.
///
/// Colour is deliberately NOT sent: it lived here as a second copy of
/// `RESOURCE_COLORS` in the frontend and all six values had already drifted
/// apart, so a card and the drill-down it opens coloured the same carrier
/// differently. The fragment carries `key` and the browser owns the palette.
fn energy_label(t: &str) -> &str {
    match t {
        "electricity" => "El",
        "district_heating" => "Fjernvarme",
        "district_cooling" => "Fjernkøling",
        "gas" => "Gas",
        "water" => "Vand",
        "heat" => "Varme",
        other => other,
    }
}

/// `?view=chart` — an HTML fragment carrying an ECharts config as inline
/// `application/json`, which the frontend's `mountDeclaredCharts` picks up.
///
/// The pivot from rows to series lives here rather than in the browser. That is
/// the point of this representation: a chart was the last thing the UI fetched as
/// JSON and shaped client-side, which meant a second data path with its own auth
/// handling. Now every widget arrives the same way — over htmx, as markup.
fn rows_to_chart(rows: &[Row], kind: ChartKind) -> String {
    use std::collections::{BTreeMap, BTreeSet};

    if rows.is_empty() {
        return "<p class=\"muted chart-state\">Ingen data for perioden.</p>".to_string();
    }

    let buckets: BTreeSet<&str> = rows.iter().map(|r| r.timestamp.as_str()).collect();
    let buckets: Vec<&str> = buckets.into_iter().collect();
    let index: BTreeMap<&str, usize> =
        buckets.iter().enumerate().map(|(i, b)| (*b, i)).collect();
    let unit = rows.iter().find(|r| !r.unit.is_empty()).map_or("", |r| r.unit.as_str());

    // series name -> values aligned to `buckets`
    let mut series: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for r in rows {
        let key = match kind {
            ChartKind::Total => "total",
            ChartKind::PerPurposeStacked => r.purpose.as_str(),
            _ => r.energy_type.as_str(),
        };
        let slot = series.entry(key).or_insert_with(|| vec![0.0; buckets.len()]);
        if let Some(i) = index.get(r.timestamp.as_str()) {
            slot[*i] += r.value;
        }
    }

    let categories: Vec<String> = buckets
        .iter()
        .map(|b| b.replace('T', " ").chars().take(16).collect())
        .collect();

    let series: Vec<serde_json::Value> = series
        .iter()
        .map(|(key, data)| {
            let (name, key) = match kind {
                ChartKind::Total => ("Total", None),
                // Purposes carry no carrier colour; the palette cycles instead.
                ChartKind::PerPurposeStacked => (purpose_label(key), None),
                _ => (energy_label(key), Some(*key)),
            };
            serde_json::json!({
                "name": name,
                // The browser maps key -> colour; see energy_label's note.
                "key": key,
                "type": kind.series_type(),
                "data": data,
            })
        })
        .collect();

    chart_block(&serde_json::json!({
        "categories": categories,
        "series": series,
        "unit": unit,
        "stacked": kind.stacked(),
        "zoom": false,
    }))
}

/// The markup contract `mountDeclaredCharts` looks for, in one place.
///
/// Mirrors `frontend/src/components/Chart.astro`; a change to the contract
/// should touch exactly these two.
fn chart_block(config: &serde_json::Value) -> String {
    format!(
        "<div data-chart><div data-chart-canvas style=\"width:100%;height:100%\"></div>\
         <script type=\"application/json\">{config}</script></div>"
    )
}

/// A `<tr>` table fragment of rollup rows
/// (energy type / purpose / time / value / unit / count).
///
/// Both axes are emitted because this fragment serves two endpoints: for
/// `get_aggregations` the purpose column is constant (`total`) and the energy type
/// is what varies, and for `get_purpose_split` it is exactly the other way round.
fn rows_to_html(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "<tr><td colspan=\"6\" class=\"muted\">Ingen data.</td></tr>".to_string();
    }
    let mut out = String::new();
    for r in rows {
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"mono\">{}</td><td class=\"mono\" style=\"text-align:right\">{:.3}</td><td>{}</td><td class=\"mono\">{}</td></tr>",
            esc(&r.energy_type),
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
        energy_type: "",
        // The derived read models (cost / emissions / benchmark / alarms) are about
        // what a node actually consumed, so they read `total` — never a breakdown,
        // which would only ever be a subset and would understate every figure.
        purpose: "total",
    };
    let items = query_node(client, params)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(to_rows(items, level_id, resolution, gran))
}

/// True for the various ways a cubic-metre unit is written in the data
/// (`m3`, `m³`, `m^3`). The rollup carries whatever the raw meter reported.
fn is_cubic_metre(u: &str) -> bool {
    u.contains("m3") || u.contains("m³") || u.contains("m^3")
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Apply a per-kWh / per-m³ rate to a value's unit: energy rolls up in Wh (the
/// per-kWh rate ÷1000), volumes in m³, anything else contributes 0. Shared by the
/// tariff and emission-factor tables below — they differ only in the rate pair.
fn per_unit(per_kwh: f64, per_m3: f64, unit: &str) -> f64 {
    let u = unit.to_ascii_lowercase();
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

/// Representative unit price (DKK) for an energy type, given the rollup's stored unit.
///
/// `generation` is priced at zero: exported PV is not bought, and charging it would
/// inflate the bill by the very amount it offsets on the grid-import series.
fn tariff_dkk_per_unit(energy_type: &str, purpose: &str, unit: &str) -> f64 {
    if purpose == "generation" {
        return 0.0;
    }
    let (per_kwh, per_m3) = match energy_type {
        "electricity" => (2.50, 0.0),
        "district_heating" | "heat" => (0.90, 0.0),
        "district_cooling" => (0.50, 0.0),
        "gas" => (0.0, 8.0),
        "water" => (0.0, 50.0),
        _ => (0.0, 0.0),
    };
    per_unit(per_kwh, per_m3, unit)
}

/// Representative CO₂e emission factor (kg CO₂e) for an energy type (Danish 2026
/// figures — swap when real factors land; mirrors the tariff config).
///
/// `generation` emits nothing: counting on-site production as emissions would
/// double-count it against the import series it displaces.
fn emission_kg_per_unit(energy_type: &str, purpose: &str, unit: &str) -> f64 {
    if purpose == "generation" {
        return 0.0;
    }
    let (per_kwh, per_m3) = match energy_type {
        "electricity" => (0.12, 0.0),          // DK grid mix
        "district_heating" | "heat" => (0.06, 0.0),
        "district_cooling" => (0.04, 0.0),
        "gas" => (0.0, 2.05),                   // natural gas, per m³
        "water" => (0.0, 0.34),                 // supply + treatment, per m³
        _ => (0.0, 0.0),
    };
    per_unit(per_kwh, per_m3, unit)
}

/// Extract the `(level_id, resolution, start, end)` window from the query string
/// (resolution defaults to `daily`); 400 if `start` or `end` is missing. The
/// shared preamble of the derived read-model handlers (cost / emissions /
/// benchmark / alarms).
fn window_params(
    qs: &HashMap<String, String>,
) -> Result<(String, String, String, String), ApiError> {
    let level_id = qs.get("level_id").cloned().unwrap_or_default();
    let resolution = qs.get("resolution").cloned().unwrap_or_else(|| "daily".to_string());
    let start = qs.get("start").cloned().unwrap_or_default();
    let end = qs.get("end").cloned().unwrap_or_default();
    if start.is_empty() || end.is_empty() {
        return Err(ApiError::bad_request("start and end are required (ISO-8601)"));
    }
    Ok((level_id, resolution, start, end))
}

/// Scale each node row's value by a per-(energy_type, purpose, unit) factor and
/// re-label the unit — the shared core of get_cost and get_emissions.
fn scale_rows(rows: Vec<Row>, unit: &str, factor: impl Fn(&str, &str, &str) -> f64) -> Vec<Row> {
    rows.into_iter()
        .map(|r| {
            let value = round2(r.value * factor(&r.energy_type, &r.purpose, &r.unit));
            Row { unit: unit.to_string(), value, ..r }
        })
        .collect()
}

/// `GET /meterdata/query/get_purpose_split` — a node's end-use breakdown for one
/// energy type: one series per purpose, including `total` and `unallocated`.
///
/// Fans out over `Purpose::all()` concurrently, the same shape as `query_node`'s
/// fan-out over energy types: each purpose is its own `sk BETWEEN` key-range, so
/// no query reads outside its own window. Purposes with no rows simply contribute
/// nothing — the matrix is sparse, so an undeclared purpose has no rows at all.
///
/// `total` is returned alongside the parts deliberately: the caller needs it to
/// show what fraction is accounted for, and `unallocated` is exactly the remainder
/// (`total` − Σ declared purposes), already materialised by crates/model.
#[utoipa::path(
    get,
    path = "/meterdata/query/get_purpose_split",
    tag = "aggregations",
    params(
        ("level_id" = String, Query, description = "Hierarchy node path (…|HN2#..)"),
        ("energy_type" = String, Query, description = "Energy type to break down (electricity, district_heating, …)"),
        ("resolution" = Option<String>, Query, description = "hourly | daily (default daily)"),
        ("start" = String, Query, description = "ISO-8601 start (required)"),
        ("end" = String, Query, description = "ISO-8601 end (required)"),
        ("format" = Option<String>, Query, description = "json (default) | html"),
    ),
    responses(
        (status = 200, description = "One [Row] per (purpose, bucket)", body = Vec<Row>),
        (status = 400, description = "Missing/invalid start or end", body = api::ErrorResponse),
    ),
)]
async fn handle_purpose_split(
    client: &Client,
    table: &str,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Json);
    let (level_id, resolution, start, end) = window_params(qs)?;
    let energy_type = qs
        .get("energy_type")
        .or_else(|| qs.get("resource"))
        .cloned()
        .unwrap_or_default();

    let gran = Gran::from_resolution(&resolution);
    let (pk, sk_path) = match parse_node_keys(&level_id) {
        Ok(keys) => keys,
        Err(_) => return Ok(rows_response(&[], format, Render::Table)),
    };
    let (start_bucket, end_bucket) = match (bucket_label(&start, gran), bucket_label(&end, gran)) {
        (Ok(s), Ok(e)) => (s, e),
        _ => return Err(ApiError::bad_request("start/end must be ISO-8601 timestamps")),
    };

    let per_purpose = try_join_all(Purpose::all().map(|purpose| {
        let (pk, sk_path) = (pk.clone(), sk_path.clone());
        let (start_bucket, end_bucket) = (start_bucket.clone(), end_bucket.clone());
        let energy_type = energy_type.clone();
        async move {
            let p = QueryParams {
                table,
                pk: &pk,
                sk_path: &sk_path,
                gran,
                start_bucket: &start_bucket,
                end_bucket: &end_bucket,
                energy_type: &energy_type,
                purpose: purpose.as_str(),
            };
            query_one_energy_type(client, &p, &energy_type).await
        }
    }))
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let items: Vec<AggItem> = per_purpose.into_iter().flatten().collect();
    let rows = to_rows(items, &level_id, &resolution, gran);
    let render = Render::resolve(qs.get("view").map(String::as_str));
    if format == Format::Html && render == Render::Chart {
        // `total` is dropped from the chart: it is the sum of the other series by
        // construction, so stacking it alongside them would double the column.
        let parts: Vec<Row> = rows.into_iter().filter(|r| r.purpose != "total").collect();
        return Ok(ApiResponse::html(
            200,
            rows_to_chart(&parts, ChartKind::PerPurposeStacked),
        ));
    }
    Ok(rows_response(&rows, format, render))
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
    let (level_id, resolution, start, end) = window_params(qs)?;
    let rows = fetch_node_rows(client, table, &level_id, &resolution, &start, &end).await?;
    let priced = scale_rows(rows, "DKK", tariff_dkk_per_unit);
    // Once every carrier is in DKK, one line is the answer — separate series would
    // invite reading "electricity vs heat" off a currency axis.
    let render = Render::resolve(qs.get("view").map(String::as_str));
    if format == Format::Html && render == Render::Chart {
        return Ok(ApiResponse::html(200, rows_to_chart(&priced, ChartKind::Total)));
    }
    Ok(rows_response(&priced, format, render))
}

/// `GET /meterdata/query/get_emissions` — consumption × per-resource emission
/// factor, one row per (resource, bucket) with the value in **kg CO₂e**. Same
/// shape as get_aggregations; the client sums + ÷1000 for tonnes.
#[utoipa::path(
    get,
    path = "/meterdata/query/get_emissions",
    tag = "aggregations",
    params(
        ("level_id" = String, Query, description = "Hierarchy node path (…|HN2#..)"),
        ("resolution" = Option<String>, Query, description = "hourly | daily (default daily)"),
        ("start" = String, Query, description = "ISO-8601 start (required)"),
        ("end" = String, Query, description = "ISO-8601 end (required)"),
        ("format" = Option<String>, Query, description = "json (default) | html"),
    ),
    responses(
        (status = 200, description = "Emission rows ([Row], value in kg CO₂e)", body = Vec<Row>),
        (status = 400, description = "Missing/invalid start or end", body = api::ErrorResponse),
    ),
)]
async fn handle_emissions(
    client: &Client,
    table: &str,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Json);
    let (level_id, resolution, start, end) = window_params(qs)?;
    let rows = fetch_node_rows(client, table, &level_id, &resolution, &start, &end).await?;
    let scaled = scale_rows(rows, "kg CO₂e", emission_kg_per_unit);
    let render = Render::resolve(qs.get("view").map(String::as_str));
    if format == Format::Html && render == Render::Chart {
        return Ok(ApiResponse::html(200, rows_to_chart(&scaled, ChartKind::PerEnergyTypeBars)));
    }
    Ok(rows_response(&scaled, format, render))
}

/// One building's cost + CO₂e over the window, with its deviation from the
/// company-average building (cost).
#[derive(Debug, Serialize, ToSchema)]
struct BuildingStat {
    node_path: String,
    cost_dkk: f64,
    co2e_kg: f64,
    cost_dev_pct: f64,
}

/// Peer benchmark: every meter-bearing building under the company vs the
/// company-average building (cost + CO₂e), with above/below counts.
#[derive(Debug, Serialize, ToSchema)]
struct Benchmark {
    level_id: String,
    building_count: usize,
    avg_cost_dkk: f64,
    avg_co2e_kg: f64,
    /// The selected node's own deviation when it is itself one of the buildings.
    node_is_building: bool,
    node_cost_dev_pct: f64,
    node_co2e_dev_pct: f64,
    above_count: usize,
    above_excess_dkk: f64,
    below_count: usize,
    below_saving_dkk: f64,
    buildings: Vec<BuildingStat>,
}


// ── Card fragments (HTML-over-the-wire) ──────────────────────────────────────
//
// The node dashboard renders these server-side rather than shipping JSON to a
// client-side component. Both cards are pure markup — no canvas — so there is
// nothing a browser framework has to do here.

/// Danish number formatting: thousands separator `.`, decimal comma.
fn da(n: f64, decimals: usize) -> String {
    let s = format!("{n:.decimals$}");
    let (int, frac) = s.split_once('.').unwrap_or((s.as_str(), ""));
    let neg = int.starts_with('-');
    let digits: Vec<char> = int.trim_start_matches('-').chars().collect();
    let mut out = String::new();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(*c);
    }
    let mut r = String::new();
    if neg {
        r.push('-');
    }
    r.push_str(&out);
    if !frac.is_empty() {
        r.push(',');
        r.push_str(frac);
    }
    r
}

/// `get_benchmark?format=html` — two gradient bars (cost, CO₂e) with a marker.
///
/// For a building the marker is its own deviation from the company-average
/// building; for a company or property it is the share of buildings above that
/// average, because "this node vs itself" would be meaningless.
fn benchmark_to_html(b: &Benchmark) -> String {
    if b.building_count == 0 {
        return "<p class=\"muted\">Ingen benchmarkdata.</p>".to_string();
    }
    let co2e_above = b.buildings.iter().filter(|x| x.co2e_kg > b.avg_co2e_kg).count();
    let n = b.building_count as f64;

    let bar = |title: &str, pos: f64, caption: String| {
        let pos = pos.clamp(0.0, 1.0) * 100.0;
        let accent = if pos > 50.0 { "var(--danger)" } else { "var(--success)" };
        format!(
            "<div class=\"bm-bar\">\
               <div class=\"bm-bar__head\"><span>{title}</span>\
                 <strong class=\"mono\" style=\"color:{accent}\">{caption}</strong></div>\
               <div class=\"bm-bar__track\"><div class=\"bm-bar__marker\" style=\"left:{pos:.1}%\"></div></div>\
             </div>",
            title = esc(title),
            caption = esc(&caption),
        )
    };

    let signed = |v: f64| format!("{}{:.1} %", if v > 0.0 { "+" } else { "" }, v);
    let (cost_pos, cost_cap) = if b.node_is_building {
        (0.5 + b.node_cost_dev_pct / 100.0, signed(b.node_cost_dev_pct))
    } else {
        (b.above_count as f64 / n, format!("{}/{} over gnm.", b.above_count, b.building_count))
    };
    let (co2e_pos, co2e_cap) = if b.node_is_building {
        (0.5 + b.node_co2e_dev_pct / 100.0, signed(b.node_co2e_dev_pct))
    } else {
        (co2e_above as f64 / n, format!("{}/{} over gnm.", co2e_above, b.building_count))
    };

    format!(
        "<div class=\"bm\">\
           {cost}{co2e}\
           <div class=\"bm-metrics\">\
             <div><span class=\"muted\">Bygninger</span><strong class=\"mono\">{count}</strong></div>\
             <div><span class=\"muted\">Gnm. omkostning</span><strong class=\"mono\">{avg_cost} kr.</strong></div>\
             <div><span class=\"muted\">Merforbrug over gnm.</span><strong class=\"mono\">{excess} kr.</strong></div>\
             <div><span class=\"muted\">Besparelse under gnm.</span><strong class=\"mono\">{saving} kr.</strong></div>\
           </div>\
         </div>",
        cost = bar("Omkostning vs. gennemsnit", cost_pos, cost_cap),
        co2e = bar("CO₂e vs. gennemsnit", co2e_pos, co2e_cap),
        count = b.building_count,
        avg_cost = da(b.avg_cost_dkk, 0),
        excess = da(b.above_excess_dkk, 0),
        saving = da(b.below_saving_dkk, 0),
    )
}

/// `get_alarms?format=html` — spike count plus the largest few.
fn alarms_to_html(r: &AlarmsResponse) -> String {
    let mut out = format!(
        "<div class=\"al-counts\">\
           <div><strong class=\"al-num mono\">{}</strong><span class=\"muted\">Forbrugsspidser</span></div>\
           <div><strong class=\"al-num mono\">{}</strong><span class=\"muted\">Vises</span></div>\
         </div>",
        r.count,
        r.alarms.len().min(5),
    );
    if r.alarms.is_empty() {
        out.push_str("<p class=\"muted\">Ingen forbrugsspidser i perioden.</p>");
        return out;
    }
    out.push_str("<table class=\"al-table\"><thead><tr>\
        <th>Energiart</th><th>Tidspunkt</th><th class=\"num\">Værdi</th>\
        <th class=\"num\">Median</th><th class=\"num\">Faktor</th></tr></thead><tbody>");
    for a in r.alarms.iter().take(5) {
        out.push_str(&format!(
            "<tr><td>{}</td><td class=\"mono\">{}</td>\
             <td class=\"mono num\">{} {}</td><td class=\"mono num\">{}</td>\
             <td class=\"mono num\">×{}</td></tr>",
            esc(&a.resource),
            esc(&a.timestamp),
            da(a.value, 0),
            esc(&a.unit),
            da(a.median, 0),
            da(a.ratio, 1),
        ));
    }
    out.push_str("</tbody></table>");
    out
}

/// Deviation of `cur` from baseline `base`, in percent (0 if base ~ 0).
fn pct_change(cur: f64, base: f64) -> f64 {
    if base.abs() < f64::EPSILON {
        0.0
    } else {
        round2((cur - base) / base * 100.0)
    }
}

/// The meter-bearing node for a leaf node_path (`…|HN4#1|L#9` → `…|HN4#1`), else None.
fn building_of_leaf(node_path: &str) -> Option<String> {
    node_path.rfind("|L#").map(|i| node_path[..i].to_string())
}

/// The most granular nodes in a set — those with no descendant node in it.
///
/// A benchmark compares like with like, which used to mean "nodes that own a meter",
/// found via the synthetic `|L#<sensor>` leaf rows the old `ancestor_keys` emitted.
/// The coefficient matrix has no such rows — sensors are not nodes — so that test
/// silently matched nothing and every benchmark came back empty.
///
/// Leaf-ness is the schema-agnostic version of the same intent: hierarchy levels are
/// dense depth indices, not fixed type slots (one company's HN4 is a building, another's
/// is an area), so "deepest node with its own consumption" travels where "HN4" does not.
fn leaf_node_paths<'a>(paths: impl Iterator<Item = &'a str>) -> std::collections::BTreeSet<String> {
    let all: std::collections::BTreeSet<String> = paths.map(str::to_string).collect();
    all.iter()
        .filter(|p| {
            let prefix = format!("{p}|");
            !all.iter().any(|q| q.starts_with(&prefix))
        })
        .cloned()
        .collect()
}

/// Rollup rows for a company partition (`pk = "HN2#<id>"`) — every descendant
/// node_path / resource / gran / bucket. A benchmark is company-wide by nature,
/// so this single-partition read is the natural unit. The gran/bucket live inside
/// the sort key (so they can't be filtered server-side — `sk` is a key attribute),
/// but a ProjectionExpression to the 3 fields actually used (sk/sum/unit) shrinks each item,
/// so pages hold more rows → fewer round trips and far less deserialisation than
/// fetching every attribute. `building_stats` does the gran/window filtering.
async fn query_company(client: &Client, table: &str, pk: &str) -> Result<Vec<AggItem>> {
    let raw = client
        .query()
        .table_name(table)
        .key_condition_expression("pk = :pk")
        .projection_expression("sk, #s, #u")
        .expression_attribute_names("#s", "sum")
        .expression_attribute_names("#u", "unit")
        .expression_attribute_values(":pk", AttributeValue::S(pk.to_string()))
        .into_paginator()
        .items()
        .send()
        .collect::<Result<Vec<_>, _>>()
        .await
        .map_err(|e| anyhow!("DynamoDB query: {:?}", e))?;
    Ok(raw.into_iter().filter_map(|it| serde_dynamo::from_item(it).ok()).collect())
}

/// Aggregate a company's rollup rows into per-building (cost, CO₂e) over the
/// window. A "building" = a meter-bearing node (the parent of a `L#` leaf); its
/// total is the node's own rollup rows (which already sum its meters). Pure given
/// the items, so it is unit-tested directly.
fn building_stats(
    items: &[AggItem],
    gran: Gran,
    start_bucket: &str,
    end_bucket: &str,
) -> std::collections::BTreeMap<String, (f64, f64)> {
    use std::collections::BTreeMap;
    let in_window = |g: &str, b: &str| g == gran.code() && b >= start_bucket && b <= end_bucket;

    // Accumulate every node's own cost/CO2e, then keep only the leaf nodes — the
    // benchmark unit. Property and company rows are roll-ups of those leaves, so
    // including them would compare a node against its own parts.
    let mut by_path: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for it in items {
        let (np, et, purpose, g, b) = parse_sk(&it.sk);
        if !in_window(g, b) {
            continue;
        }
        // Legacy leaf rows (`…|L#<sensor>`) are skipped, not folded: the old
        // ancestor_keys emitted a node's own row *and* a per-sensor leaf row for the
        // same consumption, so folding would count it twice. The matrix emits no
        // leaf rows at all, so this only matters for rows predating the rebuild.
        if building_of_leaf(np).is_some() {
            continue;
        }
        let e = by_path.entry(np.to_string()).or_insert((0.0, 0.0));
        e.0 += it.sum * tariff_dkk_per_unit(et, purpose, &it.unit);
        e.1 += it.sum * emission_kg_per_unit(et, purpose, &it.unit);
    }

    let leaves = leaf_node_paths(by_path.keys().map(String::as_str));
    by_path.retain(|np, _| leaves.contains(np));
    by_path
}

/// `GET /meterdata/query/get_benchmark` — peer benchmark of the buildings under
/// the selected node's company vs the company-average building (cost + CO₂e).
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
        (status = 200, description = "Peer benchmark across the company's buildings", body = Benchmark),
        (status = 400, description = "Missing/invalid start or end", body = api::ErrorResponse),
    ),
)]
async fn handle_benchmark(
    client: &Client,
    table: &str,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Json);
    let bench_response =
        |b: &Benchmark| match format {
            Format::Json => ApiResponse::json(b),
            Format::Html => ApiResponse::html(200, benchmark_to_html(b)),
        };
    let (level_id, resolution, start, end) = window_params(qs)?;
    let gran = Gran::from_resolution(&resolution);
    let empty = || Benchmark {
        level_id: level_id.clone(),
        building_count: 0,
        avg_cost_dkk: 0.0,
        avg_co2e_kg: 0.0,
        node_is_building: false,
        node_cost_dev_pct: 0.0,
        node_co2e_dev_pct: 0.0,
        above_count: 0,
        above_excess_dkk: 0.0,
        below_count: 0,
        below_saving_dkk: 0.0,
        buildings: Vec::new(),
    };
    let (pk, sk_path) = match parse_node_keys(&level_id) {
        Ok(keys) => keys,
        Err(_) => return Ok(bench_response(&empty())),
    };
    let (start_bucket, end_bucket) = match (bucket_label(&start, gran), bucket_label(&end, gran)) {
        (Ok(s), Ok(e)) => (s, e),
        _ => return Err(ApiError::bad_request("start/end must be ISO-8601 timestamps")),
    };

    let items = query_company(client, table, &pk)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let stats = building_stats(&items, gran, &start_bucket, &end_bucket);

    let n = stats.len();
    if n == 0 {
        return Ok(bench_response(&empty()));
    }
    let avg_cost = stats.values().map(|v| v.0).sum::<f64>() / n as f64;
    let avg_co2e = stats.values().map(|v| v.1).sum::<f64>() / n as f64;

    let (mut above_count, mut above, mut below_count, mut below) = (0usize, 0.0, 0usize, 0.0);
    let mut buildings: Vec<BuildingStat> = Vec::with_capacity(n);
    for (np, (cost, co2e)) in &stats {
        if *cost > avg_cost {
            above_count += 1;
            above += cost - avg_cost;
        } else if *cost < avg_cost {
            below_count += 1;
            below += avg_cost - cost;
        }
        buildings.push(BuildingStat {
            node_path: np.clone(),
            cost_dkk: round2(*cost),
            co2e_kg: round2(*co2e),
            cost_dev_pct: pct_change(*cost, avg_cost),
        });
    }
    buildings.sort_by(|a, b| {
        b.cost_dev_pct.partial_cmp(&a.cost_dev_pct).unwrap_or(std::cmp::Ordering::Equal)
    });

    let node_stat = stats.get(&sk_path);
    let bench = Benchmark {
        level_id,
        building_count: n,
        avg_cost_dkk: round2(avg_cost),
        avg_co2e_kg: round2(avg_co2e),
        node_is_building: node_stat.is_some(),
        node_cost_dev_pct: node_stat.map_or(0.0, |(c, _)| pct_change(*c, avg_cost)),
        node_co2e_dev_pct: node_stat.map_or(0.0, |(_, e)| pct_change(*e, avg_co2e)),
        above_count,
        above_excess_dkk: round2(above),
        below_count,
        below_saving_dkk: round2(below),
        buildings,
    };
    Ok(bench_response(&bench))
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
    let mut by_energy_type: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        by_energy_type.entry(r.energy_type.clone()).or_default().push(r);
    }
    let mut alarms = Vec::new();
    for (resource, rs) in by_energy_type {
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
    let (level_id, resolution, start, end) = window_params(qs)?;
    let spike_factor = qs
        .get("spike_factor")
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|f| *f > 1.0)
        .unwrap_or(2.0);

    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Json);
    let rows = fetch_node_rows(client, table, &level_id, &resolution, &start, &end).await?;
    let alarms = detect_spikes(&rows, spike_factor);
    let resp = AlarmsResponse {
        level_id,
        count: alarms.len(),
        alarms,
    };
    Ok(match format {
        Format::Json => ApiResponse::json(&resp),
        Format::Html => ApiResponse::html(200, alarms_to_html(&resp)),
    })
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
    paths(handle_aggregations, handle_purpose_split, handle_cost, handle_emissions, handle_benchmark, handle_alarms, raw::handle_measurements),
    components(schemas(Row, Benchmark, BuildingStat, Alarm, AlarmsResponse, model::domain::measurement::Measurement, api::ErrorResponse, api::ErrorDetail)),
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
        assert!(doc.contains("/meterdata/query/get_emissions"), "missing get_emissions path");
        assert!(doc.contains("/meterdata/query/get_benchmark"), "missing get_benchmark path");
        assert!(doc.contains("/meterdata/query/get_alarms"), "missing get_alarms path");
        assert!(doc.contains("Measurement"), "missing Measurement schema");
        assert!(doc.contains("ErrorResponse"), "missing ErrorResponse schema");
    }

    // Derived models ─────────────────────────────────────────────────────────────

    fn row(energy_type: &str, unit: &str, value: f64, ts: &str) -> Row {
        Row {
            level_id: "HN2#1".into(),
            energy_type: energy_type.into(),
            purpose: "total".into(),
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
        assert!((tariff_dkk_per_unit("electricity", "total", "kWh") - 2.50).abs() < 1e-9);
        assert!((tariff_dkk_per_unit("electricity", "total", "Wh") - 0.0025).abs() < 1e-9);
        assert!((tariff_dkk_per_unit("water", "total", "m3") - 50.0).abs() < 1e-9);
        // The rollup writes volumes as "m^3" — must price the same as "m3"/"m³".
        assert!((tariff_dkk_per_unit("water", "total", "m^3") - 50.0).abs() < 1e-9);
        assert!((tariff_dkk_per_unit("water", "total", "m³") - 50.0).abs() < 1e-9);
        assert_eq!(tariff_dkk_per_unit("electricity", "total", "°C"), 0.0, "non-priced unit → 0");
        assert_eq!(tariff_dkk_per_unit("unknown", "total", "kWh"), 0.0, "unknown energy type → 0");
    }

    #[test]
    fn emission_factor_is_unit_aware() {
        // electricity 0.12 kg/kWh; rollup stores Wh → 1/1000.
        assert!((emission_kg_per_unit("electricity", "total", "kWh") - 0.12).abs() < 1e-9);
        assert!((emission_kg_per_unit("electricity", "total", "Wh") - 0.00012).abs() < 1e-9);
        assert!((emission_kg_per_unit("water", "total", "m^3") - 0.34).abs() < 1e-9);
        assert_eq!(emission_kg_per_unit("electricity", "total", "°C"), 0.0);
    }

    #[test]
    fn building_stats_groups_by_meter_bearing_node() {
        let it = |sk: &str, unit: &str, sum: f64| AggItem {
            sk: sk.into(),
            sum,
            count: 1,
            unit: unit.into(),
        };
        // Two buildings under one company. Building-level rows aggregate their
        // meters; leaf (L#) rows mark which node_paths are buildings.
        let items = vec![
            // building A own row + its leaf
            it("HN2#1|HN3#1|HN4#1#electricity#total#d#2026-06-01", "Wh", 1000.0),
            it("HN2#1|HN3#1|HN4#1|L#9#electricity#total#d#2026-06-01", "Wh", 1000.0),
            // building B own row + its leaf
            it("HN2#1|HN3#1|HN4#2#electricity#total#d#2026-06-01", "Wh", 3000.0),
            it("HN2#1|HN3#1|HN4#2|L#8#electricity#total#d#2026-06-01", "Wh", 3000.0),
            // company + property aggregate rows (not meter-bearing → excluded)
            it("HN2#1#electricity#total#d#2026-06-01", "Wh", 4000.0),
            it("HN2#1|HN3#1#electricity#total#d#2026-06-01", "Wh", 4000.0),
            // out of window → excluded
            it("HN2#1|HN3#1|HN4#1#electricity#total#d#2026-07-01", "Wh", 9999.0),
        ];
        let stats = building_stats(&items, Gran::Day, "2026-06-01", "2026-06-30");
        assert_eq!(stats.len(), 2, "only the two meter-bearing buildings");
        // cost = Wh × 0.0025; A=1000→2.5, B=3000→7.5
        assert!((stats["HN2#1|HN3#1|HN4#1"].0 - 2.5).abs() < 1e-9);
        assert!((stats["HN2#1|HN3#1|HN4#2"].0 - 7.5).abs() < 1e-9);
    }


    #[test]
    fn leaf_nodes_are_those_with_no_descendant() {
        let paths = ["HN2#1", "HN2#1|HN3#1", "HN2#1|HN3#1|HN4#1", "HN2#1|HN3#1|HN4#2", "HN2#1|HN3#2"];
        let leaves = leaf_node_paths(paths.into_iter());
        assert_eq!(
            leaves.into_iter().collect::<Vec<_>>(),
            vec!["HN2#1|HN3#1|HN4#1", "HN2#1|HN3#1|HN4#2", "HN2#1|HN3#2"],
            "company and property roll up their children, so only the tips benchmark"
        );
    }

    #[test]
    fn leaf_detection_is_not_fooled_by_a_shared_prefix() {
        // "HN4#1" must not count as an ancestor of "HN4#10" — the separator matters.
        let leaves = leaf_node_paths(["HN2#1|HN4#1", "HN2#1|HN4#10"].into_iter());
        assert_eq!(leaves.len(), 2);
    }

    #[test]
    fn building_stats_benchmarks_leaf_nodes_without_legacy_meter_rows() {
        // Regression: benchmarking used to find buildings via synthetic `|L#<sensor>`
        // rows, which the coefficient matrix does not emit — so every benchmark came
        // back empty. Leaf-ness replaces it.
        let it = |sk: &str, unit: &str, sum: f64| AggItem {
            sk: sk.to_string(), sum, count: 1, unit: unit.to_string(),
        };
        let items = vec![
            it("HN2#1|HN3#1|HN4#1#electricity#total#d#2026-06-01", "Wh", 1000.0),
            it("HN2#1|HN3#1|HN4#2#electricity#total#d#2026-06-01", "Wh", 3000.0),
            it("HN2#1|HN3#1#electricity#total#d#2026-06-01", "Wh", 4000.0),
            it("HN2#1#electricity#total#d#2026-06-01", "Wh", 4000.0),
        ];
        let stats = building_stats(&items, Gran::Day, "2026-06-01", "2026-06-30");
        assert_eq!(stats.len(), 2, "the two HN4 tips, not the property or company");
        assert!((stats["HN2#1|HN3#1|HN4#1"].0 - 2.5).abs() < 1e-9);
        assert!((stats["HN2#1|HN3#1|HN4#2"].0 - 7.5).abs() < 1e-9);
    }

    #[test]
    fn building_of_leaf_strips_meter() {
        assert_eq!(building_of_leaf("HN2#1|HN3#1|HN4#1|L#9").as_deref(), Some("HN2#1|HN3#1|HN4#1"));
        assert_eq!(building_of_leaf("HN2#1|HN3#1|HN4#1"), None);
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
        // The all-resources query fans out over the shared domain `EnergyType` — must
        // be non-empty, and each `as_str` must match what the Glue rollup writes
        // into the SK.
        assert!(EnergyType::all().next().is_some(), "fan-out needs at least one resource");
        assert_eq!(EnergyType::Electricity.as_str(), "electricity");
        assert_eq!(EnergyType::Water.as_str(), "water");
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



    #[test]
    fn chart_fragment_carries_one_series_per_energy_type() {
        let rows = vec![
            row("electricity", "kWh", 10.0, "2026-07-01T00:00:00Z"),
            row("electricity", "kWh", 12.0, "2026-07-02T00:00:00Z"),
            row("water", "m3", 3.0, "2026-07-01T00:00:00Z"),
        ];
        let html = rows_to_chart(&rows, ChartKind::PerEnergyType);
        assert!(html.contains("data-chart"), "not a mountable chart block");
        assert!(html.contains(r#"application/json"#));
        assert!(html.contains(r#""name":"El""#), "danish label missing: {html}");
        assert!(html.contains(r#""name":"Vand""#));
        assert!(html.contains("[10.0,12.0]"), "electricity series wrong: {html}");
        // water has no reading on the 2nd, so it must still align to the axis.
        assert!(html.contains("[3.0,0.0]"), "series not aligned to a shared axis: {html}");
        // Colour is the browser's job; the fragment names the carrier instead.
        assert!(html.contains(r#""key":"electricity""#), "carrier key missing: {html}");
        assert!(!html.contains("color"), "colour must not be sent: {html}");
        // The consumption trend is the only line; everything else is columns.
        assert!(html.contains(r#""type":"line""#), "consumption should be a line: {html}");
    }

    #[test]
    fn cost_chart_sums_carriers_into_one_series() {
        let rows = vec![
            row("electricity", "DKK", 10.0, "2026-07-01T00:00:00Z"),
            row("water", "DKK", 5.0, "2026-07-01T00:00:00Z"),
        ];
        let html = rows_to_chart(&rows, ChartKind::Total);
        assert!(html.contains(r#""name":"Total""#));
        assert!(html.contains("[15.0]"), "carriers not summed: {html}");
        assert_eq!(html.matches(r#""type":"bar""#).count(), 1, "cost is one column series");
    }


    #[test]
    fn each_widget_keeps_the_series_type_it_had_before_going_server_side() {
        // Only the consumption trend was ever a line. Rendering everything as a
        // line silently restyled cost, emissions and the sparklines.
        assert_eq!(ChartKind::PerEnergyType.series_type(), "line");
        assert_eq!(ChartKind::PerEnergyTypeBars.series_type(), "bar");
        assert_eq!(ChartKind::Total.series_type(), "bar");
    }

    #[test]
    fn emissions_and_sparklines_are_columns() {
        let rows = vec![
            row("electricity", "kg CO₂e", 10.0, "2026-07-01T00:00:00Z"),
            row("water", "kg CO₂e", 3.0, "2026-07-01T00:00:00Z"),
        ];
        let html = rows_to_chart(&rows, ChartKind::PerEnergyTypeBars);
        assert_eq!(html.matches(r#""type":"bar""#).count(), 2, "one column series per carrier");
        assert!(!html.contains(r#""type":"line""#));

        let cards = rows_to_cards(&rows);
        assert!(cards.contains(r#""type":"bar""#), "sparklines are columns: {cards}");
    }


    #[test]
    fn purpose_split_chart_stacks_the_parts_and_drops_total() {
        // total is the sum of the parts, so stacking it too would double the column.
        let r = |purpose: &str, v: f64| Row {
            level_id: "HN2#1".into(),
            energy_type: "electricity".into(),
            purpose: purpose.into(),
            unit: "kWh".into(),
            resolution: "daily".into(),
            timestamp: "2026-07-01T00:00:00Z".into(),
            value: v,
            contributor_count: 1,
        };
        let parts: Vec<Row> = vec![r("lighting", 4.0), r("plug_loads", 6.0)];
        let html = rows_to_chart(&parts, ChartKind::PerPurposeStacked);
        assert!(html.contains(r#""stacked":true"#), "not stacked: {html}");
        assert_eq!(html.matches(r#""type":"bar""#).count(), 2, "one column series per purpose");
        assert!(html.contains("Belysning") && html.contains("Apparater"), "danish labels: {html}");
        assert!(!html.contains("I alt"), "total must not be a stacked series: {html}");
    }

    #[test]
    fn chart_fragment_says_so_when_there_is_no_data() {
        let html = rows_to_chart(&[], ChartKind::PerEnergyType);
        assert!(html.contains("Ingen data"));
        assert!(!html.contains("data-chart"), "must not mount an empty chart");
    }

    #[test]
    fn render_is_negotiated_separately_from_the_media_type() {
        // `format` is the media type, `view` picks which HTML rendering. Keeping
        // them apart is why liveness and raw measurements never have to know a
        // chart exists.
        assert_eq!(Render::resolve(Some("chart")), Render::Chart);
        assert_eq!(Render::resolve(Some("cards")), Render::Cards);
        assert_eq!(Render::resolve(None), Render::Table);
        assert_eq!(Render::resolve(Some("nonsense")), Render::Table);
        assert_eq!(Format::resolve(Some("html"), Format::Json), Format::Html);
        assert_eq!(Format::resolve(None, Format::Json), Format::Json);
    }

    // Card fragments ────────────────────────────────────────────────────────────

    #[test]
    fn da_formats_danish_thousands_and_decimal_comma() {
        assert_eq!(da(1234567.0, 0), "1.234.567");
        assert_eq!(da(1234.5, 1), "1.234,5");
        assert_eq!(da(-2500.0, 0), "-2.500");
        assert_eq!(da(999.0, 0), "999");
        assert_eq!(da(0.0, 0), "0");
    }

    fn bench_fixture(node_is_building: bool) -> Benchmark {
        Benchmark {
            level_id: "HN2#1".into(),
            building_count: 4,
            avg_cost_dkk: 1000.0,
            avg_co2e_kg: 500.0,
            node_is_building,
            node_cost_dev_pct: 20.0,
            node_co2e_dev_pct: -10.0,
            above_count: 3,
            above_excess_dkk: 250.0,
            below_count: 1,
            below_saving_dkk: 40.0,
            buildings: vec![BuildingStat {
                node_path: "HN2#1|HN3#1|HN4#1".into(),
                cost_dkk: 1200.0,
                co2e_kg: 600.0,
                cost_dev_pct: 20.0,
            }],
        }
    }

    #[test]
    fn benchmark_html_marks_a_buildings_own_deviation() {
        let html = benchmark_to_html(&bench_fixture(true));
        assert!(html.contains("+20.0 %"), "{html}");
        assert!(html.contains("left:70.0%"), "0.5 + 20/100 -> 70%: {html}");
    }

    #[test]
    fn benchmark_html_marks_a_companys_share_above_average() {
        // "this node vs itself" is meaningless for a company, so it shows how many
        // of its buildings are above the average instead.
        let html = benchmark_to_html(&bench_fixture(false));
        assert!(html.contains("3/4 over gnm."), "{html}");
        assert!(html.contains("left:75.0%"), "3 of 4 -> 75%: {html}");
    }

    #[test]
    fn benchmark_html_says_so_when_there_are_no_buildings() {
        let mut b = bench_fixture(false);
        b.building_count = 0;
        assert!(benchmark_to_html(&b).contains("Ingen benchmarkdata"));
    }

    #[test]
    fn alarms_html_lists_at_most_five_and_reports_the_full_count() {
        let alarm = |i: usize| Alarm {
            resource: "electricity".into(),
            timestamp: format!("2026-07-0{i}"),
            value: 4000.0,
            median: 1000.0,
            ratio: 4.0,
            unit: "Wh".into(),
        };
        let resp = AlarmsResponse {
            level_id: "HN2#1".into(),
            count: 7,
            alarms: (1..=7).map(alarm).collect(),
        };
        let html = alarms_to_html(&resp);
        assert_eq!(html.matches("<tr><td>").count(), 5, "caps the table at 5");
        assert!(html.contains(">7<"), "but still reports all 7: {html}");
        assert!(html.contains("4.000 Wh"), "danish formatting: {html}");
    }

    #[test]
    fn alarms_html_handles_an_empty_period() {
        let resp = AlarmsResponse { level_id: "HN2#1".into(), count: 0, alarms: vec![] };
        let html = alarms_to_html(&resp);
        assert!(html.contains("Ingen forbrugsspidser"));
        assert!(!html.contains("<table"));
    }

    // parse_sk ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_sk_simple() {
        // node_path contains no '#'
        let (node, et, purpose, gran, bucket) =
            parse_sk("HN2#42|HN3#99#electricity#total#h#2024-01-15T10");
        assert_eq!(node, "HN2#42|HN3#99");
        assert_eq!(et, "electricity");
        assert_eq!(purpose, "total");
        assert_eq!(gran, "h");
        assert_eq!(bucket, "2024-01-15T10");
    }

    #[test]
    fn parse_sk_reads_the_purpose_segment() {
        let (path, et, purpose, gran, bucket) =
            parse_sk("HN2#2|HN3#9#electricity#lighting#h#2026-06-07T08");
        assert_eq!(
            (path, et, purpose, gran, bucket),
            ("HN2#2|HN3#9", "electricity", "lighting", "h", "2026-06-07T08")
        );
    }

    #[test]
    fn test_parse_sk_node_path_with_hash() {
        // node_path itself contains '#' — the last 4 splits still give the right fields
        let (node, et, purpose, gran, bucket) =
            parse_sk("HN2#10|HN3#20#gas#total#d#2024-06-01");
        assert_eq!(node, "HN2#10|HN3#20");
        assert_eq!(et, "gas");
        assert_eq!(purpose, "total");
        assert_eq!(gran, "d");
        assert_eq!(bucket, "2024-06-01");
    }

    #[test]
    fn test_parse_sk_too_few_hashes() {
        let (node, et, purpose, gran, bucket) = parse_sk("only#three#hashes#here");
        // 3 '#' < 4, so fallback: node = full sk, rest empty
        assert_eq!(node, "only#three#hashes#here");
        assert_eq!(et, "");
        assert_eq!(purpose, "");
        assert_eq!(gran, "");
        assert_eq!(bucket, "");
    }

    /// The physical purposes plus unallocated add up to total.
    #[test]
    fn purpose_split_rows_are_grouped_by_purpose() {
        let items = vec![
            make_item("HN2#2#electricity#total#d#2026-06-07", 100.0, 1, "kWh"),
            make_item("HN2#2#electricity#lighting#d#2026-06-07", 22.0, 1, "kWh"),
            make_item("HN2#2#electricity#cooling#d#2026-06-07", 18.0, 1, "kWh"),
            make_item("HN2#2#electricity#unallocated#d#2026-06-07", 60.0, 1, "kWh"),
        ];
        let by: std::collections::BTreeMap<_, _> = to_rows(items, "HN2#2", "daily", Gran::Day)
            .iter()
            .map(|r| (r.purpose.clone(), r.value))
            .collect();
        assert_eq!(by["total"], 100.0);
        assert_eq!(by["lighting"] + by["cooling"] + by["unallocated"], by["total"]);
    }

    #[test]
    fn query_prefix_defaults_to_total() {
        assert_eq!(
            sk_prefix("HN2#2", "electricity", "", Gran::Hour),
            "HN2#2#electricity#total#h#"
        );
        assert_eq!(
            sk_prefix("HN2#2", "electricity", "lighting", Gran::Hour),
            "HN2#2#electricity#lighting#h#"
        );
    }

    #[test]
    fn generation_has_no_tariff_or_emissions() {
        // Exported PV is not consumption: charging it or counting its CO2 as the
        // building's would double-count against the grid series it offsets.
        assert_eq!(emission_kg_per_unit("electricity", "generation", "kWh"), 0.0);
        assert_eq!(tariff_dkk_per_unit("electricity", "generation", "kWh"), 0.0);
    }

    #[test]
    fn other_purposes_fall_back_to_the_energy_type_factor() {
        assert_eq!(emission_kg_per_unit("electricity", "lighting", "kWh"), 0.12);
        assert_eq!(emission_kg_per_unit("electricity", "total", "kWh"), 0.12);
        assert_eq!(tariff_dkk_per_unit("water", "total", "m3"), 50.0);
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
    fn test_to_rows_groups_by_energy_type_then_purpose() {
        let items = vec![
            make_item("HN2#1#electricity#total#h#2024-01-01T10", 100.0, 5, "kWh"),
            make_item("HN2#1#electricity#lighting#h#2024-01-01T10", 30.0, 2, "kWh"),
            make_item("HN2#1#gas#total#h#2024-01-01T10", 50.0, 3, "m3"),
        ];
        let rows = to_rows(items, "HN2#1", "hourly", Gran::Hour);
        assert_eq!(rows.len(), 3);
        // sorted by energy_type, then purpose: (electricity, lighting) <
        // (electricity, total) < (gas, total)
        assert_eq!((rows[0].energy_type.as_str(), rows[0].purpose.as_str()),
                   ("electricity", "lighting"));
        assert_eq!((rows[1].energy_type.as_str(), rows[1].purpose.as_str()),
                   ("electricity", "total"));
        assert_eq!((rows[2].energy_type.as_str(), rows[2].purpose.as_str()),
                   ("gas", "total"));
    }

    #[test]
    fn test_to_rows_dimension_sums_across_resources_per_bucket() {
        // Two resources in the same bucket sum into one energy row; a second
        // bucket stays separate; wrong-granularity rows are dropped.
        let items = vec![
            make_item("HN2#1#electricity#total#h#2024-01-01T10", 100.0, 5, "kWh"),
            make_item("HN2#1#heat#total#h#2024-01-01T10", 40.0, 2, "kWh"),
            make_item("HN2#1#electricity#total#h#2024-01-01T11", 20.0, 1, "kWh"),
            make_item("HN2#1#electricity#total#d#2024-01-01", 999.0, 9, "kWh"), // wrong gran
        ];
        let rows = to_rows_dimension(items, "HN2#1", "hourly", Gran::Hour, "energy", "total");
        assert_eq!(rows.len(), 2, "two hourly buckets");
        assert!(rows.iter().all(|r| r.energy_type == "energy" && r.purpose == "total"));
        // sorted by bucket; first bucket = electricity + heat
        assert_eq!(rows[0].timestamp, "2024-01-01T10:00:00Z");
        assert_eq!(rows[0].value, 140.0);
        assert_eq!(rows[0].contributor_count, 7);
        assert_eq!(rows[1].value, 20.0);
    }

    #[test]
    fn test_to_rows_filters_wrong_gran() {
        let items = vec![
            make_item("HN2#1#electricity#total#h#2024-01-01T10", 100.0, 5, "kWh"),
            make_item("HN2#1#electricity#total#d#2024-01-01", 2400.0, 5, "kWh"), // daily, should be filtered
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
            make_item("HN2#1#electricity#total#h#2024-01-01T12", 30.0, 1, "kWh"),
            make_item("HN2#1#electricity#total#h#2024-01-01T10", 10.0, 1, "kWh"),
            make_item("HN2#1#electricity#total#h#2024-01-01T11", 20.0, 1, "kWh"),
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
            "HN2#10#electricity#total#h#2024-01-01T08",
            42.5,
            7,
            "kWh",
        )];
        let rows = to_rows(items, "HN2#10", "hourly", Gran::Hour);
        assert_eq!(rows.len(), 1);
        let val = serde_json::to_value(&rows[0]).unwrap();
        assert!(val.get("level_id").is_some(), "missing level_id");
        assert!(val.get("energy_type").is_some(), "missing energy_type");
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
        assert_eq!(val["energy_type"], "electricity");
        assert_eq!(val["purpose"], "total");
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
            make_item("HN2#5#heat#total#d#2024-03-02", 800.0, 10, "kWh"),
            make_item("HN2#5#heat#total#d#2024-03-01", 900.0, 10, "kWh"),
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
