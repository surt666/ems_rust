# Editable Node Metadata — Design Spec

**Date:** 2026-06-12
**Status:** Approved (design); implementation plan pending
**Surface:** Hierarchy Rust Lambda (`crates/model` + `crates/services/hierarchy`) and the
Astro frontend node view (account `339712745226`).

## Context & goal

The per-node view (`/node`, the `render_node` HTML fragment) shows a node's metadata as
read-only inputs. There is no way to edit it. We want an **Edit** button in the metadata
section that makes the metadata inputs writable and turns into a **Save** button that persists
the change to the node's DynamoDB item. The control is shown **only to Admin and Writer**
users.

There is no `update_node` command today (the `Command` ADT has add/delete node, sensors,
users, access — but no node update). The repository already has a full-item
`put_node` (DynamoDB and the in-memory `Store`), and `query.rs` already renders schema-typed
metadata inputs for the add-child form. This feature adds the missing command + logic + the
editable UI, reusing those primitives.

## Editing model

Metadata is **schema-governed**: the owning HN2 company's type-graph schema declares, per
node type, the metadata fields and their types (`schema.metadata_for(type)`). Editing is
**schema-typed**, not free-form:

- The metadata section renders one input per schema-declared field for the node's type,
  **pre-filled from `node.metadata`**, with the HTML input `type` matching the schema type:
  - `string`  → `<input type="text">`
  - `number`  → `<input type="number" step="any">`
  - `integer` → `<input type="number" step="1">`
  - `timestamp` → `<input type="date">`
  - `enum`    → `<select>` of `one_of`
  - `boolean` → `<select>` of `true`/`false`
- Only schema-declared fields are editable. A stored metadata key not in the schema is not
  shown and is not preserved on save (in practice metadata is exactly the schema fields).
- Fields are pre-selected/pre-filled to the node's current values; required fields keep the
  `*` marker.

## Backend

### Command (`crates/services/hierarchy/src/command.rs`)

```rust
/// `update_node` — replace a node's metadata.
UpdateNode {
    id: String,
    #[serde(default)]
    metadata: Option<Value>,
},
```

Serde-tagged on `action: "update_node"`. The existing form→JSON normaliser already nests
flat `data.metadata.<field>` form fields into a `metadata` object, so no parser change is
needed. A `parse_update_node` unit test is added.

### Handler (`dispatch.rs`)

`handle_update_node(id, metadata, get_node, put_node)`:
- Parse `id` (`NodeId::parse`) → `Bad_request` on failure.
- Call `hierarchy::update_node_metadata(...)`; on `Ok(node)` return `ok(node_to_json(&node))`,
  else `repo_error_response(e)`.

Wired in `dispatch::run`'s `Command::UpdateNode` arm with the existing closures:
`ddb_node::get_node` (as `Fn`, for the schema walk) and `ddb_node::put_node` (as `FnOnce`).

### Logic (`crates/model/src/logic/hierarchy.rs`)

```rust
pub async fn update_node_metadata<FGN, FGNFut, FPN, FPNFut>(
    id: NodeId,
    metadata: serde_json::Value,
    get_node_fn: FGN,   // Fn — also drives schema_check::find_for
    put_node_fn: FPN,   // FnOnce
) -> Result<Node, RepositoryError>
```

Steps:
1. `get_node_fn(id)` → `NotFound(id)` if absent.
2. If the node level is hn0/hn1 (no schema-governed metadata) → `BadRequest`
   ("node type has no editable metadata").
3. `schema_check::find_for(id, &get_node_fn)` → `(_, schema)`; `specs =
   schema.metadata_for(&node.label)`.
4. `schema::coerce_metadata(&specs, &mut metadata)` (below).
5. `schema::validate(&specs, &metadata)` → `Validation(errs)` on failure.
6. Rebuild the persisted object from **declared fields only** (a value present in `metadata`
   for each declared field name) so a non-UI caller cannot inject keys the schema doesn't
   declare. Set `node.metadata` to this object; `put_node_fn(node.clone())`; return the node.

### Metadata coercion (`crates/model/src/domain/schema.rs`)

New pure helper, because form-encoded values arrive as strings but `validate` expects JSON of
the declared type:

```rust
/// Coerce string-encoded form values to their declared schema types, in place.
pub fn coerce_metadata(specs: &[(String, FieldSpec)], v: &mut serde_json::Value);
```

Per declared field present in the object:
- `number` / `integer`: a JSON string that parses as a number → JSON number; already-number
  left as-is.
