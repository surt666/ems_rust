# Tabbed Node View — Design Spec

**Date:** 2026-06-11
**Status:** Superseded by what shipped. The tabbed view landed as a dedicated,
unlinked Astro page (`frontend/src/pages/node.astro`) that the hierarchy tree
links to (HTML over the wire, no JSON); the `masterdata` page/component was
**retired** rather than repurposed. The Data tab is still the backend
`render_node` HTMX fragment, as designed. Treat this doc as the original design
intent, not the as-built layout.
**Surface:** Frontend (Astro + Alpine + HTMX, account `339712745226` S3+CloudFront)
**Backend:** unchanged (`render_node` in the hierarchy lambda is reused as the Data tab content)

## Context & goal

Today the **masterdata page** (`frontend/src/pages/masterdata.astro`) has four page-level tabs —
**Data, Energipriser, CO2e faktorer, Licens**. Only "Data" has content: it renders the
`MasterData` component, which HTMX-loads the server-rendered node detail (`render_node`) for the
node selected in the hierarchy tree. The other three tabs are empty placeholders (`<h1>` headings),
and there are **no energy-price / CO2 / license data sources or endpoints anywhere** in the codebase.

The goal is to move those tabs **onto the node view**: pick a node in the tree, see its metadata by
default, and flip between Energipriser / CO2 / Licens for that same node in place. The standalone
masterdata page/menu is intended to go away eventually.

**This iteration (scope):** build the tabbed node view — the **Data** tab (existing per-node
metadata) plus **Energipriser / CO2 / Licens** as per-node **stub** panels. No data wiring for the
three new tabs (no source exists). The Alpine tab pattern already used on the masterdata page is
relocated onto the node view.

## Approach (chosen: A — frontend tab shell wrapping the server node detail)

The tab chrome is a frontend concern; the Data tab's content stays the server-rendered `render_node`
fragment. Rejected alternatives: server-rendered tabs inside `render_node` (couples tab chrome into
an already-large Rust function; future data tabs become server markup instead of independent
endpoints) and a brand-new Layout-hosted node view (pulls the deferred menu-removal/relocation into
this iteration).

## Components & structure

### `frontend/src/components/MasterData.astro` — becomes the tabbed node view

Wrap the component in `x-data="{ tab: 'data' }"` and render:

- **Tab bar** (reusing the existing `.tab-bar` / `.tab-btn` styles): Data · Energipriser ·
  CO2e faktorer · Licens. `Data` is selected by default. Buttons set `tab` and get the `active`
  class via `:class`.
- **Data panel** (`x-show="tab === 'data'"`): the existing node-detail loader — unchanged:
  ```html
  <div hx-get={`${API_BASE_URL}/hierarchy/query/node`}
       hx-request='{"noHeaders": true}'
       hx-vals="js:{id: sessionStorage.getItem('selectedNodeId'), user: localStorage.getItem('loginId'), path: sessionStorage.getItem('selectedNodePath')}"
       hx-trigger="node-selected from:window"
       hx-swap="innerHTML"
       id="master-data-content">
    <p>Select a node from the hierarchy to view its data…</p>
  </div>
  ```
- **Energipriser / CO2 / Licens panels** (`x-show="tab === '…'"`): per-node stub placeholders
  (see Stub panels). Each is a container shaped for a future `hx-get` to a per-node endpoint.

### `frontend/src/pages/masterdata.astro` — drops its page-level tabs

Remove the outer `<div class="tab-bar">` (the four page-level tab buttons) and the four
`x-show="activeTab === …"` content divs (the `<MasterData />` wrapper + the three placeholder
`<h1>` divs). The page renders the now-tabbed `<MasterData />` directly. The "Handlinger" dropdown
(create / delete / edit) and the create/delete dialogs are unchanged.

The page-container `x-data` currently holds **both** tab state and the delete-dialog logic. Remove
only the tab pieces — the `activeTab` property and the `@switch-to-overview.window` /
`@node-selected.window` handlers that set it — and **keep** the delete-dialog methods
(`openDeleteDialog` / `closeDeleteDialog` / `deleteNode`) and their refs. The node-selection reset
to the Data tab now lives on the `MasterData` component (see Tab behavior), not the page.

## Tab behavior

- **Default:** `tab = 'data'`.
- **Switching:** client-side via Alpine `x-show` — no network, no reload.
- **Node selection:** when the tree fires `node-selected` (window event), reset `tab` to `'data'`
  and let the Data panel's existing `hx-trigger="node-selected from:window"` reload it. Implement
  the reset with an Alpine `@node-selected.window="tab = 'data'"` on the component root. The stub
  panels read the selected node from `sessionStorage` (`selectedNodeId` / `selectedNodePath`) so
  they reflect the current node.
- **Empty state:** with no node selected, the Data panel keeps showing the existing
  "Select a node…" message.

## Stub panels (Energipriser / CO2 / Licens)

Each panel renders a minimal, per-node placeholder — a section heading (the Danish tab label) and a
muted "Ingen data for denne node endnu." line, optionally echoing the selected node id/name from
`sessionStorage` for context. No backend calls. The markup is wrapped in a container with a stable
id per tab (e.g. `id="node-energy-panel"`) so a later iteration can attach `hx-get` to a per-node
endpoint and swap real content in without restructuring.

## Testing

- `npm run build` succeeds.
- **Manual verification:** select a node → Data tab shows the node detail; clicking Energipriser /
  CO2 / Licens switches panels in place (no reload); selecting a different node returns to Data and
  reloads it; with no node selected the "Select a node…" message shows.
- A pure Alpine `x-show` tab shell is low-risk and not heavily tested. *Optional:* a Playwright
  tab-toggle smoke against a small harness page that mounts the tab bar + panels (no backend) — add
  only if desired; it mirrors the schema-designer smoke pattern.

## Out of scope (deferred)

- **Real content / data sources** for Energipriser, CO2, and Licens (no table/endpoint/feed exists;
  each is a separate data-modelling sub-project).
- **Removing the masterdata menu item** and relocating the node view out of the masterdata page to a
  shared/standalone location.
- Any change to `render_node` or other backend code.

## Deployment

Frontend-only: `cd frontend && npm run build` then
`cd infra/frontend && unset GOROOT && AWS_PROFILE=stel-sb cdk deploy OcamlFrontendStack`. No backend
deploy. `PUBLIC_*` unchanged.
