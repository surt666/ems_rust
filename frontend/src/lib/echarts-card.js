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
/** Subtle gray for comparison/baseline series (--text-dim). */
export const FAINT = "#aab2ba";

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
    grid: { left: 56, right: 18, top: 28, bottom: zoom ? 64 : 36 },
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
          { type: "slider", height: 18, bottom: 16, borderColor: GRID, textStyle: { color: MUTED } },
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
      itemStyle: { color: s.color ?? PALETTE[i % PALETTE.length] },
      areaStyle: s.areaStyle ? { opacity: 0.12 } : undefined,
    }),
    ),
  }, { notMerge: true });

  return chart;
}

/**
 * Wire an element up once: draw it, keep it sized, and redraw on an optional
 * CustomEvent.
 *
 * The ResizeObserver matters — these charts live in `x-show` tab panels, so the
 * container can go from `display:none` to visible long after init, and a canvas
 * sized at 0 stays blank forever.
 */
export function mountChart(el, options, eventName) {
  const chart = drawChart(el, options);
  if (!chart) return null;

  addEventListener("resize", () => chart.resize());
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
  for (const el of root.querySelectorAll("[data-chart]")) {
    if (el.dataset.chartMounted) continue;
    el.dataset.chartMounted = "1";
    let cfg = {};
    try {
      cfg = JSON.parse(el.querySelector('script[type="application/json"]')?.textContent || "{}");
    } catch {
      /* leave the chart empty rather than break the page */
    }
    const canvas = el.querySelector("[data-chart-canvas]") || el;
    mountChart(canvas, cfg, el.dataset.chartEvent || undefined);
  }
}
