# Sensor Map in Node Data Tab — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render a Google Map at the top of the Sensors section in the node Data tab, plotting one Advanced Marker per sensor (hover → `daqid`, click → its Datatilegnelse page).

**Architecture:** Frontend-only. A JS module reads the already-rendered sensor `<li>` rows (the datatilegnelse link + `daq` query param give the href and daqid), resolves each to a coordinate (explicit `data-lat`/`data-lon` if present, else a deterministic placeholder), and plots Google Advanced Markers into a JS-injected panel. Re-runs on every HTMX swap of the sensor list so it stays in sync. Pure coordinate math lives in a separate DOM-free module so `node --test` can cover it.

**Tech Stack:** Astro (client `<script>`), vanilla ES modules, Google Maps JavaScript API (`marker` library / `AdvancedMarkerElement`), Node built-in test runner (`node --test`).

## Global Constraints

- **Scope:** `frontend/` only. No backend / Rust / lambda changes, no deploy.
- **HTMX hypermedia:** JS is presentation glue only — it reads data already in the DOM; it does not fetch or client-render domain JSON.
- **CSS grid, not flex** for any layout.
- **Env vars (Astro inlines `PUBLIC_*` at build time; already set in `frontend/.env`):**
  - `PUBLIC_GOOGLE_MAPS_API_KEY` — Maps JS API key. Blank → show a notice, inject nothing.
  - `PUBLIC_GOOGLE_MAPS_MAP_ID` — Map ID for Advanced Markers. Blank → fall back to `"DEMO_MAP_ID"`.
- **Test runner:** `npm test` = `node --test src/lib/*.test.js`. New unit tests must be a `src/lib/*.test.js` file and must not need a DOM.
- **Sensor row DOM (rendered by the hierarchy service, do not change):**
  ```html
  <li class="sensor-item sensor-row">
    <div class="sensor-row__label">
      <span class="sensor-row__name">Electricity</span>
      <span class="muted">(daq:adeunis_pu_v1:123:…:counter_a)</span>
    </div>
    <details class="kebab">
      <summary …>⋯</summary>
      <div class="action-dropdown kebab__menu">
        …
        <a href="/measurements/?daq=<enc>&sid=…&logical=…&purpose=…&unit=…&type=…"
           data-astro-reload data-i18n="sensor.menu.datatilegnelse">Gå til datatilegnelse</a>
        …
      </div>
    </details>
  </li>
  ```
  The rows are HTMX-loaded into `<ul id="sensor-list">`. The `daqid` is the `daq` query param of that `href` (URL-encoded).

---

## File Structure

- **Create** `frontend/src/lib/sensor-map-coords.js` — pure coordinate helpers (`BASE`, `hashString`, `placeholderCoord`, `resolveCoord`). No DOM, no `import.meta.env`.
- **Create** `frontend/src/lib/sensor-map-coords.test.js` — `node --test` unit tests for the above.
- **Create** `frontend/src/lib/sensor-map.js` — DOM + Google Maps glue: loader, panel injection, row parsing, marker plotting, HTMX-swap sync. Imports `sensor-map-coords.js`.
- **Modify** `frontend/src/pages/node.astro` — add a client `<script>` that calls `initSensorMap()` (on load + `astro:page-load`) and a `<style is:global>` block for the panel + pins.
- **Modify** `frontend/.env.example` — document the two `PUBLIC_GOOGLE_MAPS_*` vars.

---

### Task 1: Pure coordinate helpers + tests

**Files:**
- Create: `frontend/src/lib/sensor-map-coords.js`
- Test: `frontend/src/lib/sensor-map-coords.test.js`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `BASE: { lat: number, lng: number }`
  - `hashString(s: string): number` — unsigned 32-bit.
  - `placeholderCoord(daqid: string): { lat: number, lng: number }`
  - `resolveCoord(explicit: { lat?: any, lon?: any }, daqid: string): { lat: number, lng: number }`

- [ ] **Step 1: Write the failing test**

Create `frontend/src/lib/sensor-map-coords.test.js`:

