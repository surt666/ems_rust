//! Raw meter-reading value objects for the `/measurements` (Datatilegnelse) view.
//!
//! These are read-side domain types: a [`Measurement`] is one raw reading (the
//! newest version for a timestamp), and [`MeasurementQuery`] is the request for a
//! page of them. The data source (Athena today, Redshift later) is a repository
//! adapter under `repository::measurements`, injected into the service handler —
//! the types themselves are source-agnostic.

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

/// One raw meter reading — the newest version (`max_by(value, ingested_time)`)
/// for a given timestamp. `value`/`unit` are pre-formatted strings as read back.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Measurement {
    /// Reading timestamp, `"YYYY-MM-DD HH:MM:SS"` (UTC).
    pub timestamp: String,
    /// Formatted reading value.
    pub value: String,
    pub unit: String,
}

/// A request for one page of raw readings for a single sensor, newest-first.
///
/// `from`/`to`/`before` are un-parsed strings (RFC3339, `YYYY-MM-DD[ HH:MM:SS]`,
/// or date-only) that the adapter validates and turns into query literals; `now`
/// is injected so defaults (`from = now-1d`, `to = now`) are deterministic in
/// tests.
#[derive(Debug, Clone)]
pub struct MeasurementQuery {
    pub daq_id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Keyset cursor — only timestamps strictly `< before` (for "load more").
    pub before: Option<String>,
    pub limit: usize,
    pub now: DateTime<Utc>,
}
