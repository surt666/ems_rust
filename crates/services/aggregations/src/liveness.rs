//! Device-liveness read: "is this device sending data?" — reads the latest-state
//! `meter-liveness` DynamoDB table (written by the meter-heartbeat lambda). Keyed
//! `pk=customerid, sk=meterid`, so a lookup is a `GetItem` (one device) or a
//! `Query(pk=customerid)` (all of a customer's meters) — O(1), no scan.
//!
//! Serves the Raw Device MFE: HTML fragment (default) or JSON.

use std::collections::HashMap;

use aws_sdk_dynamodb::{types::AttributeValue, Client};
use serde::{Deserialize, Serialize};

use api::{ApiError, ApiResponse, Format};

/// One device's latest-seen state. `customerid`/`meterid` are the table keys; the rest
/// are informational (gateway is *not* a key — a LoRaWAN uplink is heard by many).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LivenessRow {
    #[serde(rename = "pk")]
    pub customerid: String,
    #[serde(rename = "sk")]
    pub meterid: String,
    #[serde(default)]
    pub last_seen: String,
    #[serde(default)]
    pub event_time: String,
    #[serde(default)]
    pub gatewayid: String,
    #[serde(default)]
    pub schematype: String,
    #[serde(default)]
    pub transport: String,
}

/// `GET …/query/get_liveness?customerid=&meterid=&format=html|json`. `customerid` is
/// required (the partition); `meterid` narrows to one device. Key values are bound
/// params (data, not expression), so no injection surface.
pub async fn handle_liveness(
    client: &Client,
    table: &str,
    qs: &HashMap<String, String>,
) -> Result<ApiResponse, ApiError> {
    let format = Format::resolve(qs.get("format").map(String::as_str), Format::Html);

    let clean = |k: &str| qs.get(k).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let Some(customerid) = clean("customerid") else {
        return Err(ApiError::bad_request("customerid is required"));
    };
    let meterid = clean("meterid");

    let rows = match &meterid {
        Some(m) => get_one(client, table, &customerid, m).await?,
        None => query_customer(client, table, &customerid).await?,
    };

    Ok(match format {
        Format::Json => ApiResponse::json(&rows),
        Format::Html => ApiResponse::html(200, render_fragment(&customerid, meterid.as_deref(), &rows)),
    })
}

/// One device: `GetItem(pk=customerid, sk=meterid)` → 0 or 1 row.
async fn get_one(
    client: &Client,
    table: &str,
    customerid: &str,
    meterid: &str,
) -> Result<Vec<LivenessRow>, ApiError> {
    let resp = client
        .get_item()
        .table_name(table)
        .key("pk", AttributeValue::S(customerid.to_string()))
        .key("sk", AttributeValue::S(meterid.to_string()))
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("dynamodb get: {e}")))?;
    Ok(resp
        .item
        .and_then(|it| serde_dynamo::from_item(it).ok())
        .into_iter()
        .collect())
}

/// All of a customer's meters: `Query(pk=customerid)`.
async fn query_customer(
    client: &Client,
    table: &str,
    customerid: &str,
) -> Result<Vec<LivenessRow>, ApiError> {
    let resp = client
        .query()
        .table_name(table)
        .key_condition_expression("pk = :pk")
        .expression_attribute_values(":pk", AttributeValue::S(customerid.to_string()))
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("dynamodb query: {e}")))?;
    Ok(resp
        .items
        .unwrap_or_default()
        .into_iter()
        .filter_map(|it| serde_dynamo::from_item(it).ok())
        .collect())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The `#rd-result` fragment the Raw Device MFE swaps in.
fn render_fragment(customerid: &str, meterid: Option<&str>, rows: &[LivenessRow]) -> String {
    let target = match meterid {
        Some(m) => format!("meter <b>{}</b> (customer <b>{}</b>)", esc(m), esc(customerid)),
        None => format!("customer <b>{}</b>", esc(customerid)),
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
        esc(last),
    ));
    out.push_str("<table><thead><tr><th>Meter</th><th>Gateway</th><th>Type</th><th>Transport</th><th>Last seen</th></tr></thead><tbody>");
    for r in rows {
        let cell = |s: &str| if s.is_empty() { "—".to_string() } else { esc(s) };
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            cell(&r.meterid),
            cell(&r.gatewayid),
            cell(&r.schematype),
            cell(&r.transport),
            cell(&r.last_seen),
        ));
    }
    out.push_str("</tbody></table></div>");
    out
}
