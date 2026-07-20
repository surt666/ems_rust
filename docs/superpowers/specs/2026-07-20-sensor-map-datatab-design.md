# Sensor map in the node Data tab — design

**Date:** 2026-07-20
**Status:** Approved for planning
**Scope:** `frontend/` only. No backend / Rust / lambda changes, no deploy.

## Goal

In the node view's **Data tab** (`frontend/src/pages/node.astro`), show a Google Map at
the top of the **Sensors** section that plots each sensor as a point. Hovering a point
shows the sensor's `daqid`; clicking it navigates to that sensor's **Datatilegnelse**
(`/measurements/…`) page — the exact URL the sensor row already links to.

This is a first render. Real per-sensor coordinates and validated addresses are a
**future** effort; today we render placeholder coordinates and structure the code so the
backend can later supply real ones with no frontend rework.

## Context

- The Data tab HTMX-loads the server-rendered `render_node` fragment into `#node-data-panel`.
  That fragment contains the **Sensors** section (`sensor_block`), whose `#sensor-list`
  `<ul>` is itself HTMX-loaded from `/hierarchy/query/sensors` and swapped as `<li>` rows
  (`crates/services/hierarchy/src/html/node.rs`, `render_sensor_row`).
- Each `<li>` already carries what we need: the `daq_id` (rendered text / label) and the
  **Datatilegnelse** link `href` = `/measurements/?daq=…&sid=…&…`.
- The `Sensor` domain type has **no lat/lon field** — so "current lat/lon" do not exist yet.
  Placeholders are invented on the frontend for now.
- Project constraints (standing): **HTMX hypermedia** (no client JSON rendering — but a map
  is inherently JS, so the JS is presentation glue only, reading data already in the DOM);
  **CSS grid, not flex**.

## Approach: data-attributes-with-fallback

The map JS reads coordinates from **`data-lat` / `data-lon` attributes on each `<li>`**.
Those attributes are absent today, so the JS falls back to **deterministic placeholder
coordinates** derived from the `daq_id`. When lat/lon later becomes a real thing, the
backend adds `data-lat`/`data-lon` to `render_sensor_row` and the **same JS** picks them
up — no rework. This is why we do not hardcode coordinates in the map module.

## Components

### 1. `frontend/src/lib/sensor-map.js` (new)

A small ES module, imported by a client `<script>` in `node.astro`. Responsibilities:

- **Maps loader.** Inject the Google Maps JS API `<script>` **once**, with
  `libraries=marker` (Advanced Markers) and `v=weekly`, using
  `import.meta.env.PUBLIC_GOOGLE_MAPS_API_KEY`. If the key is empty/undefined, do **not**
  inject; instead render the fallback notice (below). Uses the async bootstrap loader so
  `await google.maps.importLibrary("maps"|"marker")` works.
- **Container.** Ensure a `#sensor-map-panel` element exists directly **before**
  `#sensor-list` (create + insert it if missing — the backend fragment does not include
  it). Fixed height (e.g. 320px), full width, rounded to match cards.
- **Sync on swap.** Listen on `document` for `htmx:afterSwap` where `detail.target` is
  (or contains) `#sensor-list`, plus an initial run. On each: read the current `<li>`
  rows, (re)build markers. Idempotent — clears previous markers first.
- **Row parsing.** For each sensor `<li>`: extract
  - `daqid` — from a `data-daq` attribute if present, else the row's visible label text;
  - `href` — the Datatilegnelse link's `href` within the row;
  - `lat`/`lng` — from `data-lat`/`data-lon` if present, else `placeholderCoord(daqid)`.
  Rows without a resolvable href are skipped (nothing to click through to).
