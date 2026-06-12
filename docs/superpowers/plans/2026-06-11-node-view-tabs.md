# Tabbed Node View Implementation Plan

> **Superseded:** shipped as a dedicated `frontend/src/pages/node.astro` page
> the tree links to, with the `masterdata` page/component retired. This plan
> describes the original (masterdata-component) approach, not the as-built one.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the masterdata page's tabs (Data / Energipriser / CO2e faktorer / Licens) onto the per-node view: select a node, see its metadata by default, flip between the other (stub) tabs for that same node in place.

**Architecture:** Frontend-only (Approach A). The `MasterData` component becomes the tabbed node view — an Alpine `x-show` tab bar whose **Data** panel is the existing HTMX-loaded `render_node` fragment and whose Energipriser/CO2/Licens panels are per-node stubs. The masterdata page drops its own page-level tabs and renders the tabbed component directly. No backend change.

**Tech Stack:** Astro, Alpine.js, HTMX.

**Spec:** `docs/superpowers/specs/2026-06-11-node-view-tabs-design.md`

**Working dir for commands:** `/home/sla/projects/ems_rust/frontend`

---

### Task 1: Turn `MasterData.astro` into the tabbed node view

**Files:**
- Modify (replace whole file): `frontend/src/components/MasterData.astro`

The current file is just the node-detail HTMX loader. Replace it with a tab bar + the same loader as the Data panel + three per-node stub panels.

- [ ] **Step 1: Replace the component**

Overwrite `frontend/src/components/MasterData.astro` with:
```astro
---
const API_BASE_URL = import.meta.env.PUBLIC_API_BASE_URL;
---

<div x-data="{ tab: 'data' }" @node-selected.window="tab = 'data'">
  <!-- Tab bar (per-node) -->
  <div class="tab-bar">
    <button class="tab-btn" :class="tab === 'data' ? 'active' : ''" @click="tab = 'data'">Data</button>
    <button class="tab-btn" :class="tab === 'prices' ? 'active' : ''" @click="tab = 'prices'">Energipriser</button>
    <button class="tab-btn" :class="tab === 'co2' ? 'active' : ''" @click="tab = 'co2'">CO2e faktorer</button>
    <button class="tab-btn" :class="tab === 'license' ? 'active' : ''" @click="tab = 'license'">Licens</button>
  </div>

  <!-- Data panel: the server-rendered node detail (render_node), loaded on node-selected -->
  <div x-show="tab === 'data'"
    hx-get={`${API_BASE_URL}/hierarchy/query/node`}
    hx-request='{"noHeaders": true}'
    hx-vals="js:{id: sessionStorage.getItem('selectedNodeId'), user: localStorage.getItem('loginId'), path: sessionStorage.getItem('selectedNodePath')}"
    hx-trigger="node-selected from:window"
    hx-swap="innerHTML"
    id="master-data-content">
    <p style="color: var(--text-muted); padding: 1.5rem;">Select a node from the hierarchy to view its data...</p>
  </div>

  <!-- Stub panels (per-node placeholders; seam for future hx-get to a per-node endpoint) -->
  <div x-show="tab === 'prices'" id="node-energy-panel" style="display:none">
    <div style="padding: 1.5rem;">
      <h2 class="section-title">Energipriser</h2>
      <p style="color: var(--text-muted);">Ingen data for denne node endnu.</p>
    </div>
  </div>
  <div x-show="tab === 'co2'" id="node-co2-panel" style="display:none">
    <div style="padding: 1.5rem;">
      <h2 class="section-title">CO2e faktorer</h2>
      <p style="color: var(--text-muted);">Ingen data for denne node endnu.</p>
    </div>
  </div>
  <div x-show="tab === 'license'" id="node-license-panel" style="display:none">
    <div style="padding: 1.5rem;">
      <h2 class="section-title">Licens</h2>
      <p style="color: var(--text-muted);">Ingen data for denne node endnu.</p>
    </div>
  </div>
</div>
```

Notes for the implementer:
- The Data panel keeps the **exact** HTMX attributes from the old file (`id="master-data-content"`,
  the `hx-get`/`hx-vals`/`hx-trigger`/`hx-swap`) so node loading is unchanged.
- The three stub panels start with `style="display:none"` so they don't flash before Alpine
  initialises; Alpine's `x-show` then governs visibility (it sets `display` to visible when its tab
  is active).
- `@node-selected.window="tab = 'data'"` resets to the Data tab whenever a node is picked in the
  tree, so you always land on metadata first.

- [ ] **Step 2: Verify the build compiles**

Run: `npm run build`
Expected: succeeds (21 pages). The masterdata page still builds (it imports this component).

- [ ] **Step 3: Confirm the tab structure is in the built output**

