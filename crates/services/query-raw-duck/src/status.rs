//! Device-liveness status: "is this gateway/meter sending data?" — reads the heartbeat
//! Parquet lake (written by the meter-heartbeat lambda) via DuckDB `read_parquet`.
//! Query keys are `gatewayid` and/or `meterid` (meterid == the heartbeat `device_id`).

use chrono::{Duration, Utc};

/// Validated status query. At least one of gateway/meter is required.
pub struct StatusQuery {
    pub gatewayid: Option<String>,
    pub meterid: Option<String>,
    /// lower bound partition (dt >= this), as YYYY-MM-DD.
    pub since: String,
    pub days: i64,
}

impl StatusQuery {
    pub fn parse(get: impl Fn(&str) -> Option<String>) -> Result<StatusQuery, String> {
        let clean = |k: &str| {
            get(k).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).filter(|s| {
                s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '#'))
            })
        };
        let gatewayid = clean("gatewayid");
        let meterid = clean("meterid");
        if gatewayid.is_none() && meterid.is_none() {
            return Err("provide a gatewayid and/or meterid".into());
        }
        // No `days` control in the MFE, and the lake only retains 14 days (bucket
        // expiration) — so default to the full retention window and cap there. Querying
        // further back can only scan empty partitions.
        let days = get("days").and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(14).clamp(1, 14);
        let since = (Utc::now() - Duration::days(days)).format("%Y-%m-%d").to_string();
        Ok(StatusQuery { gatewayid, meterid, since, days })
    }
}

/// One (device, gateway, type) group with its last-seen + count.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusRow {
    pub device_id: Option<String>,
    pub gateway_id: Option<String>,
    pub schematype: Option<String>,
    pub transport: Option<String>,
    pub last_seen: Option<String>,
    pub last_event: Option<String>,
    pub msgs: i64,
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// HTML fragment for the Raw Device MFE result area.
pub fn render_fragment(q: &StatusQuery, rows: &[StatusRow]) -> String {
    let mut out = String::from("<div id=\"rd-result\">");
    let target = match (&q.gatewayid, &q.meterid) {
        (Some(g), Some(m)) => format!("meter <b>{}</b> on gateway <b>{}</b>", esc(m), esc(g)),
        (None, Some(m)) => format!("meter <b>{}</b>", esc(m)),
        (Some(g), None) => format!("gateway <b>{}</b>", esc(g)),
        _ => String::new(),
    };
    if rows.is_empty() {
        out.push_str(&format!(
            "<p>\u{26d4} No data — {} has not sent anything in the last {} days.</p>",
            target, q.days
        ));
        out.push_str("</div>");
        return out;
    }
    let total: i64 = rows.iter().map(|r| r.msgs).sum();
    let last = rows.iter().filter_map(|r| r.last_seen.clone()).max().unwrap_or_default();
    out.push_str(&format!(
        "<p>\u{2705} {} is reporting — {} messages, last seen <b>{}</b> (last {} days).</p>",
        target, total, esc(&last), q.days
    ));
    out.push_str("<table><thead><tr><th>Meter</th><th>Gateway</th><th>Type</th><th>Transport</th><th>Last seen</th><th>Msgs</th></tr></thead><tbody>");
    for r in rows {
        let c = |o: &Option<String>| esc(o.as_deref().unwrap_or("—"));
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            c(&r.device_id), c(&r.gateway_id), c(&r.schematype), c(&r.transport), c(&r.last_seen), r.msgs
        ));
    }
    out.push_str("</tbody></table></div>");
    out
}
