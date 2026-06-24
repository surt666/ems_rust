# Future spec — schema-driven, level-aware node dashboards

Date: 2026-06-24 · Status: **FUTURE / not yet implemented** (captured per request)

## Idea

Each hierarchy node (the `/node` view) gets a **Dashboard** tab. Today it renders one generic
consumption-overview dashboard (`NodeDashboard.astro`, static + HTMX-ready). The future goal:

1. **Make Dashboard the default tab** of the node view once the backend fragment exists
   (`/query/node/dashboard?id=<nodeId>`). (Implementation note left as a TODO in `node.astro`:
   change `x-data="{ tab: 'data' }"` → `"{ tab: 'dashboard' }"`.)
2. **Show a different dashboard depending on the hierarchy level / node type** — e.g. a
   department/partner dashboard vs. a company dashboard vs. a building dashboard vs. an
   area/meter-group dashboard differ in which widgets, KPIs and breakdowns make sense.

## Why schema-driven

Hierarchy shape is **not fixed** — it is defined per company by the **company schema definition**
(the type-graph / metadata designed in `SchemaDesigner`, see the v2 type-graph work). So the set of
levels and their semantics vary by company. Therefore the dashboard-per-level mapping must be driven
by that schema, not hard-coded to a fixed Partner→Company→Property→Building→Area ladder.

### Sketch (to be designed properly later)
- The company schema definition declares node **types** (and their level/role in the DAG).
- A **dashboard descriptor** is associated with each node type (which widgets/sections to render:
  consumption overview, benchmark, alarms, KPIs, sub-node rollups, …) — either authored in the
  schema designer or defaulted per type-role.
- The node Dashboard tab resolves: `nodeType → dashboard descriptor → render widgets`, scoped to the
  node's data. Backend returns the data per widget; the frontend composes HTML/HTMX fragments
  (reusing the chart islands + card/grid primitives already built).

## Open questions (resolve when implemented)
- Where does the descriptor live — in the schema definition itself, or a separate dashboard-config
  keyed by node type? How is it edited (extend SchemaDesigner, or a new editor)?
- Default dashboards per type-role when no explicit descriptor exists.
- Widget catalogue + per-widget data contracts (endpoints), and rollup semantics for parent nodes.
- Permissions/visibility per role (cf. the EMS rights model).

## Out of scope for now
Everything above. The current deliverable is only: the Dashboard **tab** exists on the node view
with a single generic consumption dashboard (static, HTMX-ready). Level-awareness + schema wiring +
default-tab switch come later, with their own spec → plan cycle.

Related: `docs/superpowers/specs/2026-06-24-frontend-ems-views-design.md`,
`frontend/src/components/NodeDashboard.astro`, `frontend/src/pages/node.astro`.
