// Pure, DOM-free schema model for the v2 type-graph designer.
// Serializes to the exact shape crates/services/hierarchy/src/json.rs::schema_of_json parses.

export const RESERVED_ROOT = "company";
export const RESERVED_PARTNER = "partner";
export const FIELD_TYPES = ["string", "number", "integer", "boolean", "timestamp", "enum"];
export const MAX_DEPTH = 7; // longest path from "company"; deepest node = hn(2+depth) ≤ hn9

export function defaultPreset() {
  return {
    types: ["company", "group", "property", "building", "area"],
    edges: {
      company: ["group", "property", "building"],
      group: ["building"],
      property: ["building"],
      building: ["area"],
      area: [],
    },
    metadata: {
      building: [
        { name: "lat", type: "number", required: true, min: -90, max: 90 },
        { name: "lng", type: "number", required: true, min: -180, max: 180 },
      ],
    },
    sensors: ["building", "area"],
  };
}

export function blankState() {
  return { types: [RESERVED_ROOT], edges: { [RESERVED_ROOT]: [] }, metadata: {}, sensors: [] };
}

const isNum = (v) => typeof v === "number" && !Number.isNaN(v);

function serializeField(f) {
  const out = { type: f.type, required: !!f.required };
  if (f.type === "string") {
    if (isNum(f.min_len)) out.min_len = f.min_len;
    if (isNum(f.max_len)) out.max_len = f.max_len;
  } else if (f.type === "number" || f.type === "integer") {
    if (isNum(f.min)) out.min = f.min;
    if (isNum(f.max)) out.max = f.max;
  } else if (f.type === "enum") {
    out.one_of = [...(f.one_of || [])];
  }
  return out;
}

export function serialize(state) {
  const edges = {};
  for (const p of state.types) {
    const kids = state.edges[p] || [];
    if (kids.length) {
      const o = {};
      for (const c of kids) o[c] = {};
      edges[p] = o;
    }
  }
  const metadata = {};
  for (const t of Object.keys(state.metadata || {})) {
    if (!state.types.includes(t)) continue;
    const fields = state.metadata[t] || [];
    if (!fields.length) continue;
    const obj = {};
    for (const f of fields) obj[f.name] = serializeField(f);
    metadata[t] = obj;
  }
  return { version: 2, edges, metadata, sensors: [...(state.sensors || [])] };
}
