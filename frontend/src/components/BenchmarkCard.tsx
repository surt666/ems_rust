import { useEffect, useState } from "react";

// "Bygningsbenchmark" — peer benchmark of the buildings under the selected node's
// company vs the company-average building (cost + CO₂e), fed by the aggregations
// `get_benchmark` action. Horizontal performance bars + above/below counts,
// mirroring the EMS reference. Self-contained client island.

interface BuildingStat { node_path: string; cost_dkk: number; co2e_kg: number; cost_dev_pct: number }
interface Bench {
  building_count: number;
  avg_cost_dkk: number;
  avg_co2e_kg: number;
  node_is_building: boolean;
  node_cost_dev_pct: number;
  node_co2e_dev_pct: number;
  above_count: number;
  above_excess_dkk: number;
  below_count: number;
  below_saving_dkk: number;
  buildings: BuildingStat[];
}

interface Props { levelId?: string; days?: number }

function resolveLevelId(): string {
  const id = sessionStorage.getItem("selectedNodeId") || "";
  const path = sessionStorage.getItem("selectedNodePath") || "";
  const company = sessionStorage.getItem("selectedCompanyId") || "";
  let level = path ? `${path}#${id}` : id;
  if (company && level && !level.includes(company)) level = `${company}#${level}`;
  return level;
}

const da = (n: number, frac = 0) => new Intl.NumberFormat("da-DK", { maximumFractionDigits: frac }).format(n);

// Horizontal green→red performance bar with a marker at p∈0..1.
function PerfBar({ title, p, caption, accent }: { title: string; p: number; caption: string; accent: string }) {
  const pos = Math.max(0, Math.min(1, p));
  return (
    <div style={{ display: "grid", gap: "6px" }}>
      <div style={{ display: "flex", justifyContent: "space-between", fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
        <span>{title}</span>
        <strong style={{ color: accent, fontVariantNumeric: "tabular-nums" }}>{caption}</strong>
      </div>
      <div style={{ position: "relative", height: "12px", borderRadius: "6px", background: "linear-gradient(90deg,#22c55e 0%,#f59e0b 55%,#ef4444 100%)" }}>
        <div style={{ position: "absolute", left: `${pos * 100}%`, top: "-5px", transform: "translateX(-50%)", width: 0, height: 0, borderLeft: "5px solid transparent", borderRight: "5px solid transparent", borderTop: "9px solid var(--text-primary)" }} />
      </div>
    </div>
  );
}

export default function BenchmarkCard({ levelId, days = 30 }: Props) {
  const [b, setB] = useState<Bench | null>(null);
  const [state, setState] = useState<"loading" | "ok" | "empty" | "error">("loading");
  const [msg, setMsg] = useState("");

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      const lvl = levelId || resolveLevelId();
      if (!lvl) { setState("error"); setMsg("Ingen node valgt."); return; }
      const base = import.meta.env.PUBLIC_AGG_API_BASE_URL || "";
      const end = new Date();
      const start = new Date(end.getTime() - days * 86400000);
      const params = new URLSearchParams({ level_id: lvl, resolution: "daily", start: start.toISOString(), end: end.toISOString() });
      try {
        setState("loading");
        const res = await fetch(`${base}/meterdata/query/get_benchmark?${params}`);
        if (!res.ok) { if (!cancelled) { setState("error"); setMsg(`Fejl: ${res.statusText}`); } return; }
        const data = (await res.json()) as Bench;
        if (cancelled) return;
        if (!data.building_count) { setState("empty"); return; }
        setB(data);
        setState("ok");
      } catch (e) {
        if (!cancelled) { setState("error"); setMsg(e instanceof Error ? e.message : "Kunne ikke hente data"); }
      }
    };
    run();
    return () => { cancelled = true; };
  }, [levelId, days]);

  if (state !== "ok" || !b) {
    const color = state === "error" ? "#dc2626" : "#6b7280";
    const text = state === "loading" ? "Indlæser…" : state === "empty" ? "Ingen benchmarkdata." : msg;
    return <div style={{ display: "grid", placeItems: "center", minHeight: "120px", color }}>{text}</div>;
  }

  // For a building selection the bar marks its own deviation; for a company/property
  // it marks the share of buildings above the average (more above = worse).
  const co2eAbove = b.buildings.filter((x) => x.co2e_kg > b.avg_co2e_kg).length;
  const costP = b.node_is_building ? 0.5 + b.node_cost_dev_pct / 100 : b.above_count / b.building_count;
  const co2eP = b.node_is_building ? 0.5 + b.node_co2e_dev_pct / 100 : co2eAbove / b.building_count;
  const costCap = b.node_is_building ? `${b.node_cost_dev_pct > 0 ? "+" : ""}${b.node_cost_dev_pct.toFixed(1)} %` : `${b.above_count}/${b.building_count} over gnm.`;
  const co2eCap = b.node_is_building ? `${b.node_co2e_dev_pct > 0 ? "+" : ""}${b.node_co2e_dev_pct.toFixed(1)} %` : `${co2eAbove}/${b.building_count} over gnm.`;
  const accent = (p: number) => (p > 0.5 ? "var(--danger)" : "var(--success)");

  const metric = (label: string, value: string, sub: string, cls?: string) => (
    <div style={{ display: "grid", gap: "2px" }}>
      <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>{label}</span>
      <strong style={{ fontVariantNumeric: "tabular-nums" }} className={cls}>{value}</strong>
      <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>{sub}</span>
    </div>
  );

  return (
    <>
      <div style={{ display: "grid", gap: "14px", margin: "4px 0 12px" }}>
        <PerfBar title="Omkostning ift. gennemsnit" p={costP} caption={costCap} accent={accent(costP)} />
        <PerfBar title="CO₂e ift. gennemsnit" p={co2eP} caption={co2eCap} accent={accent(co2eP)} />
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: "10px" }}>
        {metric("Alle bygninger", `${b.building_count}`, `gnm. ${da(b.avg_cost_dkk)} kr.`)}
        {metric("Under gnm.", `${b.below_count}`, `−${da(b.below_saving_dkk)} kr.`, "nd-pos")}
        {metric("Over gnm.", `${b.above_count}`, `+${da(b.above_excess_dkk)} kr.`, "nd-neg")}
      </div>
    </>
  );
}
