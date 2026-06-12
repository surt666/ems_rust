# Graphical Schema Designer (company creation) — Design Spec

**Date:** 2026-06-11
**Status:** Shipped, but integrated differently than written here. The designer
landed in the **server-rendered add-child dialog** (`render_add_child_form`)
shown when "company" is the chosen child type — not the masterdata create
dialog. It serialises the type graph into a hidden `schema_json` form field
that the backend folds into the `add_node` command (not a `create_node` JSON
fetch). The pure `schema-serialize.js` module and `SchemaDesigner.astro` editor
are as designed.
**Account / surface:** Frontend (Astro + Alpine, account `339712745226` S3+CloudFront)
**Depends on:** the v2 type-graph schema (`docs/superpowers/specs/2026-06-10-type-graph-schema-design.md`), now deployed.

## Context & goal

Creating a company (HN2) requires a v2 type-graph schema: node **types**, a DAG of allowed
containment between them rooted at the reserved type `company`, per-type **metadata** field specs,
and which types **allow sensors**. The existing "Opret under element" dialog in
`frontend/src/pages/masterdata.astro` posts company fields but **no schema**, so it cannot create a
company against the v2 backend at all. This is the immediate blocker.

This spec defines a **graphical schema designer** that opens in its own dialog from the company
creation flow, lets an admin build/customize the schema from a sensible preset, validates it live,
and posts it with `create_node`.

**Scope (v1): create-only.** Editing an existing company's schema (which must reckon with nodes
already beneath it) is explicitly deferred.

## User flow

1. In the create dialog, the user picks type **company**. A **"Design hierarchy schema"** button
   appears; the company cannot be saved until a schema is defined. (The standard preset is loaded by
   default, so a schema exists immediately — the button reads "✓ schema defined (edit)".)
2. Clicking it opens `<dialog id="schema-designer-dialog">` — the two-panel designer, preloaded with
   the standard preset.
3. The user customizes types/edges/metadata/sensors with live validation. **Done** is disabled while
   the schema is invalid.
4. **Done** serializes the v2 schema into the create form's Alpine state and closes the designer.
   Closing without Done keeps the last good schema.
5. Saving the company submits node `data` + `schema` together (see Submit path).

## Architecture

A self-contained **Alpine component**, `frontend/src/components/SchemaDesigner.astro`, holding the
two-panel UI and its state. No React; matches the existing `masterdata.astro` / `MasterData.astro`
Alpine + HTMX + hyperscript patterns. Boundaries:

- **SchemaDesigner component** — owns schema state (`types`, `edges`, `metadata`, `sensors`),
  renders the editor, runs client-side validation, and serializes to v2 JSON. Exposes the current
  serialized schema + a validity flag to its host.
- **Create dialog (`masterdata.astro`)** — owns the node `data` fields, hosts the designer, and
  performs the company submit.
- **Backend `add_node`** — unchanged; its hn1→hn2 path already requires and validates the schema.

### Two-panel UI (approach C)

```
┌─ Types ──┐  ┌─ Editing: <type> ─────────────┐  ┌─ Live validation ─┐
│ company  │  │ Allowed children (checkboxes) │  │ ✓ acyclic         │
│ group    │  │   group ☐  property ☐         │  │ ✓ reachable       │
│ property │  │   building ☑  area ☐(cycle)   │  │ ✓ depth 3 ≤ 7     │
│▸building │  │ Metadata fields (table)       │  ├─ DAG preview ─────┤
│ area     │  │   lat  number req☑ [min][max] │  │ company → …       │
│ ＋add     │  │   ＋ add field                 │  ├─ Schema JSON ─────┤
└──────────┘  │ Sensors here ☑                │  │ {…posted on save} │
              └───────────────────────────────┘  └───────────────────┘
```

- **Left** — every type; add (text + ＋) / remove (✕). `company` is fixed (no remove).
- **Centre** — for the selected type: **Allowed children** as checkboxes of the other (non-`company`)
  types, with any edge that would close a cycle **disabled and annotated "(cycle)"**; **Metadata
  fields** table; **Sensors allowed here** toggle.
- **Right** — live validation panel, a text DAG preview, and the live serialized schema JSON.

(A working reference prototype was built during brainstorming and validated by the user.)

## Data model & serialization

In-memory (Alpine):

```
types:    string[]                      // includes "company" (root)
edges:    { [parentType: string]: string[] }   // parent → allowed child types
metadata: { [type: string]: Field[] }
sensors:  string[]                      // types that allow sensors
```

`Field = { name: string, type: FieldType, required: boolean, ...constraints }` where `FieldType ∈
{string, number, integer, boolean, timestamp, enum}` and constraints depend on the type:

| type | constraint inputs | serialized keys |
|---|---|---|
| string | min length, max length | `min_len`, `max_len` |
| number | min, max (floats) | `min`, `max` |
| integer | min, max (ints) | `min`, `max` |
| boolean | — | — |
| timestamp | — | — |
| enum | list of allowed values | `one_of: [..]` |

