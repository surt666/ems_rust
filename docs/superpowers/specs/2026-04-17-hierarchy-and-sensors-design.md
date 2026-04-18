# Hierarchy and Sensors Design

**Status:** current hierarchy implemented; sensors are the next-step extension.
**Scope:** documents the node model, per-company schema, DynamoDB layout, and how sensors with formulas will be added.

---

## 1. Node model

A tree of typed nodes, rooted at a global singleton.

| Level | Role              | Notes                                       |
|-------|-------------------|---------------------------------------------|
| hn0   | root              | singleton, id = `ROOT#root`                 |
| hn1   | partner           | business tenant (e.g. "Acme Partner")       |
| hn2   | company           | **owns a schema** that shapes everything below |
| hn3+  | schema-defined    | meaning is whatever the hn2 schema declares |

Every node has:

- `id` — `<LEVEL>#<uuid>` (or `ROOT#root` for the root)
- `level` — hn0..hn9
- `name` — human label
- `parent` — parent node id (root has no parent)
- `created` — RFC 3339 timestamp
- `metadata` — free-form JSON, validated against the schema for this level
- `schema` — only present on hn2 (the company)

Depth must strictly increase from parent to child: an hn3 cannot live under another hn3. Cross-branch shortcuts (e.g. hn2 → hn4 skipping hn3) are allowed **if** the schema declares that edge.

---

## 2. Per-company schema

The schema lives on the hn2 node and governs *that* company's subtree only. Sibling companies can have completely different shapes.

```ocaml
type edge_spec = { label : string; min : int option; max : int option }

type t = {
  version  : int;
  edges    : (Level.t * (Level.t * edge_spec list) list) list;
  metadata : (Level.t * (string * Metadata.field_spec) list) list;
}
```

### 2.1 Edges

Each `(parent_level, [(child_level, specs)])` entry declares one or more labeled edges between two levels. `specs` is a **list** so a single `(parent, child)` level pair can host several semantically distinct relationships — e.g. at hn2→hn3 a company may declare both `property` and `group`.

`min` / `max` express cardinality per parent. Only `max` is enforced at add time; `min` is informational (a future "finalize" check).

When multiple labels exist between two levels, callers must disambiguate via `~label`:

```ocaml
Hierarchy.add_node ~label:"property" ~parent:company_id ~level:Hn3 ...
```

### 2.2 Metadata specs

`metadata` maps each level to the required/optional JSON fields for nodes at that level. Supported types today:

- `Number { min; max }` — bounded floats
- `Enum { one_of }` — string whitelist
- `String` — opaque
- (extend here as needed)

### 2.3 Schema resolution

`Schema_check.find_for id` walks up from `id` until it hits an hn2 with a schema. The hn2's schema is authoritative for the whole subtree rooted at that company. Orphan nodes (no hn2 ancestor) get `Schema_missing`.

---

## 3. Example: two companies, two shapes

Both companies hang off the same partner, but declare different schemas:

```mermaid
graph TD
  root["root (hn0)"] -->|partner| P["Acme Partner (hn1)"]
  P -->|company| RE["RealEstateCo (hn2)<br/>schema: property/group → building → area"]
  P -->|company| CP["ChargeCo (hn2)<br/>schema: parkinglot → chargingpool → charger → plug"]

  RE -->|property| HQ["HQ Property (hn3)"]
  RE -->|property| WH["Warehouse Property (hn3)"]
  RE -->|group| RG["Region Group (hn3)"]
  HQ -->|building| HBA["HQ Building A (hn4)<br/>lat, lng"]
  HQ -->|building| HBB["HQ Building B (hn4)"]
  WH -->|building| WBA["Warehouse Building A (hn4)"]
  WH -->|building| WBB["Warehouse Building B (hn4)"]
  HBA -->|area| AREA["HQ Parking A (hn5)"]

  CP -->|parkinglot| LOT["Parking Lot North (hn3)"]
  LOT -->|chargingpool| POOL["Pool A (hn4)"]
  POOL -->|charger| C1["CP-01 (hn5)<br/>power_kw=150, ccs"]
  POOL -->|charger| C2["CP-02 (hn5)<br/>power_kw=50, type2"]
  C1 -->|plug| P1A["Plug 01-A (hn6)"]
  C1 -->|plug| P1B["Plug 01-B (hn6)"]
  C2 -->|plug| P2A["Plug 02-A (hn6)"]
```

Two invariants to notice:

1. **RealEstateCo has no `charger` or `plug`** — its schema does not declare those labels, so `Hierarchy.add_node` rejects them with `Validation`.
2. **ChargeCo has no `property`** — same reason, the other direction. This is asserted as a negative test in `itest/test_dynamo.ml`.

---

## 4. DynamoDB layout

Single-table design, table name in `$ITEST_DYNAMO_TABLE` (prod: `hierarchy_new`).

