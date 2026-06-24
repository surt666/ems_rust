import { useEffect, useRef } from "react";
import * as echarts from "echarts";

// Reusable zoomable chart island (Apache ECharts). Kept alongside the Nivo
// islands — use this when you want dataZoom / richer interactions. Data is
// mock/static today; later feed `series`/`categories` from an HTMX-loaded
// fragment or an endpoint via the wrapper. No app JS lives outside this island.

export interface EChartsSeries {
  name: string;
  data: number[];
  color?: string;
  type?: "line" | "bar";
  areaStyle?: boolean;
}

interface Props {
  categories?: string[];
  series?: EChartsSeries[];
  unit?: string;
  height?: number;
  stacked?: boolean;
  zoom?: boolean;
}

// palette aligned to the app's energy/accent tokens
const PALETTE = ["#38bdf8", "#f97316", "#facc15", "#a855f7", "#22c55e", "#ef4444"];
const TEXT = "#c9d3e3";
const MUTED = "#8b97b0";
const GRID = "#1f2a42";

function mockYear(): { categories: string[]; series: EChartsSeries[] } {
  const months = ["Jan", "Feb", "Mar", "Apr", "Maj", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dec"];
  // deterministic-ish synthetic seasonal curve (index-based, no Date/random)
  const cur = months.map((_, i) => Math.round(900 - 700 * Math.cos((i / 11) * Math.PI * 2) + i * 12));
  const prev = months.map((_, i) => Math.round(820 - 640 * Math.cos((i / 11) * Math.PI * 2) + i * 9));
  return {
    categories: months,
    series: [
      { name: "I år", data: cur, color: PALETTE[0], type: "line", areaStyle: true },
      { name: "Sidste år", data: prev, color: MUTED, type: "line" },
    ],
  };
}

export default function EChartsChart({ categories, series, unit = "kWh", height = 320, stacked = false, zoom = true }: Props) {
  const ref = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<echarts.ECharts | null>(null);

  useEffect(() => {
    if (!ref.current) return;
    const chart = echarts.init(ref.current, undefined, { renderer: "canvas" });
    chartRef.current = chart;

    const fallback = mockYear();
    const cats = categories ?? fallback.categories;
    const ser = series ?? fallback.series;

    chart.setOption({
      backgroundColor: "transparent",
      textStyle: { color: TEXT, fontSize: 11 },
      grid: { left: 56, right: 18, top: 28, bottom: zoom ? 64 : 36 },
      tooltip: { trigger: "axis", backgroundColor: "#141d30", borderColor: GRID, textStyle: { color: TEXT } },
      legend: { data: ser.map((s) => s.name), textStyle: { color: MUTED }, top: 0, right: 0 },
      xAxis: {
        type: "category",
        data: cats,
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
      // zoomable: drag-select / scroll inside + a slider handle (skip for small category charts)
      dataZoom: zoom
        ? [
            { type: "inside", throttle: 50 },
            { type: "slider", height: 18, bottom: 16, borderColor: GRID, textStyle: { color: MUTED } },
          ]
        : [],
      series: ser.map((s, i) => ({
        name: s.name,
        type: s.type ?? "line",
        stack: stacked ? "total" : undefined,
        smooth: (s.type ?? "line") === "line",
        symbol: "circle",
        symbolSize: 5,
        data: s.data,
        itemStyle: { color: s.color ?? PALETTE[i % PALETTE.length] },
        areaStyle: s.areaStyle ? { opacity: 0.12 } : undefined,
      })),
    });

    const onResize = () => chart.resize();
    window.addEventListener("resize", onResize);
    // Resize when the container itself changes size — e.g. when a hidden tab
    // panel (x-show / display:none) becomes visible. Avoids 0-width charts.
    const ro = new ResizeObserver(() => chart.resize());
    ro.observe(ref.current);
    return () => {
      window.removeEventListener("resize", onResize);
      ro.disconnect();
      chart.dispose();
      chartRef.current = null;
    };
  }, [categories, series, unit, stacked, zoom]);

  return <div ref={ref} style={{ width: "100%", height: `${height}px` }} />;
}
