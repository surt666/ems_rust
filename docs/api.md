# API Reference

Base URL: `https://xbvb3nzp1h.execute-api.eu-central-1.amazonaws.com`

Two endpoints:

- `POST /command` — mutations. JSON body with an `action` field. (Also accepts
  `application/x-www-form-urlencoded`, which the HTMX frontend uses.)
- `GET /query/<action>` — reads. Parameters go in the query string.

`/command` and `/query/<action>` are also reachable under a `/hierarchy/`
prefix; `GET /hierarchy/query/<action>` additionally exposes HTML-fragment
actions for the HTMX UI. The JSON actions below are identical on both paths.

JSON responses; errors use:

```json
{ "error": { "code": "Bad_request", "message": "..." } }
```

Status code is derived from the error (`400`, `404`, `409`, `500`). Codes:
`Bad_request`, `Validation` (with a `details` array), `Schema_missing`,
`Not_found`, `Conflict`, `Internal`.

## URL encoding

Node, sensor, and user ids all contain `#` (e.g. `HN2#102`, `S#42`,
`U#alice@example.com`). Node and sensor ids are `<prefix>#<integer>` — the
integer is allocated by a per-level counter, not a uuid. `curl` strips
everything after `#` before sending — encode it as `%23`, or use
`--data-urlencode`:

```sh
curl -G "$BASE/query/get_node" --data-urlencode "id=HN2#102"
curl -G "$BASE/query/get_user" --data-urlencode "id=U#alice@example.com"
```

## Typical workflow

A new tenant is bootstrapped by chaining a handful of calls. Every
mutation response includes the new id, which is the input to the next
call:

```sh
# 1. create the user who will operate the tree (profile → Cognito group)
curl -X POST "$BASE/command" -d '{
  "action": "create_user", "email": "alice@acme.test",
  "name": "Alice", "profile": "SysAdm"
}'
# → { "id": "U#alice@acme.test", … }

# 2. create the partner (hn1) under the implicit root HN0#root
curl -X POST "$BASE/command" -d '{
  "action": "add_node", "parent_id": "HN0#root",
  "name": "Acme Partner"
}'
# → { "id": "HN1#11", … }

# 3. create the company (hn2) — the type-graph schema lives here
curl -X POST "$BASE/command" -d '{
  "action": "add_node", "parent_id": "HN1#11",
  "name": "Acme Co",
  "schema": { "version": 2, "edges": { "company": { "building": {} } },
              "metadata": {}, "sensors": ["building"] }
}'
# → { "id": "HN2#102", … }

# 4. block Alice from a sub-tree (descendants inherit the block)
curl -X POST "$BASE/command" -d '{
  "action": "block_user",
  "user_id": "U#alice@acme.test", "node_id": "HN2#102"
}'

# 5. ask for the effective capability at a descendant node
curl -G "$BASE/query/effective_permission" \
  --data-urlencode "user=U#alice@acme.test" \
  --data-urlencode "node=HN3#1003"
# → { "capability": null, "reason": "blocked" }
```

The Lambda only reports — it does not reject the caller. Enforcement
lives in the upstream authorizer; `effective_permission` is what it
consults.

---

## Commands — `POST /command`

### add_node

Create a non-root node under a parent. Schema is required when creating an
`hn2` node and forbidden everywhere else.

Request:

```json
{
  "action": "add_node",
  "parent_id": "HN1#11",
  "name": "Building A",
  "label": "building",
  "metadata": { "floor_count": 5 }
}
```

Fields:

| field       | required        | notes                                                |
|-------------|-----------------|------------------------------------------------------|
| parent_id   | yes             | `HN<n>#<int>`                                        |
| name        | yes             |                                                      |
| label       | no              | the child **type** (must be allowed under the parent type) |
| level       | no              | `hn1` .. `hn9`; if given **must equal** parent + 1    |
| metadata    | no, default `{}`| validated against the schema for the child type      |
| schema      | only for `hn2`  | see schema shape below                               |

**Level and type resolution.** The child's **level is always parent + 1**
(derived); an explicit `level` that disagrees is rejected. The child's **type**
is the `label`:

- `hn0 → hn1` is the reserved type `partner`; `hn1 → hn2` is `company`; neither
  takes a schema lookup.
