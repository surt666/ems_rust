/**
 * Shared ECharts renderer — the plain-JS replacement for the EChartsChart /
 * EChartsChartWrapper React pair.
 *
 * A canvas chart needs data, not markup, so these stay client-rendered; what goes
 * away is the framework around them. Everything else on the dashboard is a
 * server-rendered fragment (see BenchmarkCard.astro / PurposeSplit.astro).
 *
 * Palette and axis styling are the Enity light theme, matching enity-theme.css.
 */
import * as echarts from "echarts";

export const PALETTE = ["#f5841f", "#1f9e8f", "#46b97c", "#a855f7", "#facc15", "#ef4444"];
const TEXT = "#28333d";
const MUTED = "#5a6671";
const GRID = "#e5e8eb";
/** Per-energy-type series colours. The one place a carrier's colour is decided —
 *  the lambda sends `key`, not a colour, so the two cannot drift apart. */
export const RESOURCE_COLORS = {
  electricity: "#f5841f",
  district_heating: "#ef4444",
  district_cooling: "#38bdf8",
  gas: "#a855f7",
  water: "#1f9e8f",
  heat: "#facc15",
};

/**
 * Draw (or redraw) a chart into `el`.
 *
 * Reuses the existing instance when there is one, so repeated calls — a filter
 * change, an `ems:refresh`, a soft navigation — update in place instead of
 * leaking canvases.
 */
export function drawChart(el, { categories = [], series = [], unit = "kWh", stacked = false, zoom = true } = {}) {
  if (!el) return null;
  const chart = echarts.getInstanceByDom(el) || echarts.init(el, undefined, { renderer: "canvas" });

  chart.setOption({
    backgroundColor: "transparent",
    textStyle: { color: TEXT, fontSize: 11 },
    // Margins are what is left for axis labels, legend and the zoom slider — every
    // pixel here comes out of the plot, so they are kept tight.
    grid: { left: 48, right: 12, top: 24, bottom: zoom ? 46 : 26 },
    tooltip: { trigger: "axis", backgroundColor: "#ffffff", borderColor: GRID, textStyle: { color: TEXT } },
    legend: { data: series.map((s) => s.name), textStyle: { color: MUTED }, top: 0, right: 0 },
    xAxis: {
      type: "category",
      data: categories,
      axisLine: { lineStyle: { color: GRID } },
      axisLabel: { color: MUTED },
    },
    yAxis: {
      type: "value",
      name: unit,
      nameTextStyle: { color: MUTED },
      splitLine: { lineStyle: { color: GRID } },
      axisLabel: { color: MUTED },
    },
    // Drag-select / scroll inside plus a slider handle; skipped for small category charts.
    dataZoom: zoom
      ? [
          { type: "inside", throttle: 50 },
          { type: "slider", height: 14, bottom: 6, borderColor: GRID, textStyle: { color: MUTED } },
        ]
      : [],
    series: series.map((s, i) => ({
      name: s.name,
      type: s.type ?? "line",
      stack: stacked ? "total" : undefined,
      smooth: (s.type ?? "line") === "line",
      symbol: "circle",
      symbolSize: 5,
      data: s.data,
      itemStyle: { color: s.color ?? RESOURCE_COLORS[s.key] ?? PALETTE[i % PALETTE.length] },
      areaStyle: s.areaStyle ? { opacity: 0.12 } : undefined,
    }),
    ),
  }, { notMerge: true });

  return chart;
}

/** Live chart instances, so one window listener can size them all. */
const mounted = new Set();

// A single resize listener for the page. Registering one per chart meant a
// dashboard (up to ~9 charts) added nine, none of which were ever removed —
// every htmx re-swap mounted a fresh set on top of the old.
addEventListener("resize", () => {
  for (const c of mounted) {
    if (c.getDom()?.isConnected) c.resize();
    else mounted.delete(c);
  }
});

/**
 * Wire an element up: draw it, keep it sized, and redraw on an optional
 * CustomEvent.
 *
 * The ResizeObserver matters — these charts live in `x-show` tab panels, so the
 * container can go from `display:none` to visible long after init, and a canvas
 * sized at 0 stays blank forever.
 */
export function mountChart(el, options, eventName) {
  const chart = drawChart(el, options);
  if (!chart) return null;

  mounted.add(chart);
  new ResizeObserver(() => chart.resize()).observe(el);

  if (eventName) {
    addEventListener(eventName, (e) => drawChart(el, { ...options, ...(e.detail || {}) }));
  }
  return chart;
}

/**
 * Mount every `[data-chart]` on the page from its inline JSON config.
 *
 * Lets an .astro page declare a chart as markup + data and keep its own script
 * block empty. Idempotent, so it can run on every `astro:page-load`.
 */
export function mountDeclaredCharts(root = document) {
  // The root itself can be the swapped-in chart, not just an ancestor of one.
  const blocks = root.querySelectorAll?.("[data-chart]") ?? [];
  const all = root.matches?.("[data-chart]") ? [root, ...blocks] : [...blocks];
  for (const el of all) {
    if (el.dataset.chartMounted) continue;
    el.dataset.chartMounted = "1";
    let cfg = {};
    try {
      cfg = JSON.parse(el.querySelector('script[type="application/json"]')?.textContent || "{}");
    } catch {
      /* leave the chart empty rather than break the page */
    }
    const canvas = el.querySelector("[data-chart-canvas]") || el;
    // htmx swaps replace the element, so a stale instance can still own this DOM
    // node's canvas. Dispose it rather than leaking it plus its ResizeObserver.
    echarts.getInstanceByDom(canvas)?.dispose();
    mountChart(canvas, cfg, el.dataset.chartEvent || undefined);
  }
}
