# API Reference

Base URL: `https://vp9p5wrn6f.execute-api.eu-central-1.amazonaws.com`

Two endpoints:

- `POST /command` — mutations. JSON body with an `action` field.
- `GET /query/<action>` — reads. Parameters go in the query string.

All responses are JSON. Errors use:

```json
{ "error": { "code": "bad_request", "message": "..." } }
```

Status code is derived from the error (`400`, `404`, etc.).

## URL encoding

Node, sensor, and user ids all contain `#` (e.g. `HN2#2ab951b2-...`,
`S#ff0863e0-...`, `U#alice@example.com`). `curl` strips everything after `#`
before sending — encode it as `%23`, or use `--data-urlencode`:

```sh
curl -G "$BASE/query/get_node" --data-urlencode "id=HN2#2ab951b2-e31c-4049-9dbb-ad82b196b901"
curl -G "$BASE/query/get_user" --data-urlencode "id=U#alice@example.com"
```

## Typical workflow

A new tenant is bootstrapped by chaining a handful of calls. Every
mutation response includes the new id, which is the input to the next
call:

```sh
# 1. create the user who will operate the tree
curl -X POST "$BASE/command" -d '{
  "action": "create_user", "email": "alice@acme.test",
  "name": "Alice", "cognito_group": "admin"
}'
# → { "id": "U#alice@acme.test", … }

# 2. create the partner (hn1) under the implicit root hn0
curl -X POST "$BASE/command" -d '{
  "action": "add_node", "parent_id": "<hn0-root>",
  "name": "Acme Partner"
}'
# → { "id": "HN1#0c5a…", … }

# 3. create the company (hn2) — schema lives here
curl -X POST "$BASE/command" -d '{
  "action": "add_node", "parent_id": "HN1#0c5a…",
  "name": "Acme Co",
  "schema": { "version": 1, "edges": { … }, "sensors": ["hn6"] }
}'
# → { "id": "HN2#2ab9…", … }

# 4. block Alice from a sub-tree (descendants inherit the block)
curl -X POST "$BASE/command" -d '{
  "action": "block_user",
  "user_id": "U#alice@acme.test", "node_id": "HN2#2ab9…"
}'

# 5. ask for the effective capability at a descendant node
curl -G "$BASE/query/effective_permission" \
  --data-urlencode "user=U#alice@acme.test" \
  --data-urlencode "node=HN3#aa…"
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
  "parent_id": "HN1#0c5a...-...",
  "name": "Building A",
  "label": "building",
  "metadata": { "floor_count": 5 }
}
```

Fields:

| field       | required        | notes                                                |
|-------------|-----------------|------------------------------------------------------|
| parent_id   | yes             | `HN<n>#<uuid>`                                       |
| name        | yes             |                                                      |
| label       | no              | if given, resolves the child level from the schema   |
| level       | no              | `hn1` .. `hn9`; override the inferred level          |
| metadata    | no, default `{}`| validated against the schema for the child level     |
| schema      | only for `hn2`  | see schema shape below                               |

**Level resolution.** `level` is optional; if omitted it is inferred:

- No `label` → defaults to parent+1. Error if that level has zero or multiple
  schema candidates without a disambiguating label.
- `label` given → the unique target level in the schema whose edge list
  contains that label. Error if 0 or >1 target levels match.
- Levels `hn0 → hn1` (partner) and `hn1 → hn2` (company) are fixed; no schema
  lookup is involved.

Response — the newly created node:

```json
{
  "id": "HN3#aa...-...",
  "name": "Building A",
  "parent": "HN1#0c5a...-...",
  "created": "2026-04-18T10:22:14Z",
  "metadata": { "floor_count": 5 }
}
```

`hn2` nodes additionally include `"schema": { ... }`.

#### Schema shape (hn2 only)

```json
{
  "version": 1,
  "edges": {
    "hn2": { "hn3": { "building": { "min": 1, "max": 50 } } },
    "hn3": { "hn4": { "floor":    {}                      } }
  },
  "metadata": {
    "hn3": {
      "floor_count": { "required": true, "type": "integer", "min": 0 }
    }
  },
  "sensors": [ "hn6", "hn7" ]
}
```

### delete_node

```json
{ "action": "delete_node", "id": "HN3#aa...-..." }
```

Response:

```json
{ "deleted": "HN3#aa...-..." }
```

### attach_sensor

Create a new sensor under a parent node. The parent's level must be listed
in the owning `hn2`'s `schema.sensors`.

```json
{
  "action": "attach_sensor",
  "parent_id": "HN6#bb...-...",
  "daq_id": "meter-0042",
  "purpose": "electricity",
  "meter_type": "counter",
  "unit": "kWh"
}
```

| field       | required | notes                     |
|-------------|----------|---------------------------|
| parent_id   | yes      | `HN<n>#<uuid>`            |
| daq_id      | yes      | data-acquisition id       |
| purpose     | yes      |                           |
| meter_type  | yes      | `counter` or `gauge`      |
| unit        | no       |                           |

