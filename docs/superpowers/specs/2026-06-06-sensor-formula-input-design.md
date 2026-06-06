# Sensor Formula Input — Design Spec

**Date:** 2026-06-06
**Status:** Draft
**Component:** `services/hierarchy`

## Context

When adding a sensor, the backend domain already supports a per-sensor `formula`
(`lib/domain/formula.ml`): `Identity`, `Zero`, or `Expr { ast; refs }` where `ast`
is an expression over `Self`, `Num`, `Ref alias`, `Abs`, and `+ − × ÷`, and `refs`
binds each alias to another sensor's id. The codec already persists it to DynamoDB.

However, **the formula is never accepted from the API or the UI**: `api_command.ml`'s
`run_attach_sensor` does not parse a formula, `api_json.ml` neither reads nor emits one,
and the add-sensor form in `api_html.ml` has no formula control. Every sensor is created
with the default `Identity`. There is also no text parser — only the codec's tagged AST
for persistence.

This spec adds the ability to specify a formula when adding a sensor, defaulting to
`Identity`, supporting the full backend model including cross-sensor references, entered as
a **text expression with alias→sensor binding**. Because `Identity` is the overwhelmingly
common case, the main add-sensor form stays uncluttered and formula building happens in a
**separate, opt-in dialog**.

## Goals

- Specify a formula on the add-sensor form; default remains `Identity`.
- Support `Identity`, `Zero`, and full text expressions with cross-sensor references.
- Honor existing backend constraints (parse validity, all aliases bound, cycle-free).
- Keep the common path (Identity) friction-free — no expression UI in the main form.

## Non-Goals (v1)

- Editing a sensor's formula after creation (add-only for now).
- Autocomplete / live evaluation / preview of formula results.

## Formula text grammar

A small recursive-descent grammar, owned by the backend (single source of truth):

```
expr    := term (('+' | '-') term)*
term    := factor (('*' | '/') factor)*
factor  := '-' factor | primary
primary := number
         | 'self'
         | ident                       (* alias → Ref *)
         | 'abs' '(' expr ')'
         | '(' expr ')'
number  := float literal (e.g. 3, 3.5, .5, 1e3)
ident   := [A-Za-z_][A-Za-z0-9_]*      (* excluding reserved 'self', 'abs' *)
```

- `self` → `Self`; numeric literal → `Num`; `ident` → `Ref ident`.
- Unary minus desugars to `Sub (Num 0., factor)`.
- `*` / `/` bind tighter than `+` / `-`; left-associative.
- Whitespace insignificant. Parse errors return a human-readable message.

## Wire format

The form posts flat `data.*` fields; the existing `handler.ml:set_path` decoder turns dotted
keys into nested JSON, so no handler changes are needed. The formula is carried as:

```
data.formula.kind = "identity" | "zero" | "expr"
data.formula.expr = "abs(self - a - b)"          # only when kind=expr
data.formula.refs = '{"a":"S#12","b":"S#19"}'    # JSON string, only when kind=expr
```

which decodes to:

```json
{ "formula": { "kind": "expr",
               "expr": "abs(self - a - b)",
               "refs": { "a": "S#12", "b": "S#19" } } }
```

`refs` is accepted as **either** a JSON object (programmatic callers) **or** a JSON-encoded
string (the form, which builds it client-side to avoid dynamic field names). Absent
`formula` → `Identity` (backward compatible with existing callers).

## Component design

### 1. `lib/domain/formula_parser.ml` (new, pure)

`parse : string -> (Formula.expr, string) result` implementing the grammar above.
No effects, fully unit-testable.

### 2. `lib/domain/formula.ml` (extend)

- `expr_aliases : expr -> string list` — distinct alias names referenced by an AST
  (used to validate every alias is bound).
- `to_string : t -> string` / `expr_to_string : expr -> string` — unparser, for round-trip
  display in `sensor_to_json` and the (future) edit flow. Renders with minimal parentheses.

### 3. Cross-sensor reference enumeration (company-wide)

The reference picker must offer sensors across the **company (HN2)** the parent belongs to —
not just siblings. (HN1 = partner, HN2 = company; the per-company schema lives on the HN2 node
and governs its subtree, so HN2 is the correct boundary — partner-wide would cross companies
and schemas.) Storage already supports subtree enumeration via the sensor GSI partition
(`begins_with(gsi1sk, <ancestor_path>)`), the same mechanism the delete path uses
(`repo/dynamo.ml`).

- **`lib/effects.ml`**: add effect `List_sensors_under_path : string -> Sensor.t list` (active
  sensors whose `path` begins with the given prefix).
- **`lib/repo/dynamo.ml`**: handle it via a sensor-GSI prefix query on the company path.
- **`lib/repo/memory.ml`**: handle it by filtering stored active sensors whose `path` starts
  with the prefix.