Run: `grep -o 'tab === .data.\|tab === .prices.\|node-energy-panel\|id="master-data-content"' dist/masterdata/index.html | sort -u`
Expected: shows the `tab === 'data'`, `tab === 'prices'`, `node-energy-panel`, and `master-data-content` markers (the tab bar, a stub, and the unchanged node-detail loader are all present).

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/MasterData.astro
git commit -m "feat(frontend): MasterData becomes the tabbed node view"
```

---

### Task 2: Drop the page-level tabs from the masterdata page

**Files:**
- Modify: `frontend/src/pages/masterdata.astro`

The page-container `x-data` mixes tab state with the delete-dialog logic. Remove only the tab parts;
keep the delete methods. Then replace the page-level tab bar + the four tab-content divs with a bare
`<MasterData />` (which now owns the tabs).

- [ ] **Step 1: Remove `activeTab` from the page `x-data`**

In `frontend/src/pages/masterdata.astro`, delete the `activeTab: 'maindata',` line inside the
page-container `x-data` (it is the first property, immediately before `deleteDialogOpen: false,`):
```
		activeTab: 'maindata',
```
Leave `deleteDialogOpen` and the three methods (`openDeleteDialog` / `closeDeleteDialog` /
`deleteNode`) untouched.

- [ ] **Step 2: Remove the two tab-reset window handlers on the page-container div**

The page-container `<div class="page-container" x-data="{ … }"` opening tag ends with these two
attributes:
```
		@switch-to-overview.window="activeTab = 'maindata'"
		@node-selected.window="activeTab = 'maindata'">
```
Delete both attribute lines so the div opening tag ends with `}"` followed by `>` (i.e. the `x-data`
object is the last attribute). The node-selection → Data-tab reset now lives in the `MasterData`
component (Task 1), so nothing on the page needs these.

- [ ] **Step 3: Replace the page-level tab bar + tab content with `<MasterData />`**

Replace this whole block (the `<!-- Tab Navigation -->` tab bar through the closing `</div>` of the
`activeTab === 'license'` panel) —

```astro
			<!-- Tab Navigation -->
			<div class="tab-bar">
				<button
					class="tab-btn"
					:class="activeTab === 'maindata' ? 'active' : ''"
					@click="activeTab = 'maindata'"
				>
					Data
				</button>
				<button
					class="tab-btn"
					:class="activeTab === 'prices' ? 'active' : ''"
					@click="activeTab = 'prices'"
				>
					Energipriser
				</button>
				<button
					class="tab-btn"
					:class="activeTab === 'co2' ? 'active' : ''"
					@click="activeTab = 'co2'"
				>
					CO2e faktorer
				</button>
				<button
					class="tab-btn"
					:class="activeTab === 'license' ? 'active' : ''"
					@click="activeTab = 'license'"
				>
					Licens
				</button>
			</div>

			<!-- Tab Content -->
			<div x-show="activeTab === 'maindata'">
				<MasterData />
			</div>

			<div x-show="activeTab === 'prices'">
				<h1 class="page-title">Priser</h1>
			</div>

			<div x-show="activeTab === 'co2'">
				<h1 class="page-title">CO2</h1>
			</div>

			<div x-show="activeTab === 'license'">
				<h1 class="page-title">License</h1>
			</div>
```

— with just:
```astro
			<!-- Tabbed per-node view -->
			<MasterData />
```

(If the implementer finds the exact whitespace differs, match by the visible structure: the
`<!-- Tab Navigation -->` tab-bar block plus the four `x-show="activeTab === …"` content divs are
removed and replaced by a single `<MasterData />`.)

- [ ] **Step 4: Verify build + that page-level tabs are gone, node view present**

Run: `npm run build`
Expected: succeeds.

Run: `grep -c "activeTab" dist/masterdata/index.html`
Expected: `0` (no page-level tab state remains).

Run: `grep -o 'tab === .data.\|Handlinger' dist/masterdata/index.html | sort -u`
Expected: shows both `tab === 'data'` (the component's tabs are there) and `Handlinger` (the action
dropdown is preserved).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/pages/masterdata.astro
git commit -m "feat(frontend): masterdata page renders the tabbed node view directly"
```

---

### Task 3: Final verification

- [ ] **Step 1: Build is green**

Run: `npm run build`
Expected: succeeds, 21 pages.

- [ ] **Step 2: Confirm the delete flow + create dialog are intact (no regression)**

Run: `grep -o 'openDeleteDialog\|create-dialog\|deleteNode' dist/masterdata/index.html | sort -u`
Expected: shows `openDeleteDialog`, `deleteNode`, and `create-dialog` — the Handlinger actions and
dialogs are untouched.

- [ ] **Step 3: Clean tree**

Run: `git status --short`
Expected: only intended files committed (clean, aside from any pre-existing untracked dirs).

---

## Manual verification (after deploy — NOT executed by this plan)

Log into the live site, open the page hosting the hierarchy + node view, and confirm:
1. Selecting a node shows the **Data** tab with the node detail (metadata + add-child + sensors).
2. Clicking **Energipriser / CO2e faktorer / Licens** switches panels in place (no reload), each
   showing the "Ingen data for denne node endnu." stub.
3. Selecting a *different* node returns to the **Data** tab and reloads the node detail.
4. With no node selected, the Data panel shows "Select a node…".
5. The **Handlinger** dropdown (Opret/Slet) and the create/delete dialogs still work.

## Deployment runbook (manual — NOT executed by this plan)

Frontend-only (account `339712745226`):
```bash
cd frontend && npm run build
cd ../infra/frontend && unset GOROOT
AWS_PROFILE=stel-sb cdk deploy OcamlFrontendStack --require-approval never
```
No backend deploy; `PUBLIC_*` unchanged.