```js
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  BASE,
  hashString,
  placeholderCoord,
  resolveCoord,
} from "./sensor-map-coords.js";

const SPREAD = 0.0015; // must match the module

test("hashString is a stable unsigned 32-bit number", () => {
  const h = hashString("daq:abc");
  assert.equal(h, hashString("daq:abc"));
  assert.ok(Number.isInteger(h) && h >= 0 && h <= 0xffffffff);
});

test("placeholderCoord is deterministic for the same daqid", () => {
  assert.deepEqual(placeholderCoord("daq:abc"), placeholderCoord("daq:abc"));
});

test("placeholderCoord differs for different daqids", () => {
  assert.notDeepEqual(placeholderCoord("daq:abc"), placeholderCoord("daq:xyz"));
});

test("placeholderCoord stays within SPREAD of BASE", () => {
  for (const id of ["a", "daq:1", "counter_a", "S#20001", ""]) {
    const c = placeholderCoord(id);
    assert.ok(Math.abs(c.lat - BASE.lat) <= SPREAD, `lat in range for ${id}`);
    assert.ok(Math.abs(c.lng - BASE.lng) <= SPREAD, `lng in range for ${id}`);
  }
});

test("resolveCoord prefers explicit finite lat/lon", () => {
  assert.deepEqual(resolveCoord({ lat: "10.5", lon: "20.25" }, "daq:abc"), {
    lat: 10.5,
    lng: 20.25,
  });
});

test("resolveCoord falls back to placeholder when lat/lon missing or non-finite", () => {
  assert.deepEqual(resolveCoord({}, "daq:abc"), placeholderCoord("daq:abc"));
  assert.deepEqual(
    resolveCoord({ lat: "nope", lon: "20" }, "daq:abc"),
    placeholderCoord("daq:abc"),
  );
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd frontend && node --test src/lib/sensor-map-coords.test.js`
Expected: FAIL — cannot find module `./sensor-map-coords.js`.

- [ ] **Step 3: Write minimal implementation**

Create `frontend/src/lib/sensor-map-coords.js`:

```js
// Pure, DOM-free coordinate helpers for the sensor map. Unit-tested with
// `node --test`. No `import.meta.env` here — keep this importable by the runner.

/** Base point placeholder markers cluster around (Copenhagen). Swap for a real
 *  building coordinate when validated addresses land. */
export const BASE = { lat: 55.6761, lng: 12.5683 };

/** Max placeholder offset from BASE, in degrees (~150 m). */
const SPREAD = 0.0015;

/** Deterministic unsigned 32-bit FNV-1a hash of a string. */
export function hashString(s) {
  let h = 0x811c9dc5;
  const str = String(s);
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Map a 16-bit value to [-1, 1]. */
function unit16(n) {
  return (n / 0xffff) * 2 - 1;
}

/** Deterministic placeholder coordinate for a daqid, clustered around BASE. */
export function placeholderCoord(daqid) {
  const h = hashString(daqid);
  const latOff = unit16(h & 0xffff) * SPREAD;
  const lngOff = unit16((h >>> 16) & 0xffff) * SPREAD;
  return { lat: BASE.lat + latOff, lng: BASE.lng + lngOff };
}

/** Prefer an explicit finite {lat, lon}; otherwise fall back to the placeholder. */
export function resolveCoord(explicit, daqid) {
  const lat = Number(explicit?.lat);
  const lon = Number(explicit?.lon);
  if (Number.isFinite(lat) && Number.isFinite(lon)) return { lat, lng: lon };
  return placeholderCoord(daqid);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd frontend && node --test src/lib/sensor-map-coords.test.js`
Expected: PASS — all 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/sensor-map-coords.js frontend/src/lib/sensor-map-coords.test.js
git commit -m "feat(frontend): pure coordinate helpers for the sensor map"
```

---

### Task 2: Map glue module (loader, panel, markers, sync)

**Files:**
- Create: `frontend/src/lib/sensor-map.js`

**Interfaces:**
- Consumes: `resolveCoord` from `./sensor-map-coords.js`; `import.meta.env.PUBLIC_GOOGLE_MAPS_API_KEY`, `import.meta.env.PUBLIC_GOOGLE_MAPS_MAP_ID`.
- Produces: `initSensorMap(): void` — idempotent; registers the HTMX-swap listener once and triggers an initial sync. Safe to call on every `astro:page-load`.

This module has no unit test (DOM + Google Maps + `import.meta.env`); it is covered by the Task 4 smoke check. Because there is no test, review the logic carefully at implementation time.

- [ ] **Step 1: Write the module**

Create `frontend/src/lib/sensor-map.js`:

```js
// Sensor map glue: reads the server-rendered sensor <li> rows, plots one Google
// Advanced Marker per sensor, keeps them in sync across HTMX swaps of #sensor-list.
// Presentation only — all data comes from the DOM (hypermedia). See
// docs/superpowers/specs/2026-07-20-sensor-map-datatab-design.md.
import { resolveCoord } from "./sensor-map-coords.js";

