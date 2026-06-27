pub mod forms;
pub mod node;
pub mod tree;

/// The `data-hx-request` value that tells HTMX to omit its default request
/// headers. Shared by every fragment that posts/gets over HTMX (node, forms,
/// tree) so the literal isn't copy-pasted. Rendered through maud's `(..)`, so the
/// `"`s come out `&quot;`-escaped (matching the previous inline literals).
pub(crate) const NO_HEADERS: &str = r#"{"noHeaders": true}"#;

/// Convert a JSON scalar to its display string. Objects/arrays/null collapse to
/// the empty string — callers only ever pass scalars, so the non-scalar arm is
/// inert (kept as a catch-all rather than a separate variant per type).
pub(crate) fn scalar_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Percent-encode a string component for use in URL paths/query values.
///
/// Only unreserved chars (`A-Z a-z 0-9 - _ . ~`) are left as-is; everything
/// else is `%XX`-encoded (uppercase hex).
pub fn pct(s: &str) -> String {
    let mut buf = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                buf.push(b as char);
            }
            _ => {
                buf.push_str(&format!("%{:02X}", b));
            }
        }
    }
    buf
}

/// Return `(icon_href, bar_class)` for a given level.
pub fn level_visual(level: model::domain::ids::Level) -> (&'static str, &'static str) {
    use model::domain::ids::Level;
    match level {
        Level::Hn0 => ("", ""),
        Level::Hn1 => ("#icon-partner", "partner"),
        Level::Hn2 => ("#icon-company", "company"),
        Level::Hn3 => ("#icon-property", "property"),
        Level::Hn4 => ("#icon-building", "building"),
        Level::Hn5 => ("#icon-area", "area"),
        Level::Hn6 => ("#icon-group", "group"),
        Level::Hn7 | Level::Hn8 | Level::Hn9 => ("#icon-area", "area"),
    }
}
