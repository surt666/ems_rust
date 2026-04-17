# Onion architecture + hierarchy domain — design

**Date:** 2026-04-17
**Status:** approved (pre-plan)
**Scope:** first-pass skeleton of `lib/` plus a concrete hierarchy domain (no users or permissions)

## 1. Purpose and shape

Set up an onion-style architecture for an OCaml AWS Lambda so that:

- Domain types and rules live at the center, ignorant of persistence, HTTP, and AWS.
- Business logic is pure — it expresses effects via OCaml 5 algebraic effects, not by calling a repository directly.
- Repositories are effect **handlers**, not modules called from logic. The first concrete handler targets DynamoDB via `smaws-clients`. A second in-memory handler backs unit and logic tests.
- The outermost layer (the Lambda binary) wires the chosen handler around the logic call and exposes a CQRS HTTP surface via API Gateway v2.

The first implementation slice is intentionally small: hierarchy nodes only, no users, no permissions, no sensors, no measurements. The design reserves space for those additions without requiring rework.

## 2. Module layout

```
lib/
  domain/                 # pure types — no effects, no IO, no serialization
    level.ml              # type hn = Hn0 .. Hn9 + depth
    node_id.ml            # record { level; uuid }, parse/render HN<n>#<uuid>
    metadata.ml           # field_type, field_spec, value bag, validator
    schema.ml             # edges DAG + per-level metadata specs + self-check
    node.ml               # HierarchyNode record and builders
    errors.ml             # domain error sum type

  effects.ml              # flat effect declarations (single module)

  logic/                  # pure functions that `perform` effects
    hierarchy.ml          # add_node, delete_node, get_node, list_children
    schema_check.ml       # validate metadata, enforce DAG edge + cardinality

  repo/                   # effect handlers — this is the "repo"
    memory.ml             # in-memory handler for tests/local dev
    dynamo.ml             # smaws handler for production
    codec.ml              # node <-> dynamo item, schema <-> map, edge <-> item

  api/                    # CQRS dispatch + JSON envelope
    command.ml            # POST /command parser + dispatcher
    query.ml              # GET /query/{action} parser + dispatcher
    json.ml               # API-Gateway v2 envelope + error formatting

bin/
  main.ml                 # Lambda_runtime.start, Eio env + smaws client,
                          #   Repo.Dynamo.run + Api dispatch

test/
  test_domain.ml          # pure domain tests (alcotest)
  test_logic.ml           # logic under Repo.Memory (alcotest + base_quickcheck)
  test_api.ml             # JSON-in/JSON-out through Api + Repo.Memory
  test_codec.ml           # Dynamo item <-> Node round-trip via fixtures

itest/                    # integration tests, own dune alias
  test_dynamo.ml          # real DynamoDB round-trips via smaws + Repo.Dynamo
```

### Dependency rule

| Layer    | May depend on                      |
|----------|------------------------------------|
| `domain` | stdlib, `uuidm`, `ptime`, `yojson` |
| `effects`| `domain`                           |
| `logic`  | `domain`, `effects`                |
| `repo`   | `domain`, `effects`, `smaws-*`     |
| `api`    | `domain`, `effects`, `logic`       |
| `bin`    | everything (composition only)      |

`logic` never imports `repo`. `api` never imports `repo`. Handler selection is only visible in `bin/main.ml`.

### File granularity

One type per file in `domain/`. OCaml's `file = module` mechanic means `Node.t`, `Node.create`, `Node.pp`, and a dedicated `node.mli` all live under one module without boilerplate, and the file-level dependency DAG makes layer rules inspectable at a glance. This is idiomatic.

## 3. Effect surface (flat)

All effects are top-level constructors of `_ Effect.t`. Flat and minimal; split later if handlers for reads and writes start to diverge.

```ocaml
(* lib/effects.ml *)
open Domain

type _ Effect.t +=
  (* reads *)
  | Get_node       : Node_id.t -> Node.t option Effect.t
  | List_children  : Node_id.t * string option -> Node.t list Effect.t
      (* second arg is an optional has_<label> prefix filter *)
  | Get_schema     : Node_id.t -> Schema.t option Effect.t

  (* writes *)
  | Put_node       : Node.t -> unit Effect.t
  | Put_edge       : { from_ : Node_id.t; to_ : Node_id.t; label : string }
                   -> unit Effect.t
  | Delete_node    : Node_id.t -> unit Effect.t
      (* edge cleanup via gsi1 query; non-recursive in v1 *)

  (* determinism helpers, isolated so tests can fix them *)
  | Gen_uuid       : unit -> Uuidm.t Effect.t
  | Now            : unit -> Ptime.t Effect.t
```