const API_KEY = import.meta.env.PUBLIC_GOOGLE_MAPS_API_KEY || "";
const MAP_ID = import.meta.env.PUBLIC_GOOGLE_MAPS_MAP_ID || "DEMO_MAP_ID";

let map = null;
let markers = [];
let syncSeq = 0; // guards against overlapping async syncs
let mapsPromise = null;
const watched = new WeakSet();

/** Load the Google Maps JS API once (marker library included). Resolves google.maps. */
function loadGoogleMaps() {
  if (mapsPromise) return mapsPromise;
  mapsPromise = new Promise((resolve, reject) => {
    if (window.google?.maps) return resolve(window.google.maps);
    const cb = "__sensorMapGmapsReady";
    window[cb] = () => resolve(window.google.maps);
    const s = document.createElement("script");
    s.src =
      "https://maps.googleapis.com/maps/api/js?key=" +
      encodeURIComponent(API_KEY) +
      "&libraries=marker&v=weekly&loading=async&callback=" +
      cb;
    s.async = true;
    s.onerror = () => reject(new Error("Google Maps failed to load"));
    document.head.appendChild(s);
  });
  return mapsPromise;
}

/** Ensure the map panel (canvas + notice) exists directly before #sensor-list. */
function ensurePanel() {
  const list = document.getElementById("sensor-list");
  if (!list) return null;
  let panel = document.getElementById("sensor-map-panel");
  if (panel && panel.isConnected) return panel;
  panel = document.createElement("div");
  panel.id = "sensor-map-panel";
  panel.innerHTML =
    '<div class="sensor-map__canvas"></div>' +
    '<p class="sensor-map__notice" hidden></p>';
  list.parentNode.insertBefore(panel, list);
  map = null; // fresh panel → the old map DOM is gone
  return panel;
}

function showNotice(panel, msg) {
  const canvas = panel.querySelector(".sensor-map__canvas");
  const notice = panel.querySelector(".sensor-map__notice");
  canvas.hidden = true;
  notice.hidden = false;
  notice.textContent = msg;
}

function hideNotice(panel) {
  panel.querySelector(".sensor-map__canvas").hidden = false;
  panel.querySelector(".sensor-map__notice").hidden = true;
}

/** Read the current sensor rows into { daqid, href, coord } records. */
function readRows() {
  const rows = [];
  const list = document.getElementById("sensor-list");
  if (!list) return rows;
  for (const li of list.querySelectorAll("li.sensor-row")) {
    const link = li.querySelector('a[href^="/measurements/"]');
    if (!link) continue;
    const href = link.getAttribute("href");
    let daqid = li.dataset.daq;
    if (!daqid) {
      try {
        daqid = new URL(href, window.location.origin).searchParams.get("daq") || "";
      } catch {
        daqid = "";
      }
    }
    if (!daqid) daqid = "(unknown)";
    const coord = resolveCoord({ lat: li.dataset.lat, lon: li.dataset.lon }, daqid);
    rows.push({ daqid, href, coord });
  }
  return rows;
}

/** A marker's content: a dot with a daqid label revealed on hover (CSS). */
function buildPin(daqid) {
  const el = document.createElement("div");
  el.className = "sensor-pin";
  const dot = document.createElement("span");
  dot.className = "sensor-pin__dot";
  const label = document.createElement("span");
  label.className = "sensor-pin__label";
  label.textContent = daqid;
  el.append(dot, label);
  return el;
}

/** Re-run syncMap once the canvas actually gains a size (e.g. Data tab shown). */
function ensureVisibilityWatcher(canvas) {
  if (watched.has(canvas) || typeof ResizeObserver === "undefined") return;
  watched.add(canvas);
  const ro = new ResizeObserver(() => {
    if (canvas.clientWidth > 0 && canvas.clientHeight > 0) syncMap();
  });
  ro.observe(canvas);
}