- Below hn2 the type is resolved from the parent type's allowed children:
  `label` must name an allowed child type, or it may be omitted only when the
  parent type has exactly one allowed child type.

Response — the newly created node:

```json
{
  "id": "HN4#10044",
  "name": "Building A",
  "label": "building",
  "parent": "HN3#10003",
  "created": "2026-04-18T10:22:14Z",
  "metadata": { "lat": 55.68, "lng": 12.57 }
}
```

`hn2` nodes additionally include `"schema": { ... }`.

#### Schema shape (hn2 only) — type graph (v2)

`edges` is keyed by **type name** (a DAG rooted at the reserved `"company"`),
not by level. The child type *is* the label; `metadata` and `sensors` are keyed
by type. See `docs/hierarchy-and-sensors.md` §2 for the validation rules
(DAG, reachable from `company`, longest chain ≤ 7).

```json
{
  "version": 2,
  "edges": {
    "company":  { "group": {}, "property": {}, "building": { "max": 50 } },
    "group":    { "building": { "min": 1 } },
    "property": { "building": { "min": 1 } },
    "building": { "area": {} }
  },
  "metadata": {
    "building": {
      "lat": { "required": true, "type": "number", "min": -90, "max": 90 },
      "lng": { "required": true, "type": "number", "min": -180, "max": 180 }
    }
  },
  "sensors": [ "building", "area" ]
}
```

### delete_node

```json
{ "action": "delete_node", "id": "HN4#10044" }
```

Response:

```json
{ "deleted": "HN4#10044" }
```

### update_node

Replace a node's metadata. Validated against the node type's schema (form
strings are coerced to their declared types first); only schema-declared fields
are kept. hn0/hn1 nodes have no editable metadata.

```json
{
  "action": "update_node",
  "id": "HN4#10044",
  "metadata": { "lat": 55.68, "lng": 12.57 }
}
```

| field    | required | notes                                              |
|----------|----------|----------------------------------------------------|
| id       | yes      | `HN<n>#<int>` (hn2 and below)                       |
| metadata | no       | default `{}`; validated against the type's schema  |

Response — the updated node (same shape as `get_node`).

### attach_sensor

Create a new sensor under a parent node. The parent's level must be listed
in the owning `hn2`'s `schema.sensors`.

```json
{
  "action": "attach_sensor",
  "parent_id": "HN6#600",
  "daq_id": "meter-0042",
  "purpose": "electricity",
  "meter_type": "counter",
  "unit": "kWh",
  "resample_minutes": 15
}
```

| field       | required | notes                                              |
|-------------|----------|----------------------------------------------------|
| parent_id   | yes      | `HN<n>#<int>`                                      |
| daq_id      | yes      | data-acquisition id                                |
| purpose     | yes      |                                                    |
| meter_type  | yes      | `counter` or `gauge`                               |
| unit        | no       |                                                    |
| resample_minutes | no    | resample interval in minutes, `> 0`; accepts an int or a numeric string |
| formula          | no    | `{"kind":"identity"\|"zero"\|"expr", "expr": <text>, "refs": <object or JSON string>}`; default `identity`. For `expr`, `expr` is a text formula over `self`, numbers, `+ - * /`, `abs()`, and aliases; `refs` maps each alias to a sensor id (`"S#<n>"`). Every alias used in the expression must be bound; refs with unused aliases are rejected. Cross-sensor references are scoped to the owning HN2 company. |

Response — the created sensor (mirrors `json::sensor_to_json`). `path` is
the pipe-separated ancestry ending in the sensor id; `unit`/`resample_minutes` are
`null` when unset. `formula` is always present: `{"kind":"identity"}` by
default, or for `expr` formulas also includes `"expr"` (the text) and `"refs"`
(the alias-to-sensor-id map):

```json
{
  "id": "S#42",
  "created": "2026-04-18T10:22:14Z",
  "daq_id": "meter-0042",
  "path": "HN0#root|HN1#11|HN2#102|HN6#600|S#42",
  "purpose": "electricity",
  "meter_type": "counter",
  "unit": "kWh",
  "resample_minutes": 15,
  "formula": { "kind": "identity" }
}
```

