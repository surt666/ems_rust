import { useEffect, useState } from "react";

// "Bygningsbenchmark" — real performance vs the node's own preceding period,
// fed by the aggregations `get_benchmark` action. Two semicircle gauges (energy
// and cost deviation): needle left = used less than before (good), right = more.
// Self-contained client:only island; resolves the node from sessionStorage.

interface Bench {
  energy_kwh: number;
  energy_prev_kwh: number;
  energy_deviation_pct: number;
  cost_dkk: number;
  cost_prev_dkk: number;
  cost_deviation_pct: number;
  period_days: number;
}

interface Props {
  levelId?: string;
  days?: number;
}

function resolveLevelId(): string {
  const id = sessionStorage.getItem("selectedNodeId") || "";
  const path = sessionStorage.getItem("selectedNodePath") || "";
  const company = sessionStorage.getItem("selectedCompanyId") || "";
  let level = path ? `${path}#${id}` : id;
  if (company && level && !level.includes(company)) level = `${company}#${level}`;
  return level;
}

// Map a deviation % to a needle position 0..1: -50% → 0 (green), 0 → 0.5, +50% → 1 (red).
const needlePos = (devPct: number) => Math.max(0, Math.min(1, 0.5 + devPct / 100));
const needle = (p: number) => {
  const a = Math.PI * (1 - p);
  return `${(80 + 56 * Math.cos(a)).toFixed(1)},${(80 - 56 * Math.sin(a)).toFixed(1)}`;
};

// Self-contained styles — NodeDashboard's `.nd-*` CSS is Astro-scoped and does
// not reach this client island, so size the gauge here (an unconstrained inline
// SVG would otherwise fill the whole card width).
function Gauge({ label, devPct }: { label: string; devPct: number }) {
  const p = needlePos(devPct);
  const [nx, ny] = needle(p).split(",");
  const sign = devPct > 0 ? "+" : "";
  return (
    <div style={{ display: "grid", justifyItems: "center", gap: "2px" }}>
      <svg viewBox="0 0 160 92" style={{ width: "100%", maxWidth: "104px", height: "auto" }}>
        <path d="M16,80 A64,64 0 0 1 144,80" fill="none" stroke="#22c55e" strokeWidth="12" strokeDasharray="67 201" strokeDashoffset="0" />
        <path d="M16,80 A64,64 0 0 1 144,80" fill="none" stroke="#f97316" strokeWidth="12" strokeDasharray="67 201" strokeDashoffset="-67" />
        <path d="M16,80 A64,64 0 0 1 144,80" fill="none" stroke="#ef4444" strokeWidth="12" strokeDasharray="67 201" strokeDashoffset="-134" />
        <line x1="80" y1="80" x2={nx} y2={ny} stroke="var(--text-primary)" strokeWidth="2.5" />
        <circle cx="80" cy="80" r="4" fill="var(--text-primary)" />
      </svg>
      <div style={{ fontSize: "var(--text-base)", fontWeight: 700, fontVariantNumeric: "tabular-nums", color: devPct > 0 ? "var(--danger)" : "var(--success)" }}>
        {sign}{devPct.toFixed(1)} %
      </div>
      <div style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)", textAlign: "center" }}>{label}</div>
    </div>
  );
}

const da = (n: number, frac = 0) => new Intl.NumberFormat("da-DK", { maximumFractionDigits: frac }).format(n);

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
        if (!data.energy_kwh && !data.cost_dkk && !data.energy_prev_kwh) { setState("empty"); return; }
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

  const metric = (label: string, value: string) => (
    <div style={{ display: "grid", gap: "2px" }}>
      <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>{label}</span>
      <strong style={{ fontVariantNumeric: "tabular-nums" }}>{value}</strong>
    </div>
  );
  return (
    <>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "12px", justifyItems: "center", maxWidth: "320px", margin: "0 auto" }}>
        <Gauge label="Energi vs. forrige periode" devPct={b.energy_deviation_pct} />
        <Gauge label="Omkostning vs. forrige periode" devPct={b.cost_deviation_pct} />
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: "10px", marginTop: "12px" }}>
        {metric("Energi (periode)", `${da(b.energy_kwh, 1)} kWh`)}
        {metric("Forrige", `${da(b.energy_prev_kwh, 1)} kWh`)}
        {metric("Omkostning", `${da(b.cost_dkk)} kr.`)}
      </div>
    </>
  );
}