**Hierarchy rows:**

| Attribute | Node row                       | Edge row                                |
|-----------|--------------------------------|-----------------------------------------|
| `pk`      | `<LEVEL>#<uuid>`               | `<parent_pk>`                           |
| `sk`      | `NODE#`                        | `has_<label>#<child_pk>`                |
| `gsi1pk`  | `<parent_pk>` (or `ROOT#root`) | — (edges don't need the inverted index) |
| `gsi1sk`  | `CHILD#<LEVEL>#<uuid>`         | —                                       |

**Sensor rows:**

| Attribute | Active assignment row             | History row        | Sensor edge row       |
|-----------|-----------------------------------|--------------------|-----------------------|
| `pk`      | `S#<uuid>`                        | `S#<uuid>`         | `<parent_node_pk>`    |
| `sk`      | `active#<iso8601>`                | `<iso8601>`        | `has_sensor#S#<uuid>` |
| `gsi1pk`  | `<parent_node_pk>`                | —                  | —                     |
| `gsi1sk`  | `active#<iso8601>`                | —                  | —                     |

Only the active assignment row carries GSI attributes (sparse index). History rows have no GSI attributes and are never modified after creation.

`gsi1` is used for `list_children` — query `gsi1pk = parent_pk` returns every child in one round trip.

`delete_node` is cascading: walks the subtree via `list_children`, removes every edge and every node row.

---

## 5. Rules summary

| Rule | Where enforced |
|------|----------------|
| Parent depth < child depth | `Hierarchy.add_node` |
| Edge label is declared by the hn2 schema | `Hierarchy.add_node` via `Schema.edges_between` |
| When multiple labels are valid, caller must pass `~label` | `Hierarchy.add_node` |
| Metadata matches the level's field spec (types, bounds, required) | `Metadata.validate` |
| `max` cardinality per parent | `Hierarchy.add_node` |
| Schema self-consistency (depth order, unique labels per edge, valid metadata specs) | `Schema.validate` |

---

## 6. Sensors (next step) — extension design

Goal: attach physical meter readings to nodes. Schema decides which levels can host sensors; a sensor has its own identity independent from the node it is attached to, plus a formula that may reference other sensors.

### 6.1 Data shape

A sensor's logical identity is a stable `uuid`. Physical daq devices break and get replaced; each replacement records the old device as a history row and promotes the new one to active. Only the active row carries GSI attributes, so querying by node returns only live sensors with no filter expression.

**Sensor rows** — one partition per logical sensor (`pk = S#<uuid>`):

| Attribute        | Active assignment                                  | History row     | Purpose |
|------------------|----------------------------------------------------|-----------------|---------|
| `pk`             | `S#<uuid>`                                         | `S#<uuid>`      | logical sensor identity |
| `sk`             | `active#2026-04-18T10:00:00Z`                      | `2026-03-01T...`| `active#` prefix marks the live device; plain timestamp for history |
| `gsi1pk`         | `<parent_node_pk>`                                 | —               | sparse — only active rows appear in the GSI |
| `gsi1sk`         | `active#2026-04-18T10:00:00Z`                      | —               | |
| `daq_address`    | `daq:adeunis_pu_v1:123:0018b210000191c7:counter_a` | (same)          | physical device address |
| `hierarchy_path` | `P1#C1#PR1#B2`                                     | (same)          | cached human path; denormalized |
| `purpose`        | `Electricity` \| `Heat` \| …                       | (same)          | what is being measured |
| `meter_type`     | `counter` \| `gauge`                               | (same)          | semantic kind |
| `unit`           | `kWh`, `m3`, …                                     | (same)          | optional, informational |
| `formula`        | (see §6.3)                                         | (same)          | expression evaluated at read time |

Query patterns:
- **Active sensors on a node**: GSI query `gsi1pk = HN5#uuid`, `gsi1sk begins_with active#` — no filter, no dedup, exactly one row per sensor.
- **Full history for a sensor**: query `pk = S#<uuid>` — returns the active row plus all history rows sorted by sk.

**Device replacement** — because sk is part of the primary key it cannot be updated in place. A replacement is a 3-op `TransactWriteItems` (read old active row first, then in one transaction):
1. **Delete** `{pk: S#uuid, sk: active#<old-timestamp>}`
2. **Put** `{pk: S#uuid, sk: <old-timestamp>, ...attributes...}` — demotes old device to history with no GSI attributes; the timestamp is preserved so the row records when that device *was* active from
3. **Put** `{pk: S#uuid, sk: active#<now>, gsi1pk: ..., gsi1sk: active#<now>, daq_address: <new>, ...}` — new active row timestamped at the moment of replacement

The history rows therefore form a timeline: each plain-timestamp row shows which physical device was active and from when, up until the next replacement.

Replacements are rare (a broken device is an exceptional event), so the read-then-transact cost is acceptable.

**Sensor edge row** (written into the parent node's partition):

| Attribute | Example                | Purpose |
|-----------|------------------------|---------|
| `pk`      | `<parent_node_pk>`     | the hierarchy node this sensor is attached to |
| `sk`      | `has_sensor#S#<uuid>`  | written once at attachment; survives device replacements |

**Flink-optimized table** — a separate DynamoDB table fed by a DDB stream from the hierarchy table. The flink pipeline reads from this table using a partition scheme tuned for high-throughput reads. The hierarchy table remains the source of truth; the flink table is a derived projection.

### 6.2 Schema extension

Extend `Schema.t` with a third map, sibling to `edges` and `metadata`:

```ocaml
type sensor_slot = {
  kind     : string;            (* e.g. "electricity", "water" *)
  min      : int option;
  max      : int option;
  meter_type : [ `Counter | `Gauge | `Either ];
}

