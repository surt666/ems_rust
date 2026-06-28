import { useEffect, useState } from "react";
import EChartsChart, { type EChartsSeries } from "./EChartsChart";

// Live per-resource consumption chart (Apache ECharts), fed by the aggregations
// API. This is the dashboard's ECharts replacement for the rimain Nivo chart:
// one series per resource (El / Vand / Varme …) over the selected window, summed
// per bucket. Self-contained client:only island — resolves the selected node
// from sessionStorage (same contract as AggregationChartWrapper), no page JS.

interface Row {
  purpose: string; // carries the resource (electricity, water, district_heating, …)
  unit: string;
  timestamp: string;
  value: number;
}

interface Props {
  /** Override the node path; otherwise resolved from sessionStorage. */
  levelId?: string;
  /** "hourly" | "daily" — default daily. */
  resolution?: string;
  /** Window length in days back from now — default 30. */
  days?: number;
  height?: number;
}

// Danish resource labels (values match the aggregations lambda's Resource::as_str).
const RESOURCE_LABELS: Record<string, string> = {
  electricity: "El",
  district_heating: "Fjernvarme",
  district_cooling: "Fjernkøling",
  gas: "Gas",
  water: "Vand",
  heat: "Varme",
};

// Same node-path contract as AggregationChartWrapper: the rollup is keyed by the
// full hierarchy path, partitioned by company (HN2), so ensure the company prefix
// is present even when the tree only stored a partial parent path.
function resolveLevelId(): string {
  const id = sessionStorage.getItem("selectedNodeId") || "";
  const path = sessionStorage.getItem("selectedNodePath") || "";
  const company = sessionStorage.getItem("selectedCompanyId") || "";
  let level = path ? `${path}#${id}` : id;
  if (company && level && !level.includes(company)) level = `${company}#${level}`;
  return level;
}

export default function ResourceChart({ levelId, resolution = "daily", days = 30, height = 320 }: Props) {
  const [categories, setCategories] = useState<string[]>([]);
  const [series, setSeries] = useState<EChartsSeries[]>([]);
  const [unit, setUnit] = useState("kWh");
  const [state, setState] = useState<"loading" | "ok" | "empty" | "error">("loading");
  const [msg, setMsg] = useState("");

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      const lvl = levelId || resolveLevelId();
      if (!lvl) {
        setState("error");
        setMsg("Ingen node valgt.");
        return;
      }
      const end = new Date();
      const start = new Date(end.getTime() - days * 24 * 60 * 60 * 1000);
      const params = new URLSearchParams({
        level_id: lvl,
        resolution,
        start: start.toISOString(),
        end: end.toISOString(),
      });
      const base = import.meta.env.PUBLIC_AGG_API_BASE_URL || "";
      try {
        setState("loading");
        const res = await fetch(`${base}/meterdata/query/get_aggregations?${params.toString()}`);
        if (!res.ok) {
          if (!cancelled) {
            setState("error");
            setMsg(`Fejl: ${res.statusText}`);
          }
          return;
        }
        const rows = (await res.json()) as Row[];
        if (cancelled) return;
        if (!rows.length) {
          setState("empty");
          return;
        }

        // One numeric series per resource, aligned to a shared, sorted time axis.
        const cats = Array.from(new Set(rows.map((r) => r.timestamp))).sort();
        const idx = new Map(cats.map((t, i) => [t, i]));
        const byResource = new Map<string, number[]>();
        let firstUnit = "";
        for (const r of rows) {
          if (!byResource.has(r.purpose)) byResource.set(r.purpose, new Array(cats.length).fill(0));
          byResource.get(r.purpose)![idx.get(r.timestamp)!] = r.value;
          if (!firstUnit && r.unit) firstUnit = r.unit;
        }

        setCategories(cats.map((t) => t.replace("T", " ").slice(0, 16)));
        setSeries(
          Array.from(byResource.entries()).map(([resName, data]) => ({
            name: RESOURCE_LABELS[resName] ?? resName,
            data,
            type: "line",
          })),
        );
        setUnit(firstUnit || "kWh");
        setState("ok");
      } catch (e) {
        if (!cancelled) {
          setState("error");
          setMsg(e instanceof Error ? e.message : "Kunne ikke hente data");
        }
      }
    };
    run();
    return () => {
      cancelled = true;
    };
  }, [levelId, resolution, days]);

  if (state !== "ok") {
    const color = state === "error" ? "#dc2626" : "#6b7280";
    const text = state === "loading" ? "Indlæser…" : state === "empty" ? "Ingen data for perioden." : msg;
    return <div style={{ display: "grid", placeItems: "center", height: `${height}px`, color }}>{text}</div>;
  }

  return <EChartsChart categories={categories} series={series} unit={unit} height={height} zoom={true} />;
}
