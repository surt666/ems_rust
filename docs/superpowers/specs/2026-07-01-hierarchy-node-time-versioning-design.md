# Time-Versioned Node Hierarchy — Design Spec

**Date:** 2026-07-01
**Status:** Draft (design) — for review; no implementation yet
**Surface:** Hierarchy Rust Lambda (`crates/model` + `crates/services/hierarchy`),
DynamoDB `hierarchy_new` (account `339712745226`). Read-model consumers (frontend
node view, and the daq aggregations lambda when interpreting historical rollups).

## Context & goal

Today a node is a **single, overwritten** DynamoDB item. `node_to_item`
(`crates/model/src/repository/dynamodb/codec.rs`) writes `pk = sk = "<NodeId>"`
(e.g. `HN2#10003`) with `name`, `metadata` (M), `label`, `schema`, `created`, and
the `gsi1pk`/`gsi1sk` listing keys. A metadata edit (`logic::hierarchy` →
`put_node`) **replaces** that item — the previous definition is gone.

We want the node hierarchy to be **time-versioned**: every change appends an
immutable snapshot, the canonical row always reflects *now*, and any node (its
name, `metadata`, `label`, `schema`) can be read **as of an earlier instant**.

Why "versioned", not "event-sourced": the hierarchy changes rarely (human-driven
edits, not a high-rate event stream), and each node's state is small. Storing a
**full snapshot per version** — rather than a stream of deltas that must be folded
— is simpler to write, trivial to read (no replay), and cheap at this change rate.
We keep the *append-only* discipline (the project's stated architectural
principle) without the fold machinery of true event sourcing. See
[Alternatives](#alternatives-considered).

**Motivating use cases**
- Audit / "who changed what when" for a node's metadata and schema.
- Correctly interpret **historical aggregations**. Rollups in
  `measurements_aggregate` are written under the node path *as it was at ingest
  time*; to re-interpret or re-aggregate old data against the hierarchy **as it
  then existed** (labels, tariffs/factors stored in `metadata`, grouping), we need
  the node definition **as-of** that time — not today's.
- Reproducible reports: "the building's metadata on 2026-03-01".

**Non-goal (this phase):** reconstructing the *whole tree / permission graph* as of
T. That needs versioned **edges** and is a larger Phase 2 (below). Phase 1 answers
the user's ask — "older definitions of nodes and metadata".

## Precedent already in the codebase

Sensors are **already** temporal (`crates/model/src/domain/sensor_sk.rs`): the
current row is `sk = "active#<rfc3339Z>"`, superseded rows are bare
`sk = "<rfc3339Z>"` history; `replace_device` demotes the old to history and
writes a new active. This spec generalises that idea to nodes, with one
refinement (below) so the hot "read current" path stays a single `GetItem` and
point-in-time is a clean key-range query.

## The storage model

### Row layout (recommended: current-row preserved + append-only version log)

Per node, in the same `hierarchy_new` partition (`pk = "<NodeId>"`):

| Row | `sk` | Meaning | Mutability |
|---|---|---|---|
| **Current** | `"<NodeId>"` (unchanged) | The node *now* — exactly today's item, incl. `gsi1pk`/`gsi1sk` | upserted each write |
| **Version** | `"v#<valid_from rfc3339Z>"` | Immutable full snapshot of the versioned fields | append-only |

- The **current row is unchanged from today** — same key, same attributes, same
  `gsi1` listing keys. So every existing read (`get_node` = `GetItem`, list-by-level
  = `gsi1` query, children = edge query) and the write path keep working with **zero
  migration** to the hot path.
- Each mutating write **also appends** a `v#<now>` version row: a full snapshot of
  the versioned fields (`name`, `metadata`, `label`, `schema`, `path`) plus
  `valid_from = now`. Version `i` is effective over `[valid_from_i, valid_from_{i+1})`.
- **Version rows carry no `gsi1pk`/`gsi1sk`.** They must never appear in
  list-by-level or child queries — only the current row is indexed. (Critical: a
  version row with gsi keys would duplicate the node in every listing.)
- **Delete** appends a **tombstone** version (`deleted = true`, no snapshot body)
  and removes the current row (+ its gsi projection). "As-of T" then correctly
  reports "did not exist" when the newest version ≤ T is a tombstone.

`sk` namespaces stay disjoint and range-clean: current `"HN2#10003"`, versions
`"v#2026-…"`, edges `"has_label:building#…"` / `"has_sensor#…"`. A node's history
is the key range `sk BETWEEN "v#" AND "v#￿"`.

### Why this beats the sensor `active#`/bare scheme for nodes

The sensor scheme folds the "current" pointer into the timestamp namespace
(`active#…` sorts *after* bare timestamps), so a point-in-time lookup must reason
about the mixed prefix. Keeping the node's current definition at its existing
`sk = "<NodeId>"` gives us (a) an untouched `GetItem` hot path and gsi, (b) zero
read-migration, and (c) history isolated under a single `v#` range that is a pure
key-condition for as-of queries. (Sensors could later converge on this shape; not
required.)

### Reads

- **As-of now (default, hot path):** `GetItem(pk = id, sk = id)` — unchanged.
- **As-of T:** `Query(pk = id, sk BETWEEN "v#" AND "v#<T>", ScanIndexForward = false,
  Limit = 1)` → the newest version with `valid_from ≤ T`.
  - no row → node didn't exist yet at T;
  - tombstone → node was deleted before T;
  - else → decode the snapshot.
- **History list:** `Query(pk = id, sk begins_with "v#")` → all versions
  (timestamps, optionally field-level diffs computed in the read model).

### Writes — atomicity

Each mutation is a **`TransactWriteItems`** of two puts: upsert the current row +
put the `v#<now>` version (delete = put tombstone + delete current). The transaction
guarantees the current row and the log never diverge. At the hierarchy's change
rate the 2× write cost and transaction overhead are negligible. `valid_from = now`
(single time axis = transaction time; see [bitemporal](#open-questions)).

## Onion / layered changes

Following the layering the metadata-edit spec used (command → dispatch → logic →
codec/domain), versioning lives in the **repository adapter** so every write path
gets it automatically and the domain logic is untouched.

### Domain (`crates/model/src/domain`)
- New `node_sk.rs` (mirrors `sensor_sk.rs`): `NodeSk::{Current, Version(DateTime<Utc>)}`
  with `Display`/`parse` for `"<NodeId>"` vs `"v#<rfc3339Z>"`. Owns the `v#` prefix
  so the codec and query layer can't drift.
- New `NodeVersion { valid_from: DateTime<Utc>, node: Option<Node> }` (`None` =
  tombstone) as the return type for as-of / history reads. `Node` itself is
  unchanged.

### Repository (`repository/dynamodb/{codec,node}.rs` + `memory.rs`)
- `node_version_item(&Node, valid_from)` / `node_version_of_item` — encode/decode the
  snapshot rows (same attributes as `node_to_item` **minus** `gsi1pk`/`gsi1sk`, plus
  `valid_from`, plus optional `deleted`).
- `put_node` → **transactional dual-write** (current + `v#<now>`).
- `delete_node` → tombstone version + delete current (+ gsi projection) in one
  transaction.
- New injected reads: `get_node_asof(id, t) -> Option<NodeVersion>` and
  `list_node_versions(id) -> Vec<NodeVersion>`.
- The in-memory `Store` (`repository/memory.rs`) mirrors all of the above so the
  logic/service tests stay DB-free.

### Logic (`crates/model/src/logic/hierarchy.rs`)
- **Write logic is unchanged** — `add_node` / `update_metadata` / rename / move /
  `set_schema` still call the same injected `put_node`; versioning is now a
  storage guarantee, not something each command re-implements.
- Add thin `read_asof` / `history` helpers that take the new injected closures
  (same closure-injection style as the rest of the logic layer — no traits).

### Services (`crates/services/hierarchy`)
- **Command side unchanged** (writes version implicitly).
- **Query side (CQRS)** — new read actions on `GET /query/{action}`:
  - `get_node?id=<>&asof=<rfc3339>` — `asof` absent ⇒ current row; present ⇒ as-of
    query. HTML + `?format=json` per the existing convention.
  - `node_history?id=<>` — the version timeline (JSON, and an HTML fragment for the
    node view).
- Frontend (later): an "as-of" selector / history timeline on the node view
  (`render_node`), read-only; reuses the metadata section rendering with a
  disabled state.

## What is versioned

- **Phase 1 (this spec):** a node's own definition — `name`, `metadata`, `label`,
  `schema`, `path`. Answers "older definitions of nodes and metadata" directly.
  Because each snapshot stores its `path`, a node's **ancestors as-of T** are
  derivable from the as-of snapshot's path with no extra queries.
- **Phase 2 (larger, separate spec):** structural time-travel — versioned **edges**
  (`has_label` parent/child, `has_sensor`, access edges) so the **children of** a
  node and the **permission graph** can be reconstructed as-of T. Needs edge
  tombstones (deletes), move handling, and a tree-as-of walk. Sensors are already
  half-temporal (`active#`/history), so a node-as-of that includes attached sensors
  would query sensor history the same way. Flagged, not designed here.

## Migration / backfill

Existing nodes have only the current row. A one-time backfill writes an initial
`v#<created>` version per node (seed the log from the present state stamped with the
node's existing `created`), so history is complete from creation rather than from
first-edit-after-deploy. Idempotent; runnable as a small admin command over a
`gsi1` scan of current nodes. Alternatively, lazy seeding (first post-deploy write
creates the first version) if a complete pre-deploy history isn't required.

## Cost, retention, indexing

- **Storage:** append-only, but at human edit rates the version count per node is
  tiny; snapshots are small (node metadata). No `TTL` — history is the point.
- **Writes:** 2 items per mutation via one transaction; negligible at this rate.
- **GSI:** version rows are **not** projected to `gsi1` — listings and child
  queries are unaffected. This is a hard invariant, enforced in `node_version_item`.
- **Reads:** as-of adds one bounded key-range query (Limit 1); "now" stays a
  `GetItem`.

## Alternatives considered

1. **Full event sourcing (append commands/deltas, fold to state).** Rejected for
   this domain: rare changes + small state make snapshot-per-version simpler and
   read-trivial; folding buys nothing here and adds replay complexity. (Snapshots
   *are* an event log at version granularity — just materialised.)
2. **Sensor `active#`/bare scheme for nodes.** Rejected — mixes the current pointer
   into the timestamp namespace; keeping the current row at `sk = "<NodeId>"` is a
   cleaner hot path + as-of range (above).
3. **Separate `hierarchy_history` table / S3 snapshots.** Rejected for Phase 1 —
   same-partition version rows keep as-of a single query with transactional
   consistency and no cross-store fan-out. Revisit only if history volume ever
   dwarfs live data (it won't at this change rate).
4. **In-place `metadata` map of `{version → value}`.** Rejected — unbounded item
   growth, no per-field/whole-node as-of, breaks the 400 KB item limit eventually.

## Open questions

- **Bitemporal?** This design uses one axis: `valid_from = write time` (transaction
  time). If we ever need to *backdate* a correction (record that a change was
  effective earlier than entered), add a second `effective_from` axis. Deferred
  unless a real requirement appears.
- **Granularity of `node_history` diffs** — return whole snapshots, or compute
  field-level diffs in the read model? (Lean: snapshots on the wire, diffing is a
  frontend/read-model concern.)
- **Backfill completeness** — seed full history from `created` (recommended) vs lazy
  first-write seeding?
- **Phase 2 trigger** — do we need tree-as-of / permissions-as-of soon (drives
  edge versioning), or is per-node as-of sufficient for now?

## Out of scope (Phase 1)

Versioned edges / tree- and permission-as-of (Phase 2); bitemporal backdating;
frontend history UI beyond a basic as-of selector; any change to the sensor
temporal scheme; changes to the aggregations pipeline (it already writes at current
time — this spec only lets that data be *interpreted* against a historical node).

## Testing

- Domain: `node_sk` round-trips (`Current` vs `Version(ts)`); `NodeVersion` decode
  incl. tombstone.
- Repository (`memory::Store` + codec): put appends a version; update appends a
  second; `get_node_asof` returns the correct version at boundaries (before first,
  between, at exact `valid_from`, after last); tombstone ⇒ `None`; version rows
  never surface in `gsi1` listings.
- Logic: writes remain behaviourally identical (current row correct after N edits);
  as-of/history helpers return the expected sequence.
- Service: `get_node?asof=` and `node_history` routes (JSON + HTML), `asof` default
  = current, bad `asof` ⇒ 400.
- Backfill: idempotent; seeds exactly one version per existing node.