type t = {
  version  : int;
  edges    : (Level.t * (Level.t * edge_spec list) list) list;
  metadata : (Level.t * (string * Metadata.field_spec) list) list;
  sensors  : (Level.t * sensor_slot list) list;   (* NEW *)
}
```

Then `Hierarchy.attach_sensor ~parent ~kind ...` mirrors `add_node`:
- resolve schema via `Schema_check.find_for`
- look up `sensor_slot`s declared for the parent's level
- disambiguate by `~kind` when multiple slots exist
- enforce `max` the same way cardinality is enforced for edges

Levels with no entry in `sensors` simply cannot host meters, and attachment fails with `Validation`.

### 6.3 Formula model

Default is identity — "what the meter reports" — which must be representable without the user writing anything. Anything else is an expression referencing this sensor and/or others by local alias.

```ocaml
type formula =
  | Identity                        (* = 1 * value, the common case *)
  | Expr of {
      ast  : expr;
      refs : (string * string) list;   (* alias -> sensor uuid, e.g. "S1" -> "<uuid>" *)
    }

and expr =
  | Num  of float
  | Self                            (* this meter's value *)
  | Ref  of string                  (* alias from refs, e.g. "S1" *)
  | Neg  of expr
  | Add  of expr * expr
  | Sub  of expr * expr
  | Mul  of expr * expr
  | Div  of expr * expr
```

Examples:
- `1 * value`  → `Identity`
- `S1 - S2`    → `Expr { ast = Sub (Ref "S1", Ref "S2"); refs = [("S1", uuid_a); ("S2", uuid_b)] }`
- `S1 * 1 - S2 * 1` → same as above (multiplier elided; parser folds `Mul (_, Num 1.0)`)

`refs` maps human-readable aliases to sensor uuids. The uuid is the stable identity (the non-prefix part of `S#<uuid>`), so sensors can be renamed without rewriting every formula that references them. Evaluation looks up `S#<uuid>` node rows, fetches their latest readings, substitutes, and reduces.

### 6.4 How this stays additive

- The existing `edges` / `metadata` model is untouched; `sensors` is a new optional key on the schema record.
- Sensors are hierarchy nodes (`pk = S#<uuid>`, `sk = NODE#`). The `gsi1sk = SENSOR#…` prefix separates them from `CHILD#…` entries in the GSI, so `list_children` and `list_sensors` share the same index without conflict.
- Existing node rows don't change shape. `daq_address` is a plain attribute on sensor assignment rows; there is no `partner_id` concept in the hierarchy — that was a flink-table partitioning artifact.
- `Schema_check.find_for` already gives any node its owning schema — `Hierarchy.attach_sensor` reuses it unchanged.
- Formula evaluation is pure and lives in a new `Formula` module; it has no side effects beyond the `Effects.get_sensor_value` it will introduce.

---

## 7. File map

| File                          | Role                                        |
|-------------------------------|---------------------------------------------|
| `lib/domain/level.ml`         | hn0..hn9 enum + depth                       |
| `lib/domain/node_id.ml`       | `<LEVEL>#<uuid>` parser                     |
| `lib/domain/node.ml`          | node record                                 |
| `lib/domain/schema.ml`        | schema type + `validate` + `edges_between`  |
| `lib/domain/metadata.ml`      | field spec + validation                     |
| `lib/logic/hierarchy.ml`      | `add_node`, `list_children`, `delete_node`  |
| `lib/logic/schema_check.ml`   | `find_for` — walk up to the hn2 schema      |
| `lib/repo/codec.ml`           | node ↔ DynamoDB attribute map               |
| `lib/repo/dynamo.ml`          | Eio-based effect handler over smaws         |
| `lib/repo/memory.ml`          | in-memory handler for unit tests            |
| `itest/test_dynamo.ml`        | real-table integration test (this doc's example) |
