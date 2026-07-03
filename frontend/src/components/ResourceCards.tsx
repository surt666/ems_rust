import { useEffect, useState } from "react";
import EChartsChart from "./EChartsChart";
import { RESOURCE_LABELS, aggBase, resolveLevelId } from "../lib/agg";

// Per-resource consumption cards (Varme / Vand / El …), fed by the live
// aggregations API — the real replacement for the mock `energyCards` block in
// NodeDashboard. One card per resource present under the selected node: period
// total + daily average + a daily-consumption bar trend. Self-contained
// client:only island; resolves the node from sessionStorage (same contract as
// ResourceChart / AggregationChartWrapper), no page JS.

interface Row {
  purpose: string; // the per-meter resource (electricity, water, district_heating, …)
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
}

// Maps to the existing `badge energy-*` classes in the theme CSS.
const RESOURCE_BADGE: Record<string, string> = {
  electricity: "el",
  district_heating: "varme",
  district_cooling: "cooling",
  gas: "gas",
  water: "vand",
  heat: "varme",
};
const RESOURCE_COLOR: Record<string, string> = {
  electricity: "#facc15",
  district_heating: "#f97316",
  district_cooling: "#38bdf8",
  gas: "#a855f7",
  water: "#38bdf8",
  heat: "#f97316",
};

const da = (n: number, frac: number) =>
  new Intl.NumberFormat("da-DK", { maximumFractionDigits: frac }).format(n);

// Inline styles — NodeDashboard's `.nd-*` CSS is Astro-scoped and doesn't reach
// this client island, so the totals row is styled here.
const totalsRow: React.CSSProperties = { display: "grid", gridAutoFlow: "column", justifyContent: "start", gap: "22px", margin: "4px 0 8px" };
const totalCell: React.CSSProperties = { display: "grid", gap: "2px" };
const totalLabel: React.CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-muted)" };
const totalVal: React.CSSProperties = { fontVariantNumeric: "tabular-nums" };

interface Card {
  resource: string;
  label: string;
  badge: string;
  color: string;
  unit: string;
  total: number;
  avg: number;
  cats: string[];
  data: number[];
}

export default function ResourceCards({ levelId, resolution = "daily", days = 30 }: Props) {
  const [cards, setCards] = useState<Card[]>([]);
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
      const base = aggBase();
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

        // Group rows per resource, building a sorted daily series + totals.
        const byResource = new Map<string, Row[]>();
        for (const r of rows) {
          if (!byResource.has(r.purpose)) byResource.set(r.purpose, []);
          byResource.get(r.purpose)!.push(r);
        }
        const built: Card[] = Array.from(byResource.entries()).map(([resource, rs]) => {
          rs.sort((a, b) => a.timestamp.localeCompare(b.timestamp));
          const total = rs.reduce((s, r) => s + r.value, 0);
          const unit = rs.find((r) => r.unit)?.unit || "";
          return {
            resource,
            label: RESOURCE_LABELS[resource] ?? resource,
            badge: RESOURCE_BADGE[resource] ?? "el",
            color: RESOURCE_COLOR[resource] ?? "#1f9e8f",
            unit,
            total,
            avg: rs.length ? total / rs.length : 0,
            cats: rs.map((r) => r.timestamp.slice(0, 10)),
            data: rs.map((r) => Math.round(r.value * 1000) / 1000),
          };
        });
        // Stable, human order: energy carriers first, then volumes.
        const order = ["electricity", "district_heating", "heat", "gas", "district_cooling", "water"];
        built.sort((a, b) => order.indexOf(a.resource) - order.indexOf(b.resource));
        setCards(built);
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
    const text = state === "loading" ? "Indlæser…" : state === "empty" ? "Ingen forbrugsdata for perioden." : msg;
    return <div style={{ display: "grid", placeItems: "center", minHeight: "120px", color }}>{text}</div>;
  }

  return (
    <>
      {cards.map((c) => {
        const frac = c.total < 100 ? 2 : 0;
        return (
          <div className="card nd-energy" key={c.resource}>
            <div className="section-header">
              <h2 className="section-title" style={{ marginBottom: 0 }}>
                {c.label} <span className={`badge energy-${c.badge}`}>{c.label}</span>
              </h2>
              <span className="chart-kpi__val" style={{ marginLeft: "auto" }}>
                {da(c.total, frac)} {c.unit}
              </span>
            </div>
            <div style={totalsRow}>
              <div style={totalCell}>
                <span style={totalLabel}>Periode (seneste {days} dage)</span>
                <strong style={totalVal}>{da(c.total, frac)} {c.unit}</strong>
              </div>
              <div style={totalCell}>
                <span style={totalLabel}>Dagligt gennemsnit</span>
                <strong style={totalVal}>{da(c.avg, frac)} {c.unit}</strong>
              </div>
            </div>
            <EChartsChart
              categories={c.cats}
              series={[{ name: c.label, data: c.data, color: c.color, type: "bar" }]}
              unit={c.unit}
              height={180}
              zoom={false}
            />
          </div>
        );
      })}
    </>
  );
}
