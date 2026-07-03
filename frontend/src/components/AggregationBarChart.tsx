import { useEffect, useMemo, useState } from "react";
import EChartsChart, { type EChartsSeries } from "./EChartsChart";
import {
  MEASURES, apiResolution, rollup, RESOURCE_COLORS, RESOURCE_LABELS,
  resolveLevelId, aggBase, type Granularity, type Measure,
} from "../lib/agg";

// Resource Insights drill-down (Apache ECharts, bar). URL-param driven so the
// dashboard cards can deep-link into it; the top controls rewrite the URL and
// re-fetch. All chart JS lives in this island (no page-level script).

const RESOLUTIONS: { key: Granularity; label: string }[] = [
  { key: "yearly", label: "År" }, { key: "monthly", label: "Måned" },
  { key: "weekly", label: "Uge" }, { key: "daily", label: "Dag" }, { key: "hourly", label: "Time" },
];

interface Row { purpose: string; unit: string; timestamp: string; value: number }

function readParams() {
  const p = new URLSearchParams(typeof location !== "undefined" ? location.search : "");
  const now = new Date();
  const y = now.getFullYear();
  return {
    measure: (p.get("measure") as Measure) || "consumption",
    resource: p.get("resource") || "",
    from: p.get("from") || `${y}-01-01`,
    to: p.get("to") || `${y}-12-31`,
    resolution: (p.get("resolution") as Granularity) || "monthly",
  };
}

function writeParams(next: Record<string, string>) {
  const p = new URLSearchParams(location.search);
  for (const [k, v] of Object.entries(next)) v ? p.set(k, v) : p.delete(k);
  history.replaceState(null, "", `${location.pathname}?${p.toString()}`);
}

export default function AggregationBarChart() {
  const [{ measure, resource, from, to, resolution }, setState] = useState(readParams);
  const [rows, setRows] = useState<Row[]>([]);
  const [selected, setSelected] = useState<string[]>(resource ? [resource] : []);
  const [status, setStatus] = useState<"loading" | "ok" | "empty" | "error">("loading");
  const [msg, setMsg] = useState("");

  const measureDef = MEASURES[measure];

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      const lvl = resolveLevelId();
      if (!lvl) { setStatus("error"); setMsg("Ingen node valgt."); return; }
      const params = new URLSearchParams({
        level_id: lvl, resolution: apiResolution(resolution),
        start: new Date(from).toISOString(), end: new Date(to + "T23:59:59Z").toISOString(),
      });
      if (measure === "consumption" && resource) params.set("resource", resource);
      try {
        setStatus("loading");
        const res = await fetch(`${aggBase()}/meterdata/query/${measureDef.action}?${params.toString()}`);
        if (!res.ok) { if (!cancelled) { setStatus("error"); setMsg(`Fejl: ${res.statusText}`); } return; }
        const data = (await res.json()) as Row[];
        if (cancelled) return;
        setRows(data);
        setStatus(data.length ? "ok" : "empty");
        // reconcile selected on every fetch — keep still-present picks, otherwise default
        if (data.length) {
          const present = Array.from(new Set(data.map((r) => r.purpose)));
          setSelected((prev) => {
            const kept = prev.filter((r) => present.includes(r));
            if (kept.length) return kept;
            return resource && present.includes(resource) ? [resource] : present.slice(0, 1);
          });
        }
      } catch (e) {
        if (!cancelled) { setStatus("error"); setMsg(e instanceof Error ? e.message : "Kunne ikke hente data"); }
      }
    };
    run();
    return () => { cancelled = true; };
  }, [measure, resource, from, to, resolution]);

  const present = useMemo(() => Array.from(new Set(rows.map((r) => r.purpose))), [rows]);
  const active = selected.length ? selected : present;

  const { categories, series, unit } = useMemo(() => {
    const filtered = rows.filter((r) => active.includes(r.purpose));
    const { categories, byResource, unitByResource } = rollup(filtered, resolution);
    const series: EChartsSeries[] = Array.from(byResource.entries()).map(([res, data]) => ({
      name: RESOURCE_LABELS[res] ?? res,
      data: data.map((v) => v * measureDef.scale),
      color: RESOURCE_COLORS[res],
      type: "bar",
    }));
    const unit = measureDef.unit || unitByResource.get(active[0]) || "";
    return { categories, series, unit };
  }, [rows, active, resolution, measure]);

  const update = (patch: Partial<ReturnType<typeof readParams>>) => {
    const next = { measure, resource, from, to, resolution, ...patch };
    writeParams(next as any);
    setState(next);
  };

  const toggleResource = (res: string) => {
    setSelected((s) => (s.includes(res) ? s.filter((x) => x !== res) : [...s, res]));
  };

  return (
    <div>
      <div className="ri-filterbar">
        <div className="ri-daterange">
          <input type="date" className="form-input" value={from} onChange={(e) => update({ from: (e.target as HTMLInputElement).value })} />
          <span>–</span>
          <input type="date" className="form-input" value={to} onChange={(e) => update({ to: (e.target as HTMLInputElement).value })} />
        </div>
        <div className="ri-resolutions">
          {RESOLUTIONS.map((r) => (
            <button key={r.key} className={`ri-res-btn${r.key === resolution ? " active" : ""}`} onClick={() => update({ resolution: r.key })}>{r.label}</button>
          ))}
        </div>
        <div className="ri-resources">
          {present.map((res) => (
            <button key={res} className={`ri-res-chip${active.includes(res) ? " active" : ""}`}
              style={{ borderColor: RESOURCE_COLORS[res] ?? "#ccc" }} onClick={() => toggleResource(res)}>
              {RESOURCE_LABELS[res] ?? res}
            </button>
          ))}
        </div>
      </div>
      {status !== "ok" ? (
        <div style={{ display: "grid", placeItems: "center", height: "420px", color: status === "error" ? "#dc2626" : "#6b7280" }}>
          {status === "loading" ? "Indlæser…" : status === "empty" ? "Ingen data for perioden." : msg}
        </div>
      ) : (
        <EChartsChart categories={categories} series={series} unit={unit} height={440} zoom={true} />
      )}
    </div>
  );
}