- **Markers.** One `google.maps.marker.AdvancedMarkerElement` per sensor. Map created with
  `mapId: import.meta.env.PUBLIC_GOOGLE_MAPS_MAP_ID || "DEMO_MAP_ID"` (Advanced Markers
  require a Map ID). Fit bounds to all markers (single marker → fixed zoom).
  - **Content** is a DOM pin element (a `<div class="sensor-pin">` with a hidden
    `<span class="sensor-pin__label">{daqid}</span>`).
  - **Hover** → show the label. Implemented with CSS `:hover` on the pin content (Advanced
    Markers render content as real DOM, so `:hover` works; no JS listeners needed). Also
    set the pin's `title` as a plain-text fallback.
  - **Click** → `window.location.assign(href)`. Attach a `click` listener to the content
    element (and `gmp-click` on the marker as belt-and-suspenders).
- **Coord helpers** live in a separate **`frontend/src/lib/sensor-map-coords.js`** (pure,
  no DOM / no `import.meta.env`, so `node --test` can cover them; imported by
  `sensor-map.js`):
  - `placeholderCoord(daqid)` — deterministic: hash the `daq_id` string (simple 32-bit
    rolling hash) → two bounded offsets (±~0.0015°, ~150 m) added to a base point
    `BASE = { lat: 55.6761, lng: 12.5683 }` (Copenhagen; a single named constant to swap
    for a real building later). Deterministic so a marker does not jump between HTMX swaps.
  - `resolveCoord({ lat, lon }, daqid)` — returns the explicit `{lat, lng}` when both are
    finite numbers, else `placeholderCoord(daqid)`.

### 2. `frontend/src/pages/node.astro` (edit)

- Add a client `<script>` (module) that imports and initializes `sensor-map.js`. The
  script runs on page load; the module wires its own `htmx:afterSwap` listener so it
  survives the async sensor-list load. No structural HTML change to the tab is required —
  the module injects its own `#sensor-map-panel`.
- Add scoped styles for `.sensor-pin`, `.sensor-pin__label` (hover tooltip), and the
  `#sensor-map-panel` container. Layout via CSS grid where layout is needed.

### 3. `frontend/.env.example` (edit)

Document two new build-time vars (Astro inlines `PUBLIC_*`):

- `PUBLIC_GOOGLE_MAPS_API_KEY` — Maps JavaScript API key. **Blank by default**; blank →
  the panel shows a "Set PUBLIC_GOOGLE_MAPS_API_KEY to enable the map" notice instead of a
  broken map. No key is committed.
- `PUBLIC_GOOGLE_MAPS_MAP_ID` — Map ID required by Advanced Markers. Blank → falls back to
  Google's `DEMO_MAP_ID` (fine for development).

## Error / empty handling

- **No API key:** panel renders a muted notice; no `<script>` injected; no console errors.
- **Maps script fails to load:** catch the loader promise rejection → same muted notice
  with an error line; do not throw.
- **No sensors / empty list:** panel hidden or shows "No sensors to map".
- **Rows missing coords and href:** skipped individually; the rest still plot.

## Testing

The existing frontend runner is Node's built-in test runner: `npm test` →
`node --test src/lib/*.test.js` (no Vitest, no jsdom). So the **pure, DOM-free logic**
lives in its own module and gets the unit tests; DOM/Maps glue is covered by the smoke test.

- **Unit (`node --test`, new `src/lib/sensor-map-coords.test.js`):** the coordinate helpers
  are pure functions in `src/lib/sensor-map-coords.js` (imported by `sensor-map.js`) so they
  test without a DOM. Cases: `placeholderCoord(daqid)` is deterministic (same `daq_id` →
  same coord; different ids → different coords) and stays within the expected bounds around
  `BASE`; the coord resolver prefers explicit `{lat, lon}` when present and falls back to
  the placeholder when absent.
- **Manual / Playwright (smoke):** with a key set, the Data tab shows the map with one
  marker per sensor; hover reveals the `daqid`; click navigates to the `/measurements/…`
  URL. Without a key, the fallback notice shows and the page is otherwise unaffected.

## Out of scope (future)

- A real `lat`/`lon` field on `Sensor` (+ DynamoDB, + backend emit of `data-lat`/`data-lon`).
- Validated addresses / geocoding.
- Marker clustering, per-resource marker styling, map on other tabs/pages.
