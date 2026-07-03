import { useEffect, useMemo, useState } from "react";
import EChartsChart, { type EChartsSeries } from "./EChartsChart";
import {
  MEASURES, apiResolution, rollup, RESOURCE_LABELS, resolveLevelId, aggBase,
  type Measure,
} from "../lib/agg";

// One dashboard consumption card (Apache ECharts, monthly bars, this year vs last
// year). Measure + year dropdowns; clicking the chart drills into Resource
// Insights (/rimain) with this card's resource + measure + year.

interface Row { purpose: string; unit: string; timestamp: string; value: number }
interface Props {
  /** A resource token (electricity/water/…) or "combined" (all resources). */
  resource: string;
  title: string;
  height?: number;
}

const yearRange = (): number[] => {
  const y = new Date().getFullYear();
  return [y, y - 1, y - 2, y - 3, y - 4];
};

async function fetchYear(action: string, lvl: string, year: number, resource: string, filterResource: boolean) {
  const params = new URLSearchParams({
    level_id: lvl, resolution: "daily",
    start: `${year}-01-01T00:00:00.000Z`, end: `${year}-12-31T23:59:59.000Z`,
  });
  if (filterResource) params.set("resource", resource);
  const res = await fetch(`${aggBase()}/meterdata/query/${action}?${params.toString()}`);
  if (!res.ok) throw new Error(res.statusText);
  return (await res.json()) as Row[];
}

export default function ConsumptionCard({ resource, title, height = 220 }: Props) {
  const [measure, setMeasure] = useState<Measure>(resource === "combined" ? "cost" : "consumption");
  const [year, setYear] = useState<number>(new Date().getFullYear());
  const [cur, setCur] = useState<Row[]>([]);
  const [prev, setPrev] = useState<Row[]>([]);
  const [status, setStatus] = useState<"loading" | "ok" | "empty" | "error">("loading");

  const md = MEASURES[measure];
  const isCombined = resource === "combined";
  const filterResource = measure === "consumption" && !isCombined;

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      const lvl = resolveLevelId();
      if (!lvl) { setStatus("error"); return; }
      try {
        setStatus("loading");
        const [c, p] = await Promise.all([
          fetchYear(md.action, lvl, year, resource, filterResource),
          fetchYear(md.action, lvl, year - 1, resource, filterResource),
        ]);
        if (cancelled) return;
        const keep = (r: Row) => isCombined || r.purpose === resource || filterResource;
        setCur(c.filter(keep)); setPrev(p.filter(keep));
        setStatus(c.length ? "ok" : "empty");
      } catch { if (!cancelled) setStatus("error"); }
    };
    run();
    return () => { cancelled = true; };
  }, [measure, year, resource]);

  const { categories, series, unit } = useMemo(() => {
    const months = ["jan", "feb", "mar", "apr", "maj", "jun", "jul", "aug", "sep", "okt", "nov", "dec"];
    const sum = (rows: Row[]) => {
      const out = new Array(12).fill(0);
      for (const r of rows) out[new Date(r.timestamp).getUTCMonth()] += r.value * md.scale;
      return out;
    };
    const series: EChartsSeries[] = [
      { name: String(year), data: sum(cur), color: "#f5841f", type: "bar" },
      { name: String(year - 1), data: sum(prev), color: "#aab2ba", type: "bar" },
    ];
    const unit = md.unit || cur[0]?.unit || "";
    return { categories: months, series, unit };
  }, [cur, prev, measure, year]);

  const drill = () => {
    const p = new URLSearchParams({
      measure, from: `${year}-01-01`, to: `${year}-12-31`, resolution: "monthly",
    });
    if (!isCombined) p.set("resource", resource);
    const url = `/rimain?${p.toString()}`;
    if (window.emsNavigate) window.emsNavigate(url); else location.href = url;
  };

  return (
    <div>
      <div className="cc-controls">
        <select className="form-select cc-select" value={measure} onChange={(e) => setMeasure((e.target as HTMLSelectElement).value as Measure)}>
          <option value="consumption">Forbrug</option>
          <option value="cost">Omkostning</option>
          <option value="co2e">CO₂e</option>
        </select>
        <select className="form-select cc-select" value={year} onChange={(e) => setYear(Number((e.target as HTMLSelectElement).value))}>
          {yearRange().map((y) => <option value={y}>{y}</option>)}
        </select>
      </div>
      {status !== "ok" ? (
        <div style={{ display: "grid", placeItems: "center", height: `${height}px`, color: status === "error" ? "#dc2626" : "#6b7280" }}>
          {status === "loading" ? "Indlæser…" : status === "empty" ? "Ingen data." : "Fejl ved indlæsning."}
        </div>
      ) : (
        <div className="cc-chart" onClick={drill} title="Klik for at se detaljer" style={{ cursor: "pointer" }}>
          <EChartsChart categories={categories} series={series} unit={unit} height={height} zoom={false} />
        </div>
      )}
    </div>
  );
}