### replace_sensor_device

Swap the physical device behind an existing sensor. Previous row is demoted
to history; a new active row with a fresh `created` timestamp is written in
a single `TransactWriteItems`.

```json
{
  "action": "replace_sensor_device",
  "sensor_id": "S#42",
  "daq_id": "meter-0099"
}
```

Response — the new active sensor row (same `id`, new `created`, new `daq_id`).

---

## Users and permissions

Enforcement lives upstream (frontend + API Gateway Cognito authorizer). This
Lambda **stores and reports** — it never rejects a call based on the caller's
group. See `docs/architecture.md` §9.

### create_user

Create a user. Fails with `Conflict` if a user with the same email already
exists. The Lambda also provisions the Cognito user (creates it, adds it to the
mapped group, sets a permanent password) and rolls the DDB row back if Cognito
fails. Each `allowed` node gets an access edge whose kind matches the user's
group (`Admin → administrates`, `Writer → writes`, `Reader → reads`); each
`blocked` node gets a `Blocked` edge.

```json
{
  "action": "create_user",
  "email": "alice@example.com",
  "name": "Alice",
  "profile": "Developer",
  "language": "english",
  "currency": "EUR",
  "allowed": ["HN2#102"],
  "blocked": ["HN3#1003"]
}
```

| field     | required | notes                                                  |
|-----------|----------|--------------------------------------------------------|
| email     | yes      | becomes `U#<email>`                                    |
| name      | yes      |                                                        |
| profile   | yes      | `SysAdm` (→ admin) \| `Developer`/`Standard` (→ writer) \| `Technician`/`Reader` (→ reader) |
| language  | no       | `danish` \| `swedish` \| `norwegian` \| `english` \| `german` (default `danish`) |
| currency  | no       | `DKK` \| `SEK` \| `NOK` \| `USD` \| `EUR` (default `DKK`) |
| allowed   | no       | node ids granted an access edge for the user's group   |
| blocked   | no       | node ids the user is blocked from                      |

Response — the created user (note the response carries the resolved
`cognito_group`, not the input `profile`):

```json
{
  "id": "U#alice@example.com",
  "email": "alice@example.com",
  "name": "Alice",
  "cognito_group": "writer",
  "language": "english",
  "currency": "EUR",
  "created": "2026-04-19T10:00:00Z"
}
```

### update_user

Partial update. Omitted fields keep their current value.

```json
{
  "action": "update_user",
  "id": "U#alice@example.com",
  "name": "Alice Smith",
  "cognito_group": "admin"
}
```

| field         | required | notes                              |
|---------------|----------|------------------------------------|
| id            | yes      | `U#<email>`                        |
| name          | no       |                                    |
| cognito_group | no       | `reader` \| `writer` \| `admin`    |
| language      | no       |                                    |
| currency      | no       |                                    |

Response — the updated user record (same shape as `create_user`).

### delete_user

Removes the user row and cascades all that user's `Blocked` edges.

```json
{ "action": "delete_user", "id": "U#alice@example.com" }
```

```json
{ "deleted": "U#alice@example.com" }
```

### block_user

Attach a `Blocked` edge from the user to the node. The block propagates to
every descendant of `node_id`.

```json
{
  "action": "block_user",
  "user_id": "U#alice@example.com",
  "node_id": "HN4#10044"
}
```

| field   | required | notes                        |
|---------|----------|------------------------------|
| user_id | yes      | `U#<email>`                  |
| node_id | yes      | `HN<n>#<int>`                |

Response:

```json
{ "ok": true }
```

### unblock_user

Remove a `Blocked` edge. Idempotent — deleting a non-existent edge also
returns `{"ok": true}`.

```json
{
  "action": "unblock_user",
  "user_id": "U#alice@example.com",
  "node_id": "HN4#10044"
}
```

```json
{ "ok": true }
```

### grant_administrates

Attach an `Administrates` (grant) edge from the user to the node. The grant
covers `node_id` and every descendant; `access::has_admin_access` reports it,
and any access edge (incl. this one) makes the subtree browsable via
`access::has_access`. The seeded admin holds a grant on `HN0#root`.