Empty/blank constraint inputs are omitted (not serialized). **v1 ships no edge-cardinality UI**: the
serializer emits bare `{}` for every edge (matching the preset), and `min`/`max` per containment is a
later "advanced" addition — the serializer leaves room for it but the designer renders no control.

**Serialized v2 schema** (exact shape `crates/services/hierarchy/src/json.rs :: schema_of_json`
parses):

```json
{ "version": 2,
  "edges":    { "company": { "group": {}, "property": {}, "building": {} },
                "group": { "building": {} }, "property": { "building": {} },
                "building": { "area": {} } },
  "metadata": { "building": { "lat": { "type": "number", "required": true, "min": -90, "max": 90 },
                              "lng": { "type": "number", "required": true, "min": -180, "max": 180 } } },
  "sensors":  ["building", "area"] }
```

`edges` includes only parents that have ≥1 child; `metadata` only types that have ≥1 field.

### Reserved names

- `company` — the fixed root; always present, never removable, never a child option.
- `partner` — rejected as a type name (reserved for HN1).

## Preset

Hardcoded in the component:

```
company → group, property, building
group    → building
property → building
building → area
metadata: building { lat:number req ±90, lng:number req ±180 }
sensors:  building, area
```

A **Clear** action empties it to just `company` (blank start).

## Validation (two layers)

**Client, live** (mirrors `Schema::validate` for instant feedback; **Done disabled while invalid**,
reasons listed):

- Cycle-forming edges are disabled in the UI (an edge `A→B` is blocked when `B` can already reach
  `A`), so the graph stays acyclic by construction.
- Every type reachable from `company` (unreachable types flagged).
- Longest path from `company` ≤ 7 edges (shown as the deepest hn-level it implies; > 7 rejected).
- No duplicate type names; no duplicate field names within a type.
- `min ≤ max` (and `min_len ≤ max_len`) on any field constraint.
- Soft note (non-blocking) when `company` has no child edges yet (server permits a leaf-only company).

**Server, authoritative** on submit: `add_node` re-runs `Schema::validate`. Any error text is
surfaced verbatim in the dialog's error area; the dialog stays open with state intact on failure.
The client mirror is UX only and never the gate of record.

## Submit path

Non-company node types keep their current HTMX form post unchanged. **Company creation moves to a
JSON `fetch`** (the pattern `deleteNode()` already uses in `masterdata.astro`), because a nested
schema object cannot ride in flat form fields and `schema_of_json` needs a real JSON object:

```js
await fetch('/hierarchy/command', {
  method: 'POST', headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    action: 'create_node',
    data: { name, cvr, email, /* …company fields… */ },
    schema: serializedSchema,            // the v2 object
    parent_id: selectedPartnerPath,
  }),
});
```

No new backend endpoint: `Command::AddNode { schema: Option<Value> }` and the hn1→hn2 path already
accept and validate it. On success: close both dialogs, clear the designer state, reload the tree.

## Code organization

- `frontend/src/components/SchemaDesigner.astro` — markup + Alpine `x-data` logic (the editor).
- `frontend/src/lib/schema-serialize.js` — **pure** module: `serialize(state) -> v2Json` and
  `validate(state) -> { ok, errors[] }` (cycle/reachability/depth/dup/min-max). DOM-free, imported by
  the component and unit-tested directly.
- `frontend/src/pages/masterdata.astro` — host the designer dialog; add the "Design hierarchy schema"
  button + the company JSON-submit branch.

## Testing

- **Unit (`node --test`, zero new deps)** on `schema-serialize.js`: preset serializes to the expected
  v2 JSON; each constraint type serialized correctly (string len, number/integer min/max, enum
  one_of); and every validation rule — cycle detection, reachability, depth boundary (7 ok / 8
  reject), duplicate type, duplicate field, min > max.
- **E2E smoke (Playwright, already a devDependency):** open create dialog → pick company → open
  designer → assert preset renders, a cycle-forming child checkbox is disabled, adding a field +
  constraint updates the serialized JSON, **Done** captures it into form state. Asserts the
  serialized schema the component would post; does **not** submit to the backend (no test data
  written).
- **Backend safety net:** existing Rust `Schema::validate` tests remain authoritative for the rules.

## Deployment

Frontend-only change. Per `CLAUDE.md`: `cd frontend && npm run build` (env baked in), then
`cd infra/frontend && unset GOROOT && cdk deploy OcamlFrontendStack`. No `PUBLIC_*` change needed —
the designer calls the same-origin `/hierarchy/command` CloudFront proxies. Backend unchanged.

## Out of scope (deferred)

- **Editing an existing company's schema** (must handle nodes already beneath it; a removed edge
  could orphan existing children). Follow-up spec.
- **Edge cardinality UI** (`min`/`max` per containment) — deferred to a later "advanced" toggle;
  the serializer leaves room (bare `{}` per edge in v1) so adding it later needs no format change.
- **Multiple named presets** — single standard preset for now.
- **Reordering types / metadata fields** by drag — not needed for correctness.
