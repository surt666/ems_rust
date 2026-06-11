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

export function reachable(edges, from, to) {
  const seen = new Set();
  const stack = [from];
  while (stack.length) {
    const n = stack.pop();
    for (const c of edges[n] || []) {
      if (c === to) return true;
      if (!seen.has(c)) { seen.add(c); stack.push(c); }
    }
  }
  return false;
}

export function reachableFromRoot(edges) {
  const seen = new Set([RESERVED_ROOT]);
  const stack = [RESERVED_ROOT];
  while (stack.length) {
    const n = stack.pop();
    for (const c of edges[n] || []) if (!seen.has(c)) { seen.add(c); stack.push(c); }
  }
  return seen;
}

export function longestDepth(edges, node = RESERVED_ROOT, memo = {}) {
  if (node in memo) return memo[node];
  let d = 0;
  for (const c of edges[node] || []) d = Math.max(d, 1 + longestDepth(edges, c, memo));
  memo[node] = d;
  return d;
}

// Adding edge parent->child would create a cycle (or is a self-edge)?
export function wouldCycle(edges, parent, child) {
  return parent === child || reachable(edges, child, parent);
}

// 3-color DFS over all declared parents (defensive; the UI also blocks cycles).
export function hasCycle(edges, types) {
  const WHITE = 0, GRAY = 1, BLACK = 2;
  const color = {};
  const nodes = new Set([...(types || []), ...Object.keys(edges || {})]);
  const visit = (n) => {
    color[n] = GRAY;
    for (const c of edges[n] || []) {
      if (color[c] === GRAY) return true;
      if ((color[c] || WHITE) === WHITE && visit(c)) return true;
    }
    color[n] = BLACK;
    return false;
  };
  for (const n of nodes) if ((color[n] || WHITE) === WHITE && visit(n)) return true;
  return false;
}

export function validate(state) {
  const errors = [];
  const edges = state.edges || {};

  const cyclic = hasCycle(edges, state.types);
  if (cyclic) errors.push("schema has a cycle");

  const reach = reachableFromRoot(edges);
  for (const t of state.types) {
    if (t !== RESERVED_ROOT && !reach.has(t)) errors.push(`type "${t}" is not reachable from company`);
  }

  const depth = cyclic ? 0 : longestDepth(edges);
  if (!cyclic && depth > MAX_DEPTH) errors.push(`hierarchy is too deep (${depth} levels below company; max ${MAX_DEPTH})`);

  const seenT = new Set();
  for (const t of state.types) {
    if (seenT.has(t)) errors.push(`duplicate type "${t}"`);
    seenT.add(t);
  }
  if (state.types.includes(RESERVED_PARTNER)) errors.push(`"partner" is reserved and cannot be a type`);

  for (const t of Object.keys(state.metadata || {})) {
    const seenF = new Set();
    for (const f of state.metadata[t] || []) {
      if (!f.name) errors.push(`a field in "${t}" has no name`);
      else {
        if (seenF.has(f.name)) errors.push(`duplicate field "${f.name}" in "${t}"`);
        seenF.add(f.name);
      }
      if ((f.type === "number" || f.type === "integer") && isNum(f.min) && isNum(f.max) && f.min > f.max)
        errors.push(`field "${f.name}" in "${t}": min > max`);
      if (f.type === "string" && isNum(f.min_len) && isNum(f.max_len) && f.min_len > f.max_len)
        errors.push(`field "${f.name}" in "${t}": min length > max length`);
      if (f.type === "enum" && !(f.one_of && f.one_of.length))
        errors.push(`enum field "${f.name}" in "${t}" needs at least one value`);
    }
  }

  return { ok: errors.length === 0, errors, depth };
}
