//! Datatilegnelse (`get_measurements`) contract, reproduced so this lambda can serve
//! the frontend measurements page directly from DuckDB.
//!
//! The HTML fragment + query contract are the source-of-truth copy of
//! `crates/services/aggregations/src/raw.rs` (that lives in a bin crate, so it can't be
//! imported). **Keep this in sync with raw.rs** if the table shell / columns change:
//! bare `<tr>` rows (4 cols) swapped into `<tbody id="m-rows">`, plus one out-of-band
//! `#m-before` keyset cursor input.

use chrono::{Duration, Utc};

/// One reading, rendered. All strings (value pre-formatted) to match the Athena path's
/// `Measurement` exactly.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Measurement {
    pub timestamp: String,
    pub value: String,
    pub unit: String,
}

/// Content negotiation — get_measurements defaults to HTML (HTMX).
#[derive(Clone, Copy)]
pub enum Format {
    Html,
    Json,
}

impl Format {
    pub fn resolve(raw: Option<&str>) -> Format {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("json") => Format::Json,
            _ => Format::Html,
        }
    }
}

/// Validated query. `from`/`to` are normalized into DuckDB-parseable datetime strings
/// (date-only inputs from the `<input type=date>` are widened to full-day bounds);
/// `before` is the keyset cursor (strictly-older-than) for infinite scroll.
pub struct MeasurementQuery {
    pub daq_id: String,
    pub from: String,
    pub to: String,
    pub before: Option<String>,
    pub limit: usize,
}

impl MeasurementQuery {
    pub fn parse(get: impl Fn(&str) -> Option<String>) -> Result<MeasurementQuery, String> {
        let daq_id = get("daq_id")
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .ok_or("daq_id required")?;
        // Bound params make this injection-safe already; parse just gives a clean 400.
        crate::types::DaqId::parse(&daq_id)?;

        let now = Utc::now();
        let from = norm_bound(get("from"), false)
            .unwrap_or_else(|| fmt_dt(now - Duration::days(1)));
        let to = norm_bound(get("to"), true).unwrap_or_else(|| fmt_dt(now));
        let before = get("before").map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        let limit = get("limit")
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(100)
            .clamp(1, 500);

        Ok(MeasurementQuery { daq_id, from, to, before, limit })
    }
}

fn fmt_dt(t: chrono::DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Blank → None. A date-only `YYYY-MM-DD` is widened to a full-day bound
/// (`end=true` → 23:59:59, else 00:00:00) so an inclusive day range works.
fn norm_bound(raw: Option<String>, end: bool) -> Option<String> {
    let s = raw.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())?;
    if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        Some(format!("{s} {}", if end { "23:59:59" } else { "00:00:00" }))
    } else {
        Some(s)
    }
}

/// Bare `<tr>` rows for `<tbody id="m-rows">` + the OOB keyset cursor. Byte-identical to
/// aggregations/raw.rs::render_fragment.
pub fn render_fragment(rows: &[Measurement], limit: usize) -> String {
    let mut out = String::new();
    if rows.is_empty() {
        out.push_str("<tr><td colspan=\"4\" class=\"muted\">Ingen aflæsninger i perioden.</td></tr>");
    }
    for r in rows {
        out.push_str(&format!(
            "<tr><td class=\"mono\">{}</td><td class=\"mono\" style=\"text-align:right\">{}</td><td>{}</td><td class=\"mono muted\">{}</td></tr>",
            fmt_date(&r.timestamp), esc(&r.value), esc(&r.unit), fmt_date(&r.timestamp),
        ));
    }
    let next_before = if rows.len() == limit {
        rows.last().map(|r| r.timestamp.clone()).unwrap_or_default()
    } else {
        String::new()
    };
    out.push_str(&format!(
        "<input id=\"m-before\" name=\"before\" type=\"hidden\" value=\"{}\" hx-swap-oob=\"true\">",
        esc(&next_before)
    ));
    out
}

pub fn error_fragment(message: &str) -> String {
    format!("<tr><td colspan=\"4\">Fejl: {}</td></tr>", esc(message))
}

fn fmt_date(s: &str) -> String {
    s.chars().take(16).collect()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
