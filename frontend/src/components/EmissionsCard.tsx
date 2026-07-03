import { useEffect, useState } from "react";
import EChartsChart from "./EChartsChart";
import { RESOURCE_LABELS, aggBase, resolveLevelId } from "../lib/agg";

// "CO₂e" — real emissions, fed by the aggregations `get_emissions` action
// (consumption × per-resource emission factor, server-side, in kg). Shown in
// tonnes (period total + per-resource breakdown + daily trend), mirroring the
// reference's Forbrugsoverblik CO₂e headline. Self-contained client island.

interface Row { purpose: string; unit: string; timestamp: string; value: number }
interface Props { levelId?: string; resolution?: string; days?: number; height?: number }

const ton = (kg: number) => new Intl.NumberFormat("da-DK", { maximumFractionDigits: 2 }).format(kg / 1000);

export default function EmissionsCard({ levelId, resolution = "daily", days = 30, height = 180 }: Props) {
  const [cats, setCats] = useState<string[]>([]);
  const [data, setData] = useState<number[]>([]);
  const [totalKg, setTotalKg] = useState(0);
  const [byResource, setByResource] = useState<[string, number][]>([]);
  const [state, setState] = useState<"loading" | "ok" | "empty" | "error">("loading");
  const [msg, setMsg] = useState("");

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      const lvl = levelId || resolveLevelId();
      if (!lvl) { setState("error"); setMsg("Ingen node valgt."); return; }
      const base = aggBase();
      const end = new Date();
      const start = new Date(end.getTime() - days * 86400000);
      const params = new URLSearchParams({ level_id: lvl, resolution, start: start.toISOString(), end: end.toISOString() });
      try {
        setState("loading");
        const res = await fetch(`${base}/meterdata/query/get_emissions?${params}`);
        if (!res.ok) { if (!cancelled) { setState("error"); setMsg(`Fejl: ${res.statusText}`); } return; }
        const rows = (await res.json()) as Row[];
        if (cancelled) return;
        if (!rows.length) { setState("empty"); return; }
        const perBucket = new Map<string, number>();
        const perResource = new Map<string, number>();
        for (const r of rows) {
          perBucket.set(r.timestamp, (perBucket.get(r.timestamp) || 0) + r.value);
          perResource.set(r.purpose, (perResource.get(r.purpose) || 0) + r.value);
        }
        const ts = Array.from(perBucket.keys()).sort();
        setCats(ts.map((t) => t.replace("T", " ").slice(0, 16)));
        setData(ts.map((t) => Math.round((perBucket.get(t)! / 1000) * 1000) / 1000));
        setTotalKg(Array.from(perBucket.values()).reduce((a, b) => a + b, 0));
        setByResource(Array.from(perResource.entries()).sort((a, b) => b[1] - a[1]));
        setState("ok");
      } catch (e) {
        if (!cancelled) { setState("error"); setMsg(e instanceof Error ? e.message : "Kunne ikke hente data"); }
      }
    };
    run();
    return () => { cancelled = true; };
  }, [levelId, resolution, days]);

  if (state !== "ok") {
    const color = state === "error" ? "#dc2626" : "#6b7280";
    const text = state === "loading" ? "Indlæser…" : state === "empty" ? "Ingen emissionsdata." : msg;
    return <div style={{ display: "grid", placeItems: "center", minHeight: `${height}px`, color }}>{text}</div>;
  }

  const lbl: React.CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-muted)" };
  const val: React.CSSProperties = { fontVariantNumeric: "tabular-nums" };
  return (
    <>
      <div style={{ display: "grid", gridAutoFlow: "column", justifyContent: "start", gap: "22px", margin: "4px 0 8px", alignItems: "baseline" }}>
        <div style={{ display: "grid", gap: "2px" }}>
          <span style={lbl}>I alt (seneste {days} dage)</span>
          <strong style={{ ...val, fontSize: "var(--text-2xl)", fontWeight: 700 }}>{ton(totalKg)} ton</strong>
        </div>
        {byResource.map(([r, kg]) => (
          <div key={r} style={{ display: "grid", gap: "2px" }}>
            <span style={lbl}>{RESOURCE_LABELS[r] ?? r}</span>
            <strong style={val}>{ton(kg)} ton</strong>
          </div>
        ))}
      </div>
      <EChartsChart categories={cats} series={[{ name: "CO₂e", data, color: "#46b97c", type: "bar" }]} unit="ton" height={height} zoom={true} />
    </>
  );
}