```json
{
  "action": "grant_administrates",
  "user_id": "U#alice@example.com",
  "node_id": "HN2#102"
}
```

| field   | required | notes                                  |
|---------|----------|----------------------------------------|
| user_id | yes      | `U#<email>`                            |
| node_id | yes      | `HN<n>#<int>` (or `HN0#root`)          |

```json
{ "ok": true }
```

---

## Queries — `GET /query/<action>`

### get_node

```
GET /query/get_node?id=HN2%23102
```

Response:

```json
{
  "id": "HN2#102",
  "name": "Acme Co",
  "parent": "HN1#11",
  "created": "2026-04-18T09:00:00Z",
  "metadata": {},
  "schema": { "version": 2, "edges": { ... }, "metadata": { ... }, "sensors": [ ... ] }
}
```

`parent` is `null` on the root.

### list_children

Default mode returns edge rows only — `{id, name}` per child, one DynamoDB
`Query` call:

```
GET /query/list_children?parent=HN2%23102&label=building
```

```json
{
  "children": [
    { "id": "HN3#1003", "name": "Building A" },
    { "id": "HN3#1004", "name": "Building B" }
  ]
}
```

Add `full=true` (or `full=1`) to dereference each edge into the full node —
one extra `GetItem` per child:

```
GET /query/list_children?parent=HN2%23102&label=building&full=true
```

```json
{
  "children": [
    {
      "id": "HN3#1003",
      "name": "Building A",
      "parent": "HN2#102",
      "created": "2026-04-18T09:01:00Z",
      "metadata": { "floor_count": 5 }
    }
  ]
}
```

| param  | required | notes                                         |
|--------|----------|-----------------------------------------------|
| parent | yes      | parent node id                                |
| label  | no       | restricts to one edge label                   |
| full   | no       | `true`/`1` to fetch full node records         |

### list_sensors

```
GET /query/list_sensors?parent=HN6%23600
```

```json
{
  "sensors": [
    {
      "id": "S#42",
      "created": "2026-04-18T10:22:14Z",
      "daq_id": "meter-0042",
      "path": "HN0#root|HN1#11|HN2#102|HN6#600|S#42",
      "purpose": "electricity",
      "meter_type": "counter",
      "unit": "kWh",
      "resample_minutes": 15
    }
  ]
}
```

### get_sensor

```
GET /query/get_sensor?id=S%2342
```

Response — same shape as a single entry in `list_sensors`.

### get_user

```
GET /query/get_user?id=U%23alice@example.com
```

Response — same shape as `create_user`.

### list_users

```
GET /query/list_users
```

```json
{
  "users": [
    {
      "id": "U#alice@example.com",
      "email": "alice@example.com",
      "name": "Alice",
      "cognito_group": "writer",
      "language": "english",
      "currency": "EUR",
      "created": "2026-04-19T10:00:00Z"
    }
  ]
}
```

### list_blocked_nodes

Nodes on which the user carries a direct `Blocked` edge. Does not expand
ancestors — a user blocked on an ancestor is still absent from this list for
the descendants.

```
GET /query/list_blocked_nodes?user=U%23alice@example.com
```

```json
{ "nodes": [ "HN4#10044", "HN3#1003" ] }
```

### list_blocked_users

Inverse of `list_blocked_nodes`: users directly blocked on the given node.

```
GET /query/list_blocked_users?node=HN4%2310044
```

```json
{ "users": [ "U#alice@example.com", "U#bob@example.com" ] }
```

### effective_permission

Resolves `(user, node)` against the access edges and the block chain. Returns
the capability of the **nearest access edge** up the chain
(`administrates → admin`, `writes → writer`, `reads → reader`), or `null` when
the node or any ancestor is blocked, or when the user has no access edge on the
chain at all. Enforcement is upstream — this is what the authorizer consults.

```
GET /query/effective_permission?user=U%23alice@example.com&node=HN4%2310044
```

Allowed (nearest access edge is a `writes` grant):

```json
{ "capability": "writer" }
```

Blocked (or no access edge on the chain):

```json
{ "capability": null, "reason": "blocked" }
```

| param | required | notes              |
|-------|----------|--------------------|
| user  | yes      | `U#<email>`        |
| node  | yes      | `HN<n>#<int>`      |