Response — the created sensor:

```json
{
  "id": "S#ff08...-...",
  "created": "2026-04-18T10:22:14Z",
  "parent": "HN6#bb...-...",
  "daq_id": "meter-0042",
  "hierarchy_path": "Partner/Company/.../Building A/...",
  "purpose": "electricity",
  "meter_type": "counter",
  "unit": "kWh"
}
```

### replace_sensor_device

Swap the physical device behind an existing sensor. Previous row is demoted
to history; a new active row with a fresh `created` timestamp is written in
a single `TransactWriteItems`.

```json
{
  "action": "replace_sensor_device",
  "sensor_id": "S#ff08...-...",
  "daq_id": "meter-0099"
}
```

Response — the new active sensor row (same `id`, new `created`, new `daq_id`).

---

## Users and permissions

Enforcement lives upstream (frontend + API Gateway Cognito authorizer). This
Lambda **stores and reports** — it never rejects a call based on the caller's
group. See `docs/architecture.md` §8.

### create_user

Create a user. Fails with `conflict` if a user with the same email already
exists.

```json
{
  "action": "create_user",
  "email": "alice@example.com",
  "name": "Alice",
  "cognito_group": "writer",
  "language": "english",
  "currency": "EUR"
}
```

| field         | required | notes                                                  |
|---------------|----------|--------------------------------------------------------|
| email         | yes      | becomes `U#<email>`                                    |
| name          | yes      |                                                        |
| cognito_group | yes      | `reader` \| `writer` \| `admin`                        |
| language      | no       | `danish` \| `swedish` \| `norwegian` \| `english` \| `german` (default `danish`) |
| currency      | no       | `DKK` \| `SEK` \| `NOK` \| `USD` \| `EUR` (default `DKK`) |

Response — the created user:

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
  "node_id": "HN4#bb...-..."
}
```

| field   | required | notes                        |
|---------|----------|------------------------------|
| user_id | yes      | `U#<email>`                  |
| node_id | yes      | `HN<n>#<uuid>`               |

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
  "node_id": "HN4#bb...-..."
}
```

```json
{ "ok": true }
```

---

## Queries — `GET /query/<action>`

### get_node

```
GET /query/get_node?id=HN2%232ab951b2-e31c-4049-9dbb-ad82b196b901
```

Response:

```json
{
  "id": "HN2#2ab951b2-e31c-4049-9dbb-ad82b196b901",
  "name": "Acme Co",
  "parent": "HN1#0c5a...-...",
  "created": "2026-04-18T09:00:00Z",
  "metadata": {},
  "schema": { "version": 1, "edges": { ... }, "metadata": { ... }, "sensors": [ ... ] }
}
```

`parent` is `null` on the root.

### list_children

Default mode returns edge rows only — `{id, name}` per child, one DynamoDB
`Query` call:

```
GET /query/list_children?parent=HN2%232ab951b2-...&label=building
```

```json
{
  "children": [
    { "id": "HN3#aa...-...", "name": "Building A" },
    { "id": "HN3#bb...-...", "name": "Building B" }
  ]
}
```

Add `full=true` (or `full=1`) to dereference each edge into the full node —
one extra `GetItem` per child:

```
GET /query/list_children?parent=HN2%232ab951b2-...&label=building&full=true
```

```json
{
  "children": [
    {
      "id": "HN3#aa...-...",
      "name": "Building A",
      "parent": "HN2#2ab951b2-...",
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
GET /query/list_sensors?parent=HN6%23bb...-...
```

```json
{
  "sensors": [
    {
      "id": "S#ff08...-...",
      "created": "2026-04-18T10:22:14Z",
      "parent": "HN6#bb...-...",
      "daq_id": "meter-0042",
      "hierarchy_path": "Partner/.../Building A/...",
      "purpose": "electricity",
      "meter_type": "counter",
      "unit": "kWh"
    }
  ]
}
```

### get_sensor

```
GET /query/get_sensor?id=S%23ff08...-...
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
{ "nodes": [ "HN4#bb...-...", "HN3#cc...-..." ] }
```

### list_blocked_users

Inverse of `list_blocked_nodes`: users directly blocked on the given node.

```
GET /query/list_blocked_users?node=HN4%23bb...-...
```

```json
{ "users": [ "U#alice@example.com", "U#bob@example.com" ] }
```

### effective_permission

Resolves `(user, node)` through the block chain. Returns the user's
`cognito_group` when allowed, `null` when blocked by the node itself or any
ancestor. Capability group is a ceiling — enforcement is upstream.

```
GET /query/effective_permission?user=U%23alice@example.com&node=HN4%23bb...-...
```

Allowed:

```json
{ "capability": "writer" }
```

Blocked:

```json
{ "capability": null, "reason": "blocked" }
```

| param | required | notes              |
|-------|----------|--------------------|
| user  | yes      | `U#<email>`        |
| node  | yes      | `HN<n>#<uuid>`     |
