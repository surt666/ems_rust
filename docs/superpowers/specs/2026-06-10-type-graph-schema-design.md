# Type-Graph Hierarchy Schema (v2) — Design Spec

**Date:** 2026-06-10
**Status:** Approved (design); implementation plan pending
**Affects:** `crates/model`, `crates/services/hierarchy`, `scripts/` (migration), `features/` (BDD guardrails)
**Does NOT affect:** DAQ pipeline, `meter-identity` `hierarchy_path`, Flink/Glue, `measurements_aggregate`

## Context & problem

The hierarchy schema (one per company / hn2 node) is **level-keyed**: edges are
`(parentLevel → childLevel, label)`, metadata specs are keyed by level, and sensor placement is a
list of levels. The node *type* (group, property, building, area) exists only as the edge label.

This cannot express type-dependent rules. The required hierarchy is:

```
company  → { group, property, building }     ← building directly under company
group    → { building }
property → { building }
building → { area }                          ← regardless of the building's depth
```

A *building* may sit at depth hn3 (under company) or hn4 (under group/property). Level-keyed
edges make hn3 polymorphic (group | property | building) and then `hn3 → hn4` cannot say "area
under a building, building under a group" — it would wrongly admit `group → area` and
`building → building`. Level-keyed metadata breaks the same way (a building at hn3 would not
get the hn4 lat/lng rules).

**Decision:** replace the level-keyed schema with a **type graph**. Levels (`hnN`) stop being a
schema concept; a node's level is purely its depth (`parent.depth + 1`), derived at insert time.

Decisions made during design review:

- **Clean break + migration script** — new code reads/writes only v2; a one-time script
  migrates stored schemas and backfills node labels. No dual-format read path.
- **Strict DAG** — a type may not contain itself, directly or via a cycle. This also allows a
  static check that the deepest possible node fits within hn9.

## Domain model (`crates/model/src/domain/schema.rs`)

```rust
pub struct EdgeSpec {           // `label` field removed — the child type IS the label
    pub min: Option<i32>,
    pub max: Option<i32>,
}

pub struct Schema {
    pub version: u32,           // 2
    /// parent type → (child type → cardinality)
    pub edges: Vec<(String, Vec<(String, EdgeSpec)>)>,
    /// type → field specs (was: level → field specs)
    pub metadata: Vec<(String, Vec<(String, FieldSpec)>)>,
    /// types at which sensors may attach (was: Vec<Level>)
    pub sensors: Vec<String>,
}
```

Reserved type names:

- `"company"` — the hn2 node itself; the root of the type graph. May appear only as a parent.
- `"partner"` — reserved (hn0/hn1 remain outside the schema, hardcoded as today).

Query methods keep their shape, re-keyed by type:
`allowed_children(parent_type)`, `edge_between(parent_type, child_type) -> Option<EdgeSpec>`,
`metadata_for(type)`, `allows_sensors(type)`.

### `Schema::validate` (v2 rules)

1. Type names non-empty; no edge targets `"company"` or `"partner"`; no self-edges.
2. The type graph is a **DAG** (cycle → error).
3. Every type referenced anywhere (as child, in `metadata`, in `sensors`) is **reachable from
   `company`** — orphan rules are errors.
4. **Longest path from `company` ≤ 7 edges** (company is hn2; deepest node must fit hn9).
   Computed statically on the DAG at schema-save time.
5. Per parent: no duplicate child type; `min ≤ max` when both set.
6. Metadata field specs pass `validate_spec` (unchanged); no duplicate `sensors` entries.

## Node records

`Node` gains a `label: String` field, persisted as a `label` (S) attribute on the DynamoDB node
item: `"partner"` for hn1, `"company"` for hn2, the schema type for hn3+ (root may use
`"root"`). Today the type lives only on the incoming `has_<label>` edge; denormalizing onto the
node makes validation and queries O(1). `node_to_json` exposes `label` so the frontend (and the
future schema editor) can read a node's type without inferring from depth.

Edge items (`EdgeKind::HasLabel`), node ids, paths, and `gsi1` are **unchanged**.

## `add_node` semantics (`crates/model/src/logic/hierarchy.rs`)

- Child level is **always `parent.depth + 1`** — derived, never chosen. The existing `level`
  request parameter stays accepted but, if present, must equal `parent.depth + 1` (else
  `BadRequest`). `resolve_child_level`'s schema-walking logic is deleted.
- The `label` parameter names the **child type**. If omitted and the parent's type has exactly
  one allowed child type, that type is used; zero → "not allowed by schema"; several →
  "ambiguous; specify label" (same UX as today).
- Edge check: `edge_between(parent_node.label, child_type)` must exist. A building under company
  (hn3) and a building under group (hn4) validate their children against the *same*
  `building → area` rule.
- Metadata validates against `metadata_for(child_type)`. Max-cardinality still counts existing
  children with `HasLabel(child_type)`.
- hn0→hn1 (partner) and hn1→hn2 (company, schema required) flows keep their special-cased
  handling; the schema posted on company creation is validated with the v2 rules.
- `sensors.rs` checks `schema.allows_sensors(node.label)` instead of the node's level.

## Serialization

### DynamoDB codec (`repository/dynamodb/codec.rs`)

