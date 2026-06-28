import { useEffect, useState } from "react";
import EChartsChart from "./EChartsChart";

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

function resolveLevelId(): string {
  const id = sessionStorage.getItem("selectedNodeId") || "";
  const path = sessionStorage.getItem("selectedNodePath") || "";
  const company = sessionStorage.getItem("selectedCompanyId") || "";
  let level = path ? `${path}#${id}` : id;
  if (company && level && !level.includes(company)) level = `${company}#${level}`;
  return level;
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
      const base = import.meta.env.PUBLIC_AGG_API_BASE_URL || "";
      const end = new Date();
      const start = new Date(end.getTime() - days * 86400000);
      const q = (s: Date, e: Date) =>
        new URLSearchParams({ level_id: lvl, resolution, start: s.toISOString(), end: e.toISOString() });
      try {
        setState("loading");
        const [res, bench] = await Promise.all([
          fetch(`${base}/meterdata/query/get_cost?${q(start, end)}`),
          fetch(`${base}/meterdata/query/get_benchmark?${q(start, end)}`),
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
        if (bench.ok) {
          const b = await bench.json();
          if (!cancelled && typeof b.cost_prev_dkk === "number") setPrevTotal(b.cost_prev_dkk);
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
  return (
    <>
      <div className="nd-energy__totals">
        <div><span>Periode (seneste {days} dage)</span><strong>{dkk(total)}</strong></div>
        {prevTotal !== null && <div><span>Forrige periode</span><strong>{dkk(prevTotal)}</strong></div>}
        {dev !== null && (
          <div>
            <span>Afvigelse</span>
            <strong className={dev > 0 ? "nd-neg" : "nd-pos"}>{dev > 0 ? "+" : ""}{dev.toFixed(1)} %</strong>
          </div>
        )}
      </div>
      <EChartsChart categories={cats} series={[{ name: "Omkostning", data, color: "#38bdf8", type: "bar" }]} unit="kr." height={height} zoom={false} />
    </>
  );
}