- `boolean`: `"true"`/`"false"` string → JSON bool.
- `timestamp`: a date-only string (`YYYY-MM-DD`, from `<input type="date">`) is normalized to a
  valid RFC3339 instant at midnight UTC (`YYYY-MM-DDT00:00:00Z`); an already-valid RFC3339
  string is left as-is. (Without this, a `date` value fails the schema's RFC3339 check.)
- `string` / `enum`: left as the string.
- An **empty string** for any field → the key is removed (so a blank optional field validates
  as absent rather than failing a type check).

`coerce_metadata` only adjusts declared fields' values; it does not add or remove keys.
Restricting the persisted object to declared fields is done by `update_node_metadata` (step 6),
not by coercion.

**`coerce_metadata` is also called by `add_under_schema`** (the add-child path) immediately
before `validate`, fixing the same latent gap there so creating and editing the same field
(e.g. `lat`) behave identically. It is a no-op for the existing add_node tests, which pass
real JSON numbers.

## Frontend

### `render_node` (`crates/services/hierarchy/src/html/node.rs`)

`render_node` gains a `metadata_fields: &[(String, FieldSpec)]` parameter, computed in
`handle_node` from `schema.metadata_for(&node.label)`.

- **When `can_write` (Admin or Writer) and `!metadata_fields.is_empty()`**: render the metadata
  section as `<form id="metadata-form">` with:
  - a hidden `action=update_node` and `data.id=<node id>`;
  - one typed input per field (types per the editing model above), `name="data.metadata.<field>"`,
    pre-filled from `node.metadata`, starting **read-only/disabled**;
  - an **Edit** button, a **Save** button (hidden until editing), and an inline error `<div>`.
  - **Edit** (hyperscript `_="…"`, matching the codebase style): removes `readonly`/`disabled`,
    hides Edit, shows Save.
  - **Save** (`type="submit"`): `data-hx-post="/hierarchy/command"`, `data-hx-swap="none"`,
    `hx-on--after-request`: on success `htmx.trigger('#node-data-panel','load')` (re-renders the
    card read-only with persisted values); on failure show `event.detail.xhr.responseText` in
    the error div and keep editing. Mirrors the existing add-sensor flow.
- **Otherwise** (Reader / `None`, or no declared fields): the current read-only display is
  unchanged — typed-but-readonly inputs when there are fields, or "No metadata available".

The shared typed-input builder is reused/extended from the existing `build_metadata_inputs`
in `query.rs` (parameterised to accept pre-fill values and a read-only flag); it lives in one
place so add-child and node-edit render identical inputs.

### `handle_node` (`query.rs`)

Already resolves the effective `schema`. Compute `metadata_fields =
schema.metadata_for(&n.label)` (empty when no schema) and pass it to `render_node`.

### i18n

Add `node.edit` and `node.save` to `translations/da.json` and `translations/en.json`
("Rediger"/"Edit", "Gem"/"Save").

## Permissions

Gating is **UI-only**, via the `capability` already computed by `effective_permission` and
passed to `render_node`: the Edit/Save form is rendered only for `Some(Admin) | Some(Writer)`.
This matches the codebase's store-and-report model — the Lambda has no caller-auth context, so
`update_node` itself does not re-check the caller's group (enforcement is the upstream API
Gateway Cognito authorizer, consistent with `delete_node`, `block_user`, etc.).

## Error handling

- Bad node id → `400 Bad_request`.
- Node not found → `404 Not_found`.
- hn0/hn1 node → `400 Bad_request` ("no editable metadata").
- Metadata fails schema validation (after coercion) → `400 Validation` with the `details`
  array of `{path, message}`; the UI shows the first message in the inline error div.
- DynamoDB failure → `500 Internal`.

## Testing

- **`schema::coerce_metadata`** (unit): string→number/integer, `"true"`→bool, date→RFC3339,
  empty-string→key removed, already-typed values untouched, non-declared keys ignored.
- **`hierarchy::update_node_metadata`** (logic, in-memory `Store`): happy path persists +
  returns node; validation rejection (e.g. `lat` out of range / wrong type); `NotFound` for a
  missing node; hn1 node rejected; coercion makes a string `"55.0"` validate against a `number`
  field.
- **`command.rs`**: `parse_update_node` (JSON and form-encoded with `data.metadata.*`).
- **`dispatch::handle_update_node`** (in-memory): 200 + persisted metadata on success; 400 on
  a validation failure.
- **`render_node`** (unit): Writer/Admin with fields → `metadata-form` + Edit + Save present;
  Reader/None → no form/Edit; `can_write` but no declared fields → no form/Edit.
- **Golden**: the existing hn2-company golden has no `company` metadata fields, so its node card
  is unchanged; re-confirm the golden still matches (and add a golden or unit fixture for a
  node *with* fields if useful).
- `cargo test` + `cargo clippy` warning-free; `npm run build` for the frontend.

## Out of scope

- Editing the node `name` (the name input's writable-for-Writer state today has no persistence;
  not part of this feature).
- Editing nested-object / array metadata (schema metadata is flat scalar fields).
- Backend enforcement of the caller's group (remains upstream).
- Per-field server-side audit/history of metadata changes.
