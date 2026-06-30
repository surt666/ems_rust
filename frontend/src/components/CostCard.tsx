import { useEffect, useState } from "react";
import EChartsChart from "./EChartsChart";
import { aggBase, resolveLevelId } from "../lib/agg";

// "Samlet omkostning" — real cost, fed by the aggregations `get_cost` action
// (consumption × per-resource tariff, server-side). Sums all resources per
// bucket into a total cost series + a period total. Self-contained client:only
// island; resolves the node from sessionStorage (same contract as ResourceChart).

interface Row { purpose: string; unit: string; timestamp: string; value: number }

interface Props {
  levelId?: string;
  resolution?: string;
  days?: number;
  height?: number;
}

const dkk = (n: number) =>
  new Intl.NumberFormat("da-DK", { maximumFractionDigits: 0 }).format(n) + " kr.";

export default function CostCard({ levelId, resolution = "daily", days = 30, height = 200 }: Props) {
  const [cats, setCats] = useState<string[]>([]);
  const [data, setData] = useState<number[]>([]);
  const [total, setTotal] = useState(0);
  const [prevTotal, setPrevTotal] = useState<number | null>(null);
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
      const prevStart = new Date(start.getTime() - days * 86400000); // preceding equal window
      const q = (s: Date, e: Date) =>
        new URLSearchParams({ level_id: lvl, resolution, start: s.toISOString(), end: e.toISOString() });
      const sum = (rows: Row[]) => rows.reduce((a, r) => a + r.value, 0);
      try {
        setState("loading");
        // Current + previous window, both via the cheap get_cost (no slow benchmark).
        const [res, prevRes] = await Promise.all([
          fetch(`${base}/meterdata/query/get_cost?${q(start, end)}`),
          fetch(`${base}/meterdata/query/get_cost?${q(prevStart, start)}`),
        ]);
        if (!res.ok) { if (!cancelled) { setState("error"); setMsg(`Fejl: ${res.statusText}`); } return; }
        const rows = (await res.json()) as Row[];
        if (cancelled) return;
        if (!rows.length) { setState("empty"); return; }

        // Sum cost across resources per bucket → one total-cost series.
        const byBucket = new Map<string, number>();
        for (const r of rows) byBucket.set(r.timestamp, (byBucket.get(r.timestamp) || 0) + r.value);
        const sortedTs = Array.from(byBucket.keys()).sort();
        setCats(sortedTs.map((t) => t.replace("T", " ").slice(0, 16)));
        setData(sortedTs.map((t) => Math.round(byBucket.get(t)!)));
        setTotal(Array.from(byBucket.values()).reduce((a, b) => a + b, 0));
        if (prevRes.ok) {
          const prev = sum((await prevRes.json()) as Row[]);
          if (!cancelled && prev > 0) setPrevTotal(prev);
        }
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
    const text = state === "loading" ? "Indlæser…" : state === "empty" ? "Ingen omkostningsdata." : msg;
    return <div style={{ display: "grid", placeItems: "center", minHeight: `${height}px`, color }}>{text}</div>;
  }

  const dev = prevTotal && prevTotal > 0 ? ((total - prevTotal) / prevTotal) * 100 : null;
  const row: React.CSSProperties = { display: "grid", gridAutoFlow: "column", justifyContent: "start", gap: "22px", margin: "4px 0 8px" };
  const cell: React.CSSProperties = { display: "grid", gap: "2px" };
  const lbl: React.CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-muted)" };
  const val: React.CSSProperties = { fontVariantNumeric: "tabular-nums" };
  return (
    <>
      <div style={row}>
        <div style={cell}><span style={lbl}>Periode (seneste {days} dage)</span><strong style={val}>{dkk(total)}</strong></div>
        {prevTotal !== null && <div style={cell}><span style={lbl}>Forrige periode</span><strong style={val}>{dkk(prevTotal)}</strong></div>}
        {dev !== null && (
          <div style={cell}>
            <span style={lbl}>Afvigelse</span>
            <strong style={val} className={dev > 0 ? "nd-neg" : "nd-pos"}>{dev > 0 ? "+" : ""}{dev.toFixed(1)} %</strong>
          </div>
        )}
      </div>
      <EChartsChart categories={cats} series={[{ name: "Omkostning", data, color: "#38bdf8", type: "bar" }]} unit="kr." height={height} zoom={false} />
    </>
  );
}
