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
