// Shared helpers for the aggregations-API islands (AggregationBarChart, ResourceChart,
// ResourceCards, CostCard, EmissionsCard). Each island used to copy these verbatim; keep
// them here so the rollup-key contract and the resource labels stay in one place.

/** Base URL of the aggregations HTTP API (empty during local/dev). */
export const aggBase = (): string => import.meta.env.PUBLIC_AGG_API_BASE_URL || "";

/** Danish labels for a resource (values match the aggregations `Resource::as_str`). */
export const RESOURCE_LABELS: Record<string, string> = {
  electricity: "El",
  district_heating: "Fjernvarme",
  district_cooling: "Fjernkøling",
  gas: "Gas",
  water: "Vand",
  heat: "Varme",
};

/**
 * The selected node's full hierarchy path, resolved from sessionStorage. The
 * rollup is keyed by the full path and partitioned by company (HN2), so ensure
 * the company segment is present even when the tree only stored a partial parent
 * path. The same contract every dashboard island (AggregationBarChart and the
 * dashboard cards) relies on.
 */
export function resolveLevelId(): string {
  const id = sessionStorage.getItem("selectedNodeId") || "";
  const path = sessionStorage.getItem("selectedNodePath") || "";
  const company = sessionStorage.getItem("selectedCompanyId") || "";
  let level = path ? `${path}#${id}` : id;
  if (company && level && !level.includes(company)) level = `${company}#${level}`;
  return level;
}

export type Granularity = "hourly" | "daily" | "weekly" | "monthly" | "yearly";
export type Measure = "consumption" | "cost" | "co2e";

/** Measure → aggregations query action + display metadata. `scale` multiplies the
 *  summed value (CO₂e is returned in kg → tonnes). Consumption unit is per-resource. */
export const MEASURES: Record<Measure, { action: string; unit: string; label: string; scale: number }> = {
  consumption: { action: "get_aggregations", unit: "", label: "Forbrug", scale: 1 },
  cost:        { action: "get_cost",         unit: "kr.", label: "Omkostning", scale: 1 },
  co2e:        { action: "get_emissions",    unit: "t CO₂e", label: "CO₂e", scale: 0.001 },
};

/** The API only rolls up hourly or daily; week/month/year are summed client-side from daily. */
export const apiResolution = (g: Granularity): "hourly" | "daily" => (g === "hourly" ? "hourly" : "daily");

/** Per-resource series colours (aligned with enity-theme tokens). */
export const RESOURCE_COLORS: Record<string, string> = {
  electricity: "#46b97c",
  district_heating: "#f5841f",
  district_cooling: "#1f9e8f",
  gas: "#a855f7",
  water: "#3b82f6",
  heat: "#f5841f",
};

export interface AggRow { energy_type: string; purpose?: string; unit: string; timestamp: string; value: number }

/** Bucket key for a timestamp at a granularity. Timestamps are UTC ISO strings. */
export function bucketKey(ts: string, g: Granularity): string {
  if (g === "hourly") return ts.slice(0, 13); // YYYY-MM-DDTHH
  if (g === "daily") return ts.slice(0, 10); // YYYY-MM-DD
  if (g === "monthly") return ts.slice(0, 7); // YYYY-MM
  if (g === "yearly") return ts.slice(0, 4); // YYYY
  // weekly → ISO-8601 week: YYYY-Www
  const d = new Date(/[zZ]|[+-]\d\d:?\d\d$/.test(ts) ? ts : ts + "Z");
  const day = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()));
  const dayNum = (day.getUTCDay() + 6) % 7; // Mon=0
  day.setUTCDate(day.getUTCDate() - dayNum + 3); // nearest Thursday
  const firstThursday = new Date(Date.UTC(day.getUTCFullYear(), 0, 4));
  const week = 1 + Math.round((day.getTime() - firstThursday.getTime()) / 86400000 / 7);
  return `${day.getUTCFullYear()}-W${String(week).padStart(2, "0")}`;
}

/** Sum per-resource values into sorted buckets at granularity g, aligned on a shared axis. */
export function rollup(
  rows: AggRow[],
  g: Granularity,
): { categories: string[]; byResource: Map<string, number[]>; unitByResource: Map<string, string> } {
  const cats = Array.from(new Set(rows.map((r) => bucketKey(r.timestamp, g)))).sort();
  const idx = new Map(cats.map((c, i) => [c, i]));
  const byResource = new Map<string, number[]>();
  const unitByResource = new Map<string, string>();
  for (const r of rows) {
    if (!byResource.has(r.energy_type)) byResource.set(r.energy_type, new Array(cats.length).fill(0));
    byResource.get(r.energy_type)![idx.get(bucketKey(r.timestamp, g))!] += r.value;
    if (r.unit) unitByResource.set(r.energy_type, r.unit);
  }
  return { categories: cats, byResource, unitByResource };
}

/**
 * Fetch a node's rows for one aggregations action over the last `days`.
 *
 * Shared by the dashboard chart components, which were React islands each
 * repeating this. Resolves the node the same way every other widget does.
 * Throws with a readable message so callers can render it.
 */
export async function fetchAgg(
  action: string,
  { days = 30, resolution = "daily", levelId = "", extra = {} as Record<string, string> } = {},
): Promise<AggRow[]> {
  const lvl = levelId || resolveLevelId();
  if (!lvl) throw new Error("Ingen node valgt.");
  const end = new Date();
  const start = new Date(end.getTime() - days * 86400000);
  const params = new URLSearchParams({
    level_id: lvl,
    resolution,
    start: start.toISOString(),
    end: end.toISOString(),
    ...extra,
  });
  const res = await fetch(`${aggBase()}/meterdata/query/${action}?${params}`);
  if (!res.ok) throw new Error(`Fejl: ${res.statusText}`);
  return (await res.json()) as AggRow[];
}

/** Pivot rows into one chart series per energy type, on a shared sorted time axis. */
export function seriesByEnergyType(rows: AggRow[]): {
  categories: string[];
  series: { name: string; data: number[]; type: string; color?: string }[];
  unit: string;
} {
  const cats = Array.from(new Set(rows.map((r) => r.timestamp))).sort();
  const idx = new Map(cats.map((t, i) => [t, i]));
  const byType = new Map<string, number[]>();
  let unit = "";
  for (const r of rows) {
    if (!byType.has(r.energy_type)) byType.set(r.energy_type, new Array(cats.length).fill(0));
    byType.get(r.energy_type)![idx.get(r.timestamp)!] = r.value;
    if (!unit && r.unit) unit = r.unit;
  }
  return {
    categories: cats.map((t) => t.replace("T", " ").slice(0, 16)),
    series: Array.from(byType.entries()).map(([name, data]) => ({
      name: RESOURCE_LABELS[name] ?? name,
      data,
      type: "line",
      color: RESOURCE_COLORS[name],
    })),
    unit: unit || "kWh",
  };
}
