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

Node and sensor ids contain `#` (e.g. `HN2#2ab951b2-...`, `S#ff0863e0-...`).
`curl` strips everything after `#` before sending — encode it as `%23`,
or use `--data-urlencode`:

```sh
curl -G "$BASE/query/get_node" --data-urlencode "id=HN2#2ab951b2-e31c-4049-9dbb-ad82b196b901"
```

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