### Caveat flagged up front

OCaml 5 effects are **not tracked in types**. A function that performs an effect looks identical to a pure one; unhandled effects raise `Effect.Unhandled` at runtime. The architectural purity that this gives us is a matter of convention (logic files do not import `repo/`) plus tests, not a compiler proof. This is a deliberate trade-off — we pick it for the ergonomics of a single top-level handler wiring, not for type-level purity.

## 4. Storage model (DynamoDB, single table)

One table. Two item shapes distinguished by `type`. GSI1 inverts every edge to enable reverse lookups.

```
Table: hierarchy
  PK:  pk (S)
  SK:  sk (S)
  GSI1:
    gsi1pk (S)
    gsi1sk (S)
```

### Vertex item

```jsonc
{
  "pk":       "HN4#<uuid>",
  "sk":       "HN4#<uuid>",              // self-anchor
  "type":     "node",
  "name":     "Building A",
  "parent":   "HN3#<uuid>",              // single-hop parent, nullable on root
  "created":  "2026-04-17T12:00:00Z",
  "metadata": { /* map validated against schema.metadata[level] */ },
  "schema":   { /* hn2 only — see Part 5 below */ }
}
```

Level is *not* stored as a separate attribute; it is derivable from `pk`. The reserved root id is `HN0#root`.

### Edge item

```jsonc
{
  "pk":       "HN3#<parent-uuid>",
  "sk":       "has_building#HN4#<child-uuid>",
  "type":     "edge",
  "label":    "building",
  "created":  "2026-04-17T12:00:00Z",

  "gsi1pk":   "HN4#<child-uuid>",
  "gsi1sk":   "HN3#<parent-uuid>"
}
```

### Query patterns

| Use case                                | Query                                                                 |
|-----------------------------------------|-----------------------------------------------------------------------|
| Direct children of node X               | `Query pk = X, sk begins_with "has_"`                                 |
| Direct children of kind `building`      | `Query pk = X, sk begins_with "has_building#"`                        |
| Parent of node Y (breadcrumbs)          | `GetItem Y` → read `parent` attribute                                 |
| Reverse lookup on any edge into Y       | `Query gsi1pk = Y`                                                    |
| Exact node by id                        | `GetItem { pk = id, sk = id }`                                        |

## 5. Company schema (embedded on hn2 nodes)

Per-company schema is a map attribute on the hn2 vertex row. No separate `SCHEMA` item.

```jsonc
"schema": {
  "root":    "hn0",
  "version": 1,
  "edges": {
    "hn2": { "hn3": { "label": "property" } },
    "hn3": { "hn4": { "label": "building", "min": 1 } },
    "hn4": { "hn5": { "label": "area" } }
  },
  "metadata": {
    "hn4": {
      "lat": { "type": "number", "required": true, "min": -90, "max": 90 },
      "lng": { "type": "number", "required": true, "min": -180, "max": 180 }
    }
  }
}
```

### Metadata FieldType

| type        | constraints honoured                                 |
|-------------|------------------------------------------------------|
| `string`    | `min_len`, `max_len`                                 |
| `number`    | `min`, `max` (f64)                                   |
| `integer`   | `min`, `max` (i64)                                   |
| `boolean`   | —                                                    |
| `timestamp` | must parse as RFC 3339                               |
| `enum`      | `one_of` (non-empty list of allowed strings)         |

All fields may carry `required`. Unknown fields on an instance are **ignored**, matching `schema.md`'s policy. Metadata values are `Yojson.Safe.t`.

### Schema self-check

`Schema.validate` runs at load time and rejects:

- any edge where `parent.depth >= child.depth` (cycle prevention),
- any `enum` spec with empty `one_of`,
- any level key outside `hn0..hn9` (or the reserved `S` pseudo-level for sensors; see §9).

### Caveat: label in sk

The child `sk` carries the edge label (`has_building#…`). Renaming a label in the schema requires rewriting the affected edge rows. Acceptable for v1 under the assumption that schemas are stable once deployed. If that assumption weakens, two fixes: (a) add a tiny `rename_label` command that performs the rewrite; (b) drop the label from the `sk` and keep only the child level (`has_HN4#…`).

## 6. Validation flow for `add_node`

Input: `{ parent_id; level; name; metadata }`.

