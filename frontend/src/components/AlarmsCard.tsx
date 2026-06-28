import { useEffect, useState } from "react";

// "Alarmer" — real derived consumption-spike alarms, fed by the aggregations
// `get_alarms` action (buckets above spike_factor × the resource's median).
// Self-contained client:only island; resolves the node from sessionStorage.

interface Alarm { resource: string; timestamp: string; value: number; median: number; ratio: number; unit: string }
interface AlarmsResp { count: number; alarms: Alarm[] }

interface Props { levelId?: string; days?: number }

const RESOURCE_LABELS: Record<string, string> = {
  electricity: "El", district_heating: "Fjernvarme", district_cooling: "Fjernkøling",
  gas: "Gas", water: "Vand", heat: "Varme",
};

function resolveLevelId(): string {
  const id = sessionStorage.getItem("selectedNodeId") || "";
  const path = sessionStorage.getItem("selectedNodePath") || "";
  const company = sessionStorage.getItem("selectedCompanyId") || "";
  let level = path ? `${path}#${id}` : id;
  if (company && level && !level.includes(company)) level = `${company}#${level}`;
  return level;
}

const da = (n: number) => new Intl.NumberFormat("da-DK", { maximumFractionDigits: 1 }).format(n);

export default function AlarmsCard({ levelId, days = 30 }: Props) {
  const [resp, setResp] = useState<AlarmsResp | null>(null);
  const [state, setState] = useState<"loading" | "ok" | "error">("loading");
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
        const res = await fetch(`${base}/meterdata/query/get_alarms?${params}`);
        if (!res.ok) { if (!cancelled) { setState("error"); setMsg(`Fejl: ${res.statusText}`); } return; }
        const data = (await res.json()) as AlarmsResp;
        if (!cancelled) { setResp(data); setState("ok"); }
      } catch (e) {
        if (!cancelled) { setState("error"); setMsg(e instanceof Error ? e.message : "Kunne ikke hente data"); }
      }
    };
    run();
    return () => { cancelled = true; };
  }, [levelId, days]);

  if (state === "error") {
    return <div style={{ display: "grid", placeItems: "center", minHeight: "80px", color: "#dc2626" }}>{msg}</div>;
  }
  if (state === "loading" || !resp) {
    return <div style={{ display: "grid", placeItems: "center", minHeight: "80px", color: "#6b7280" }}>Indlæser…</div>;
  }

  const top = resp.alarms.slice(0, 5);
  const counts: React.CSSProperties = { display: "grid", gridAutoFlow: "column", justifyContent: "start", gap: "28px" };
  const count: React.CSSProperties = { display: "grid", gap: "2px" };
  const num: React.CSSProperties = { fontSize: "var(--text-2xl)", fontWeight: 700, color: "var(--text-primary)", fontVariantNumeric: "tabular-nums" };
  return (
    <>
      <div style={counts}>
        <div style={count}><span style={num}>{resp.count}</span><span className="muted">Forbrugsspidser</span></div>
        <div style={count}><span style={num}>{resp.alarms.length ? top.length : 0}</span><span className="muted">Vises</span></div>
      </div>
      {resp.count === 0 ? (
        <p className="muted" style={{ margin: "8px 0 0" }}>Ingen forbrugsspidser i perioden (seneste {days} dage).</p>
      ) : (
        <table className="data-table" style={{ marginTop: 8 }}>
          <tbody>
            {top.map((a, i) => (
              <tr key={i}>
                <td>{RESOURCE_LABELS[a.resource] ?? a.resource}</td>
                <td className="muted">{a.timestamp.slice(0, 10)}</td>
                <td style={{ textAlign: "right" }} className="nd-neg">
                  {da(a.value)} {a.unit} (×{a.ratio.toFixed(1)})
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}