- **`lib/logic/sensors.ml`**: `list_under_company : parent:Node_id.t -> (Sensor.t list, _) result`
  — find the HN2 (company) segment in the parent node's path (or the parent itself if it is
  HN2), build the prefix `root|HN1#…|HN2#…` **with a trailing path separator** (to avoid
  `HN2#1` matching `HN2#10`), then perform the effect. If the parent has no HN2 ancestor
  (sensors are not allowed that high, so this is defensive) → empty list.

### 4. `lib/api/api_json.ml`

- `formula_of_json : Yojson.Safe.t option -> (Formula.t, string) result`:
  - absent / `null` → `Identity`.
  - `{kind:"identity"}` → `Identity`; `{kind:"zero"}` → `Zero`.
  - `{kind:"expr", expr, refs}` → `Formula_parser.parse expr`; parse `refs` (object or JSON
    string) into `(alias, Sensor_id.t) list`; **error if any alias in `expr_aliases ast` is
    unbound**, or any bound id fails `Sensor_id.of_string`. Build `Expr { ast; refs }`.
  - Missing `kind` but `expr` present → treat as `expr`.
- `sensor_to_json`: add a `formula` field:
  - `Identity` → `{kind:"identity"}`, `Zero` → `{kind:"zero"}`,
  - `Expr` → `{kind:"expr", expr:<to_string>, refs:{alias:"S#id", …}}`.

### 5. `lib/api/api_command.ml`

`run_attach_sensor` reads `field json "formula"`, runs `formula_of_json`, and passes
`?formula` to `Sensors.attach`. Parse/validation errors become a 400 via the existing
`error_response`. No formula → `Identity` (unchanged behavior).

### 6. `lib/api/api_html.ml` — two-dialog UX

**Main add-sensor dialog** stays clean. A compact Formula row:
- A read-only summary, default text **"Identity (default)"**, plus an **"Edit formula…"** button.
- Three hidden inputs hold the result: `data.formula.kind` (default `identity`),
  `data.formula.expr`, `data.formula.refs`.
- The button opens the formula dialog (`showModal()`); modal `<dialog>` elements stack in the
  top layer, so nesting over the add-sensor dialog is fine.

**Formula dialog** (`<dialog id="formula-dialog">`):
- Kind select: **Identity (default)** / Zero / Expression. Expression sub-section is shown only
  for "Expression" (hyperscript toggle).
- Expression sub-section: a text input for the expression, and a **References** area — rows of
  `[alias] → [sensor ▾]` with a "+ Add reference" button (hyperscript clones a row template).
- The sensor `<select>` options come from a new htmx fragment endpoint (below), loaded into a
  hidden `<template>` when the add-sensor dialog opens; each new ref row clones those options.
- **Apply** runs an inline script that: validates non-empty expr, builds the `refs` JSON object
  from the rows, writes the three hidden inputs in the main form, updates the summary text
  (e.g. `abs(self − a − b)`), and closes the formula dialog. **Cancel** leaves the main form
  unchanged (still Identity).

**New htmx fragment endpoint** — company sensor options:
- Route (mirroring `/hierarchy/query/sensors`): returns `<option value="S#id">daq (purpose)</option>`
  for `Sensors.list_under_company ~parent`. Rendered in `api_html.ml` like `render_sensors`.

## Validation / constraints honored

- Parser rejects malformed expressions with a clear message → 400.
- Every alias used in the expression must be bound in `refs`, else 400.
- The existing post-allocation `has_cycle` check in `Sensors.attach` still rolls back
  self-referential or cyclic formulas.
- The backend stays lenient about whether a referenced sensor currently exists (unchanged);
  the dropdown only offers existing company sensors.

## Testing

- **`test_domain_formula`** (parser + unparser): precedence, unary minus, `abs`, parens, `self`,
  aliases, whitespace, error cases (unbalanced parens, bad tokens), `to_string` round-trip.
- **`test_repo_memory`**: `List_sensors_under_path` returns the subtree's active sensors and
  excludes sensors outside the prefix.
- **`test_logic_sensors`**: `list_under_company` derives the right company prefix.
- **`test_api_json` / `test_api_command`**: attach with no formula (→ Identity), `zero`,
  `expr`+refs (object and JSON-string forms); unbound alias → error; malformed id → error;
  cyclic formula → rollback error; `sensor_to_json` emits/round-trips each kind.

## Rollout / backward compatibility

- Purely additive. Existing callers that omit `formula` continue to get `Identity`.
- No DynamoDB schema change (the `formula` attribute and codec already exist).
- No `handler.ml` change (dotted-key decoding already supported).