Same nesting style as v1, keys are type names, and the redundant inner label layer disappears
(one level flatter):

```
schema: M{
  version: N"2",
  edges:    M{ "company":  M{ "group": M{}, "property": M{}, "building": M{} },
               "group":    M{ "building": M{} },
               "property": M{ "building": M{} },
               "building": M{ "area": M{ "min": N"1" } } },
  metadata: M{ "building": M{ "lat": M{...}, "lng": M{...} } },
  sensors:  L[ S"building", S"area" ]
}
```

**Reading a `version: 1` schema fails with an explicit error** ("schema version 1 — run the v2
migration"), never a silent misparse. That is the clean break.

### JSON API (`crates/services/hierarchy/src/json.rs`)

`schema_to_json` / `schema_of_json` mirror the DDB shape 1:1:

```json
{ "version": 2,
  "edges":    { "company": { "group": {}, "property": {}, "building": {} },
                "group": { "building": {} },
                "property": { "building": {} },
                "building": { "area": { "min": 1 } } },
  "metadata": { "building": { "lat": { "type": "number", "required": true,
                                        "min": -90, "max": 90 } } },
  "sensors":  ["building", "area"] }
```

## Migration (one-time script, `scripts/`)

Python (matching existing repo tooling), against the `hierarchy_new` table in the hierarchy
account (`339712745226`). **Dry-run mode prints every transformed item for review before any
write.**

1. **Schemas** — for each hn2 node with a v1 schema, labels become types:
   - Parent side: hn2 → `"company"`; for deeper levels, the parent types are the labels of the
     edges *into* that level.
   - SeedCo01's `hn2→hn3{group,property}, hn3→hn4{building}, hn4→hn5{area}` becomes
     `company→{group,property}, group→{building}, property→{building}, building→{area}`.
   - Level-keyed `metadata`/`sensors` map to the types at that level. If a level hosts several
     types, the rules replicate to each type and the script flags it for manual review.
   - `version` → 2. Cardinalities (`min`/`max`) carry over per edge.
2. **Node labels** — every hn3+ node gets `label` backfilled from its incoming `has_<label>`
   edge; hn1 → `"partner"`, hn2 → `"company"`.
3. The v1→v2 transform is information-preserving for all existing data: abilities added by v2
   (e.g. building under company) did not exist in v1, so migrated schemas behave identically
   until edited.

**Rollout:** the deployment targets are test accounts — breaking changes are acceptable. Deploy
the new code and run the migration in either order; any window where deployed code and stored
data disagree (v1 schemas unreadable, node `label` not yet backfilled) is tolerated. No fallback
read paths are built; the migration script is still dry-run + reviewed before writing.

The migration transform is implemented as a pure function and unit-tested against the SeedCo01
schema fixture before touching the table.

## What does NOT change

- Edge storage format (`has_<label>`), node ids, paths, `gsi1`.
- The DAQ pipeline: `meter-identity` `hierarchy_path`, the hn1..hn9 columns, Flink enrichment,
  the Glue rollup. Paths stay dense — now *guaranteed* dense, since a child is always exactly
  one level deeper than its parent. The `features/` BDD invariant notes are updated to cite the
  type graph as the reason the dense-path assumption holds.
- Partner/company creation flow shape (schema still required exactly on hn2 creation).
- The masterdata create-node dialog keeps working unmodified: it posts type names that match the
  migrated schema types; invalid choices are rejected by the backend as today.

## Testing

- **`schema.rs`** — validate: cycle rejection, unreachable type, longest path > 7, duplicate
  child type, `min > max`, sensors referencing an unknown type, edge targeting
  `company`/`partner`, self-edge, empty type name. Query methods re-keyed by type.
- **`hierarchy.rs`** — the driving scenario: one schema allowing building under company *and*
  under group; assert a building's children validate as `area` at both depths; omitted label
  with one vs several allowed child types; `level` param mismatch rejected; metadata required
  fields enforced per type at variable depth.
- **`sensors.rs`** — sensor attach allowed/denied by node type at variable depth.
- **Codec** — v2 roundtrip; v1 read → explicit migration error message.
- **`json.rs`** — v2 JSON roundtrip; malformed edges/metadata/sensors rejected.
- **Migration** — pure-function transform tested on the SeedCo01 fixture (real production item).
- **BDD** — new `features/hierarchy/schema_type_graph.feature` capturing the rules above as
  guardrails; update the ENFORCEMENT note in `features/data_pipeline/meter_enrichment.feature`
  and the dense-level note in `features/data_pipeline/measurements_rollup.feature` to reference
  the type-graph model.

## Follow-up (separate spec — committed, not part of this change)

**Graphical schema editor** for company (hn2) creation: a visual type-graph builder — nodes =
types, arrows = allowed containment with cardinality, per-type metadata fields and sensor
toggles — that posts the v2 schema JSON with `create company`. Depends on: v2 API live, `label`
exposed in node JSON. Also folds the masterdata type dropdown onto schema-driven choices
(allowed child types of the selected parent). To be brainstormed once this change is deployed
and verified.

## Out of scope

- Relation names distinct from child types (property-graph style `company -has_annex-> building`).
  Types subsume labels; revisit only with a concrete need.
- Per-type permissions or UI presentation hints in the schema.
- Any DAQ-side change.