/** Idempotent: (re)build the panel and markers from the current DOM. */
async function syncMap() {
  const seq = ++syncSeq;
  const panel = ensurePanel();
  if (!panel) return;
  const canvas = panel.querySelector(".sensor-map__canvas");

  if (!API_KEY) {
    showNotice(panel, "Set PUBLIC_GOOGLE_MAPS_API_KEY to enable the sensor map.");
    return;
  }
  const rows = readRows();
  if (rows.length === 0) {
    showNotice(panel, "No sensors to map.");
    return;
  }

  let maps;
  try {
    maps = await loadGoogleMaps();
  } catch {
    showNotice(panel, "Could not load Google Maps.");
    return;
  }
  if (seq !== syncSeq) return;

  hideNotice(panel);
  // The Data tab may be display:none on first load — a zero-size canvas can't
  // host a map. Arm a one-shot observer and bail until it becomes visible.
  if (canvas.clientWidth === 0 || canvas.clientHeight === 0) {
    ensureVisibilityWatcher(canvas);
    return;
  }

  const { Map } = await maps.importLibrary("maps");
  const { AdvancedMarkerElement } = await maps.importLibrary("marker");
  if (seq !== syncSeq) return;

  if (!map) {
    map = new Map(canvas, { mapId: MAP_ID, center: rows[0].coord, zoom: 15 });
  }
  for (const m of markers) m.map = null;
  markers = [];

  const bounds = new maps.LatLngBounds();
  for (const r of rows) {
    const marker = new AdvancedMarkerElement({
      map,
      position: r.coord,
      content: buildPin(r.daqid),
      title: r.daqid,
      gmpClickable: true,
    });
    marker.addListener("gmp-click", () => window.location.assign(r.href));
    markers.push(marker);
    bounds.extend(r.coord);
  }

  if (rows.length === 1) {
    map.setCenter(rows[0].coord);
    map.setZoom(16);
  } else {
    map.fitBounds(bounds, 48);
  }
}

/** Register the HTMX-swap sync once and trigger an initial render. Idempotent. */
export function initSensorMap() {
  if (!window.__sensorMapInit) {
    window.__sensorMapInit = true;
    document.addEventListener("htmx:afterSwap", (e) => {
      const t = e.detail?.target;
      if (!t) return;
      if (t.id === "sensor-list" || (t.querySelector && t.querySelector("#sensor-list"))) {
        syncMap();
      }
    });
  }
  syncMap();
}
```

- [ ] **Step 2: Type-check the build compiles (module is imported in Task 3; verify syntax now)**

Run: `cd frontend && node --check src/lib/sensor-map.js`
Expected: no output (syntax OK). Note `import.meta.env` is a Vite/Astro construct — `node --check` only parses, it does not execute, so this passes.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/sensor-map.js
git commit -m "feat(frontend): Google Advanced Markers glue for the sensor map"
```

---

### Task 3: Wire the map into the node Data tab

**Files:**
- Modify: `frontend/src/pages/node.astro` (add `<script>` + `<style is:global>` after `</Layout>`, currently the file ends at line 112)

**Interfaces:**
- Consumes: `initSensorMap` from `../lib/sensor-map.js`.
- Produces: nothing (page wiring).

- [ ] **Step 1: Add the client script and global styles**

At the end of `frontend/src/pages/node.astro`, immediately **after** the `</Layout>` line, append:

```astro

<script>
  // Plot sensors on a Google map at the top of the Data tab's Sensors section.
  // The module wires its own htmx:afterSwap listener, so it survives the async
  // sensor-list load; re-init on Astro soft navigations too.
  import { initSensorMap } from "../lib/sensor-map.js";
  initSensorMap();
  document.addEventListener("astro:page-load", initSensorMap);
</script>

<style is:global>
  /* Injected by sensor-map.js — the elements are created at runtime and the
     marker pins live inside Google's map DOM, so these must be global (an
     Astro-scoped block would not reach them). */
  #sensor-map-panel {
    margin-bottom: 1rem;
  }
  #sensor-map-panel .sensor-map__canvas {
    width: 100%;
    height: 320px;
    border-radius: 10px;
    overflow: hidden;
  }
  #sensor-map-panel .sensor-map__notice {
    margin: 0;
    padding: 1rem 0;
    color: var(--text-muted);
  }
  .sensor-pin {
    display: grid;
    justify-items: center;
    cursor: pointer;
  }
  .sensor-pin__dot {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--accent, #2563eb);
    border: 2px solid #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
  }
  .sensor-pin__label {
    position: absolute;
    bottom: 20px;
    left: 50%;
    transform: translateX(-50%);
    white-space: nowrap;
    background: rgba(17, 24, 39, 0.92);
    color: #fff;
    font-size: 12px;
    padding: 3px 7px;
    border-radius: 6px;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.1s ease;
  }
  .sensor-pin:hover {
    z-index: 10;
  }
  .sensor-pin:hover .sensor-pin__label {
    opacity: 1;
  }
</style>
```