1. `perform Get_node parent_id`. If `None` → `Not_found parent_id`.
2. Check `parent.level.depth < level.depth`.
3. Find the company schema by walking `parent.parent` up to the nearest hn2 ancestor (or the parent itself, if it is hn2). `perform Get_node` at each step. Worst case 7 hops. If no hn2 found → `Schema_missing`.
4. Look up `schema.edges[parent.level][level]`. If absent → `Validation "edge parent.level → level not allowed"`.
5. Pull `label` from that edge entry; it becomes the `has_<label>` prefix.
6. Validate `metadata` against `schema.metadata[level]` — per-field type, required, and constraint checks. Collect all failures, not just the first.
7. `perform List_children (parent_id, Some ("has_" ^ label ^ "#"))` to count existing. Enforce `min`/`max` cardinality from the edge entry.
8. `perform Gen_uuid` → child uuid. `perform Now` → created.
9. `perform Put_node child` + `perform Put_edge { from_ = parent_id; to_ = child.id; label }`. The Dynamo handler batches these into a single `TransactWriteItems`; the Memory handler just mutates a hashtable.
10. Return the new node.

## 7. API surface (CQRS over API Gateway v2)

Two routes. Dispatch is a sum-type match; no router library.

### Commands (v1)

```jsonc
POST /command
{
  "action":    "add_node",
  "parent_id": "HN3#4b6a...",
  "level":     "hn4",
  "name":      "Building A",
  "metadata":  { "lat": 55.68, "lng": 12.57 }
}
→ 200 { "id": "HN4#...", "name": "Building A", "parent": "HN3#...", "created": "..." }

POST /command
{ "action": "delete_node", "id": "HN4#..." }
→ 200 { "deleted": "HN4#..." }
```

### Queries (v1)

```
GET /query/get_node?id=HN4#4b6a...
→ 200 { "id": "...", "name": "...", "parent": "...", "metadata": {...}, "schema": {...}? }

GET /query/list_children?parent=HN3#4b6a...
GET /query/list_children?parent=HN3#4b6a...&label=building
→ 200 { "children": [ {...}, {...} ] }
```

Each action has a single `parse` and a single `run` per action, co-located in `api/command.ml` or `api/query.ml`. Adding an action is "add a variant + a case" in one file.

## 8. Error model

Logic returns `('a, Errors.t) result`. Effect handlers convert smaws exceptions to values before continuing; they never raise into logic.

```ocaml
(* lib/domain/errors.ml *)
type t =
  | Not_found       of Node_id.t
  | Bad_request     of string
  | Validation      of Validation_error.t
  | Schema_missing  of Node_id.t
  | Conflict        of string
  | Internal        of string
```

API boundary mapping:

| Error code          | HTTP | Example                                          |
|---------------------|------|--------------------------------------------------|
| `bad_request`       | 400  | malformed JSON, missing field, invalid level     |
| `not_found`         | 404  | parent not found, get_node miss                  |
| `validation_failed` | 422  | schema rejects metadata, cardinality breach      |
| `schema_missing`    | 409  | walk to hn2 found no schema                     |
| `conflict`          | 409  | optimistic lock collision on schema version      |
| `internal`          | 500  | anything else, including bubbled smaws errors    |

Response body:

```jsonc
{ "error": { "code": "validation_failed", "message": "...", "details": {...} } }
```

## 9. Sensors — design annex (not in v1)

v1 does not implement sensors. Choices below make sure the v1 table and schema shape do not need breaking changes when sensors arrive.

### Item shapes

```jsonc
// sensor vertex — shares the table with HN<n>#<uuid> items
{
  "pk":          "S#<uuid>",
  "sk":          "S#<uuid>",
  "type":        "sensor",
  "name":        "kWh meter",
  "unit":        "kWh",
  "external_id": "...",
  "created":     "..."
}

// sensor edge — same adjacency shape as hierarchy edges
{
  "pk":      "HN4#<building-uuid>",
  "sk":      "has_sensor#S#<sensor-uuid>",
  "type":    "edge",
  "label":   "sensor",
  "created": "...",
  "gsi1pk":  "S#<sensor-uuid>",
  "gsi1sk":  "HN4#<building-uuid>"
}
```

No new table, no new index. The inversion GSI already gives "given a sensor, find its host node" in one query.

### Schema support

`schema.edges` gains a reserved pseudo-level `"S"`. A company that allows sensors on buildings writes:

```jsonc
"edges": {
  "hn4": {
    "hn5": { "label": "area" },
    "S":   { "label": "sensor" }
  }
}
```

