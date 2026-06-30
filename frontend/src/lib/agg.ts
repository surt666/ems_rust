// Shared helpers for the aggregations-API dashboard islands (ResourceChart,
// ResourceCards, CostCard, EmissionsCard, BenchmarkCard, AlarmsCard). Each island
// used to copy these verbatim; keep them here so the rollup-key contract and the
// resource labels stay in one place.

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
 * path. The same contract every dashboard island (and AggregationChartWrapper)
 * relies on.
 */
export function resolveLevelId(): string {
  const id = sessionStorage.getItem("selectedNodeId") || "";
  const path = sessionStorage.getItem("selectedNodePath") || "";
  const company = sessionStorage.getItem("selectedCompanyId") || "";
  let level = path ? `${path}#${id}` : id;
  if (company && level && !level.includes(company)) level = `${company}#${level}`;
  return level;
}