- [ ] **Step 2: Build to verify the module bundles and env inlines**

Run: `cd frontend && npm run build`
Expected: build succeeds. Confirm the API key was inlined into the bundled node page script:

Run: `cd frontend && grep -rl "maps.googleapis.com/maps/api/js" dist/_astro/ | head`
Expected: at least one bundled JS file matches (the sensor-map module made it into the build).

- [ ] **Step 3: Commit**

```bash
git add frontend/src/pages/node.astro
git commit -m "feat(frontend): show sensor map in the node Data tab"
```

---

### Task 4: Document env vars + manual smoke verification

**Files:**
- Modify: `frontend/.env.example`

**Interfaces:**
- Consumes: nothing.
- Produces: nothing.

- [ ] **Step 1: Document the two vars in `.env.example`**

Append to `frontend/.env.example`:

```bash

# Google Maps — sensor map on the node Data tab (frontend/src/lib/sensor-map.js).
# Maps JavaScript API key (billing-enabled project, "Maps JavaScript API" enabled,
# restricted to the frontend origins). Blank → the map panel shows a "set your key"
# notice instead of a broken map; nothing is injected.
PUBLIC_GOOGLE_MAPS_API_KEY=
# Map ID (required by Advanced Markers; create a Vector JS Map ID). Blank → falls
# back to Google's DEMO_MAP_ID (dev only, shows a watermark).
PUBLIC_GOOGLE_MAPS_MAP_ID=
```

- [ ] **Step 2: Run the full frontend test suite**

Run: `cd frontend && npm test`
Expected: PASS — existing tests plus the 6 new `sensor-map-coords` tests pass.

- [ ] **Step 3: Manual smoke (browser)**

With `frontend/.env` holding the real `PUBLIC_GOOGLE_MAPS_*` values:

```bash
cd frontend && npm run build && npm run preview
```

Then in a browser (see the preview URL, typically `http://localhost:4321`):
1. Navigate to a node with sensors, e.g. `/node?id=HN2%2310003` (SeedCo01).
2. Click the **Data** tab.
3. Expected: a map appears above the sensor list with one pin per sensor.
4. Hover a pin → its `daqid` shows in a tooltip.
5. Click a pin → the browser navigates to `/measurements/?daq=…` for that sensor.

If the map area is grey/blank the first time the tab opens, confirm the pins
appear after the tab becomes visible (the ResizeObserver path) — switch away and
back to the Data tab.

- [ ] **Step 4: Commit**

```bash
git add frontend/.env.example
git commit -m "docs(frontend): document PUBLIC_GOOGLE_MAPS_* env vars"
```

---

## Self-Review Notes

- **Spec coverage:** map placement (Task 3), data-attributes-with-fallback coords (Task 1 `resolveCoord` + Task 2 `readRows`), HTMX-swap sync (Task 2 `initSensorMap`/`syncMap`), Advanced Markers + hover + click (Task 2), placeholder `BASE` (Task 1), Google loader + no-key fallback (Task 2), Map ID default (Task 2), env docs (Task 4), unit + smoke tests (Tasks 1 & 4). All spec sections mapped.
- **Hidden-tab caveat** (Data tab is `display:none` by default while `#sensor-list` still HTMX-loads) is handled by `ensureVisibilityWatcher` — not in the spec but required for the feature to actually render; called out in the smoke step.
- **Type consistency:** `resolveCoord`/`placeholderCoord`/`BASE`/`hashString` names match between Task 1 definition, its test, and Task 2's import. `initSensorMap` matches between Task 2 export and Task 3 import.
```