`S` has a pseudo-depth of 10 (greater than `hn9`), preserving the depth-ordering invariant. A new `add_sensor_to_node` command targets the `S` edge entry specifically; `add_node` continues to reject `S` because its caller passes a real `Level.t`.

### Transitive "all sensors under this subtree" query

Decision deferred to sensor-implementation time. Three candidate shapes:

| Option                               | Write cost          | Read cost                     | Trade-off                                             |
|--------------------------------------|---------------------|-------------------------------|-------------------------------------------------------|
| Walk the tree recursively            | 0                   | O(subtree × RTT)              | Simple; degrades on large trees                       |
| Ancestor-closure GSI                 | O(depth) per attach | O(result) one query           | Recommended; denormalized but bounded, moves tractable|
| Re-introduce hierarchy_path + GSI    | O(depth) per attach | O(result) one query           | Matches EMS; path coupling complicates moves          |

Recommended when implemented: ancestor-closure GSI. The v1 table shape supports all three.

### Commands/queries reserved

`add_sensor_to_node`, `detach_sensor`, `list_sensors(node_id, transitive?)`. Not in v1.

## 10. Testing strategy

### Unit tests (`test/`, offline, run by `dune runtest`)

- **`test_domain.ml`** — pure: `Schema.validate` accepts/rejects known schemas, `validate_metadata` per `FieldType`, `Node_id` string round-trip, depth ordering.
- **`test_logic.ml`** — logic under `Repo.Memory`. Alcotest + base_quickcheck properties:
  - every `add_node` result is retrievable via `get_node`,
  - `list_children` returns exactly what `add_node` added, grouped under the correct label,
  - `delete_node` removes incoming edges discovered via gsi1 (checked by Memory handler's in-memory gsi1),
  - validation errors never produce partial state.
- **`test_api.ml`** — HTTP JSON in, HTTP JSON out, wired through `Repo.Memory`. Asserts status codes and envelope shape.
- **`test_codec.ml`** — `Dynamo.Codec` round-trips against fixture items. Verifies `schema`-map and `metadata`-map fidelity.

### Integration tests (`itest/`, require AWS creds, run under `@itest` alias)

Separate binary and separate alias so `dune runtest` stays fast and offline.

```dune
; itest/dune
(executable
 (name test_dynamo)
 (libraries ocaml_lambda_test smaws-clients alcotest eio_main))

(rule
 (alias itest)
 (deps (env_var AWS_REGION) (env_var ITEST_DYNAMO_TABLE))
 (action (run %{exe:test_dynamo.exe})))
```

- Run via `dune build @itest`. Requires `AWS_REGION` and `ITEST_DYNAMO_TABLE`; AWS creds come via the standard provider chain (env, profile, instance role).
- Dedicated table matching prod shape (`hierarchy-itest`). UUID-based ids make parallel runs safe. Each test cascade-deletes its roots on teardown — no global wipe.
- Coverage: `add_node` → `get_node` round-trip with metadata and hn2 schema fidelity; `list_children` with and without label filter; cascade delete leaves no edges (verified via `gsi1pk`); `TransactWriteItems` atomicity; schema validation prevents writes.

### CI lifecycle (guidance)

1. PR pipeline: `dune runtest` — offline, must pass to merge.
2. Post-merge: build and deploy to a staging environment.
3. Post-deploy smoke: `dune build @itest` against the staging table. Green promotes to prod.

## 11. Non-goals for v1

- Users, permissions, edge kinds other than `has_<label>`.
- Sensors, measurements, aggregations (see §9 for reserved design).
- Recursive cascade delete. `delete_node` removes the node and its direct edges; descendants become orphaned. Documented on the effect.
- Optimistic schema versioning via `ConditionExpression` on `version`. No `update_schema` command in v1.
- Caching schema per-company in the warm Lambda container.
- A GSI that indexes by level (`"list all hn4 nodes globally"`). All v1 traversals start from a known parent id.

## 12. Open questions (resolvable during implementation)

- Exact shape of `Validation_error.t` (field paths, aggregated failures). Current plan: a list of `{ path; message }`.
- Whether `Get_node` / `Put_node` should go through a small lru in the Memory handler for larger logic tests. Only if tests get slow.
- Whether `TransactWriteItems` is worth the extra API call when only one item is written (i.e., future flows where `Put_edge` is optional). Current plan: always transact, for simplicity.

## 13. Dependencies to add

- `uuidm` — UUID generation and parsing.
- `ptime` (already pulled in transitively by smaws) — timestamps.
- `smaws-clients` — DynamoDB access via Eio.
- `base_quickcheck`, `alcotest` — already present from earlier work.
