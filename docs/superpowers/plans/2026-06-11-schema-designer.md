# Graphical Schema Designer Implementation Plan

> **As-shipped note:** the designer landed in the server-rendered add-child
> dialog and posts via a hidden `schema_json` field folded into the `add_node`
> command — not the masterdata create dialog / `create_node` JSON fetch this
> plan describes. The `schema-serialize.js` module + `SchemaDesigner.astro`
> editor are as planned.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A graphical, two-panel schema designer that opens in its own dialog during company (HN2) creation, builds the v2 type-graph schema from a preset with live validation, and posts it with `create_node`.

**Architecture:** A DOM-free pure module (`schema-serialize.js`) owns serialization + validation and is unit-tested with `node --test`. An Alpine component (`SchemaDesigner.astro`) renders the editor using that module and emits the schema via a CustomEvent. `masterdata.astro` hosts the designer dialog, gates company save on a valid schema, and submits the company as a JSON `fetch` (nested schema can't ride in form-encoding).

**Tech Stack:** Astro 6, Alpine.js 3, vanilla ESM, `node --test`, Playwright (already a devDependency).

**Spec:** `docs/superpowers/specs/2026-06-11-schema-designer-design.md`

**Working dir for all commands:** `/home/sla/projects/ems_rust/frontend`

**Backend note:** No backend change. `Command::AddNode { schema: Option<Value> }` and the hn1→hn2 path already validate the schema (`crates/services/hierarchy/src/json.rs :: schema_of_json`, `crates/model :: Schema::validate`).

---

### Task 1: Pure module — preset, blank, serialize

**Files:**
- Create: `frontend/src/lib/schema-serialize.js`
- Create: `frontend/src/lib/schema-serialize.test.js`

- [ ] **Step 1: Write failing tests for preset + serialize**

`frontend/src/lib/schema-serialize.test.js`:
```js
import { test } from "node:test";
import assert from "node:assert/strict";
import { defaultPreset, blankState, serialize } from "./schema-serialize.js";

test("preset serializes to the canonical v2 schema", () => {
  const json = serialize(defaultPreset());
  assert.deepEqual(json, {
    version: 2,
    edges: {
      company: { group: {}, property: {}, building: {} },
      group: { building: {} },
      property: { building: {} },
      building: { area: {} },
    },
    metadata: {
      building: {
        lat: { type: "number", required: true, min: -90, max: 90 },
        lng: { type: "number", required: true, min: -180, max: 180 },
      },
    },
    sensors: ["building", "area"],
  });
});

test("edges omit parents with no children; metadata omits empty types", () => {
  const json = serialize(blankState());
  assert.deepEqual(json.edges, {});
  assert.deepEqual(json.metadata, {});
  assert.deepEqual(json.sensors, []);
  assert.equal(json.version, 2);
});

test("each constraint type serializes correctly, blanks omitted", () => {
  const state = {
    types: ["company", "x"],
    edges: { company: ["x"], x: [] },
    metadata: {
      x: [
        { name: "s", type: "string", required: false, min_len: 1 },
        { name: "n", type: "number", required: true, min: 0, max: 5 },
        { name: "i", type: "integer", required: false },
        { name: "b", type: "boolean", required: false },
        { name: "t", type: "timestamp", required: false },
        { name: "e", type: "enum", required: true, one_of: ["a", "b"] },
      ],
    },
    sensors: [],
  };
  const md = serialize(state).metadata.x;
  assert.deepEqual(md.s, { type: "string", required: false, min_len: 1 });
  assert.deepEqual(md.n, { type: "number", required: true, min: 0, max: 5 });
  assert.deepEqual(md.i, { type: "integer", required: false });
  assert.deepEqual(md.b, { type: "boolean", required: false });
  assert.deepEqual(md.t, { type: "timestamp", required: false });
  assert.deepEqual(md.e, { type: "enum", required: true, one_of: ["a", "b"] });
});
```

- [ ] **Step 2: Run, expect failure**

Run: `node --test src/lib/schema-serialize.test.js`
Expected: FAIL ("Cannot find module './schema-serialize.js'").

- [ ] **Step 3: Implement preset/blank/serialize**

`frontend/src/lib/schema-serialize.js`:
```js
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
    const fields = state.metadata[t] || [];
    if (!fields.length) continue;
    const obj = {};
    for (const f of fields) obj[f.name] = serializeField(f);
    metadata[t] = obj;
  }
  return { version: 2, edges, metadata, sensors: [...(state.sensors || [])] };
}
```

- [ ] **Step 4: Run, expect pass**

Run: `node --test src/lib/schema-serialize.test.js`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/schema-serialize.js frontend/src/lib/schema-serialize.test.js
git commit -m "feat(frontend): pure schema serialize module + preset"
```

---

### Task 2: Pure module — graph helpers & validate

**Files:**
- Modify: `frontend/src/lib/schema-serialize.js`
- Modify: `frontend/src/lib/schema-serialize.test.js`

- [ ] **Step 1: Append failing tests**

Append to `frontend/src/lib/schema-serialize.test.js`:
```js
import { reachable, reachableFromRoot, longestDepth, wouldCycle, hasCycle, validate } from "./schema-serialize.js";

test("wouldCycle blocks an edge that closes a loop", () => {
  const edges = { company: ["group"], group: ["building"], building: [], area: [] };
  // building -> group would close company->group->building->group
  assert.equal(wouldCycle(edges, "building", "group"), true);
  assert.equal(wouldCycle(edges, "building", "area"), false);
  assert.equal(wouldCycle(edges, "building", "building"), true); // self
});

test("reachableFromRoot finds all connected types", () => {
  const edges = { company: ["group"], group: ["building"], building: [], orphan: [] };
  const r = reachableFromRoot(edges);
  assert.ok(r.has("building"));
  assert.ok(!r.has("orphan"));
});

test("longestDepth counts edges from company; boundary 7 ok / 8 too deep", () => {
  const mk = (n) => {
    const edges = {}; let p = "company";
    for (let i = 1; i <= n; i++) { const c = "t" + i; edges[p] = [c]; p = c; }
    edges[p] = [];
    return edges;
  };
  assert.equal(longestDepth(mk(7)), 7);
  const ok = validate({ types: ["company", ...Array.from({ length: 7 }, (_, i) => "t" + (i + 1))], edges: mk(7), metadata: {}, sensors: [] });
  assert.equal(ok.errors.some((e) => e.includes("too deep")), false);
  const bad = validate({ types: ["company", ...Array.from({ length: 8 }, (_, i) => "t" + (i + 1))], edges: mk(8), metadata: {}, sensors: [] });
  assert.equal(bad.errors.some((e) => e.includes("too deep")), true);
});

test("validate flags unreachable, partner, dup field, min>max, empty enum", () => {
  const state = {
    types: ["company", "building", "orphan", "partner"],
    edges: { company: ["building"], building: [], orphan: [], partner: [] },
    metadata: {
      building: [
        { name: "a", type: "number", required: true, min: 5, max: 1 },
        { name: "a", type: "string", required: false },
        { name: "e", type: "enum", required: true, one_of: [] },
      ],
    },
    sensors: [],
  };
  const { ok, errors } = validate(state);
  assert.equal(ok, false);
  assert.ok(errors.some((e) => e.includes("orphan") && e.includes("reachable")));
  assert.ok(errors.some((e) => e.includes("partner") && e.includes("reserved")));
  assert.ok(errors.some((e) => e.includes('duplicate field "a"')));
  assert.ok(errors.some((e) => e.includes("min > max")));
  assert.ok(errors.some((e) => e.includes("enum") && e.includes("value")));
});

test("validate catches a cycle even without the UI guard", () => {
  const state = { types: ["company", "a", "b"], edges: { company: ["a"], a: ["b"], b: ["a"] }, metadata: {}, sensors: [] };
  assert.ok(validate(state).errors.some((e) => e.includes("cycle")));
});

test("preset validates clean", () => {
  assert.equal(validate(defaultPreset()).ok, true);
});
```

- [ ] **Step 2: Run, expect failure**

Run: `node --test src/lib/schema-serialize.test.js`
Expected: FAIL (helpers/validate not exported).

- [ ] **Step 3: Implement helpers + validate**

Append to `frontend/src/lib/schema-serialize.js`:
```js
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

  if (hasCycle(edges, state.types)) errors.push("schema has a cycle");

  const reach = reachableFromRoot(edges);
  for (const t of state.types) {
    if (t !== RESERVED_ROOT && !reach.has(t)) errors.push(`type "${t}" is not reachable from company`);
  }

  const depth = longestDepth(edges);
  if (depth > MAX_DEPTH) errors.push(`hierarchy is too deep (${depth} levels below company; max ${MAX_DEPTH})`);

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
```

- [ ] **Step 4: Run, expect pass**

Run: `node --test src/lib/schema-serialize.test.js`
Expected: PASS (9 tests total).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/schema-serialize.js frontend/src/lib/schema-serialize.test.js
git commit -m "feat(frontend): schema validation + graph helpers"
```

---

### Task 3: SchemaDesigner Alpine component + dev harness page

**Files:**
- Create: `frontend/src/components/SchemaDesigner.astro`
- Create: `frontend/src/pages/dev/schema-designer.astro`

- [ ] **Step 1: Write the component**

`frontend/src/components/SchemaDesigner.astro`:
```astro
---
// Two-panel type-graph schema designer. Self-contained Alpine component.
// Emits a `schema-updated` CustomEvent on Done with { schema, valid }.
---
<dialog id="schema-designer-dialog" class="sd-dialog"
  _="on click if event.target == me then call me.close()">
  <div x-data="schemaDesigner" class="sd-root">
    <div class="dialog-header">
      <h2>Design hierarchy schema</h2>
      <button type="button" class="btn-close" _="on click call #schema-designer-dialog.close()">&times;</button>
    </div>

    <div class="sd-grid">
      <!-- Types -->
      <div class="sd-pane">
        <h4>Types</h4>
        <div class="sd-body">
          <template x-for="t in state.types" :key="t">
            <div class="sd-typeitem" :class="{ sel: t === sel }" @click="sel = t">
              <span x-text="t"></span>
              <span class="sd-rm" x-show="t !== 'company'" @click.stop="removeType(t)">&times;</span>
            </div>
          </template>
          <div class="sd-addrow">
            <input type="text" x-model="newType" placeholder="new type" @keydown.enter.prevent="addType()" />
            <button type="button" class="btn-primary" @click="addType()">＋</button>
          </div>
          <p class="sd-hint">"company" is the fixed root.</p>
        </div>
      </div>

      <!-- Editor -->
      <div class="sd-pane">
        <h4>Editing: <span x-text="sel"></span></h4>
        <div class="sd-body">
          <div class="sd-grp">
            <div class="sd-lbl">Allowed children</div>
            <template x-for="x in childCandidates()" :key="x.name">
              <label class="sd-chk" :class="{ disabled: x.disabled }">
                <input type="checkbox" :checked="x.on" :disabled="x.disabled"
                  @change="toggleChild(sel, x.name, $event.target.checked)" />
                <span x-text="x.name"></span>
                <span class="sd-why" x-show="x.disabled">(cycle)</span>
              </label>
            </template>
            <span class="sd-hint" x-show="childCandidates().length === 0">no other types yet</span>
          </div>

          <div class="sd-grp">
            <div class="sd-lbl">Metadata fields</div>
            <table class="sd-md">
              <template x-for="(f, i) in (state.metadata[sel] || [])" :key="i">
                <tr>
                  <td><input type="text" :value="f.name" @change="setField(i,'name',$event.target.value)" /></td>
                  <td>
                    <select @change="setField(i,'type',$event.target.value)">
                      <template x-for="ft in FIELD_TYPES" :key="ft">
                        <option :value="ft" :selected="ft === f.type" x-text="ft"></option>
                      </template>
                    </select>
                  </td>
                  <td><label class="sd-chk"><input type="checkbox" :checked="f.required" @change="setField(i,'required',$event.target.checked)" /> req</label></td>
                  <td class="sd-constraints">
                    <template x-if="f.type === 'number' || f.type === 'integer'">
                      <span>
                        <input type="number" placeholder="min" :value="f.min" @change="setField(i,'min',numOrUndef($event.target.value))" />
                        <input type="number" placeholder="max" :value="f.max" @change="setField(i,'max',numOrUndef($event.target.value))" />
                      </span>
                    </template>
                    <template x-if="f.type === 'string'">
                      <span>
                        <input type="number" placeholder="min len" :value="f.min_len" @change="setField(i,'min_len',numOrUndef($event.target.value))" />
                        <input type="number" placeholder="max len" :value="f.max_len" @change="setField(i,'max_len',numOrUndef($event.target.value))" />
                      </span>
                    </template>
                    <template x-if="f.type === 'enum'">
                      <input type="text" placeholder="a, b, c" :value="(f.one_of || []).join(', ')" @change="setField(i,'one_of',csv($event.target.value))" />
                    </template>
                  </td>
                  <td><span class="sd-rm" @click="removeField(i)">&times;</span></td>
                </tr>
              </template>
            </table>
            <button type="button" class="btn-secondary" @click="addField()">＋ add field</button>
          </div>

          <div class="sd-grp">
            <div class="sd-lbl">Sensors</div>
            <label class="sd-chk">
              <input type="checkbox" :checked="state.sensors.includes(sel)" @change="toggleSensor(sel,$event.target.checked)" />
              sensors may attach to <b x-text="sel"></b> nodes
            </label>
          </div>
        </div>
      </div>

      <!-- Validation + preview -->
      <div class="sd-side">
        <div class="sd-pane">
          <h4>Validation</h4>
          <div class="sd-body">
            <template x-if="result.ok"><p class="sd-ok">✓ valid (depth <span x-text="result.depth"></span> ≤ 7, deepest node hn<span x-text="2 + result.depth"></span>)</p></template>
            <template x-for="e in result.errors" :key="e"><p class="sd-bad" x-text="'✗ ' + e"></p></template>
          </div>
        </div>
        <div class="sd-pane">
          <h4>DAG preview</h4>
          <div class="sd-body sd-dag" x-text="dagPreview()"></div>
        </div>
      </div>
    </div>

    <div class="dialog-footer">
      <p class="sd-hint" x-show="!result.ok">Fix the issues above to enable Done.</p>
      <button type="button" class="btn-secondary" @click="reset()">Clear</button>
      <button type="button" class="btn-warning" :disabled="!result.ok" @click="done()">Done</button>
    </div>
  </div>
</dialog>

<script>
  import { defaultPreset, blankState, serialize, validate, wouldCycle, FIELD_TYPES } from "../lib/schema-serialize.js";

  document.addEventListener("alpine:init", () => {
    window.Alpine.data("schemaDesigner", () => ({
      state: defaultPreset(),
      sel: "building",
      newType: "",
      result: { ok: true, errors: [], depth: 0 },
      FIELD_TYPES,

      init() { this.revalidate(); },
      revalidate() { this.result = validate(this.state); },

      numOrUndef(v) { const n = parseFloat(v); return Number.isNaN(n) ? undefined : n; },
      csv(v) { return v.split(",").map((s) => s.trim()).filter(Boolean); },

      childCandidates() {
        return this.state.types
          .filter((x) => x !== this.sel && x !== "company")
          .map((x) => {
            const on = (this.state.edges[this.sel] || []).includes(x);
            const dis = !on && wouldCycle(this.state.edges, this.sel, x);
            return { name: x, on, disabled: dis };
          });
      },
      addType() {
        const v = this.newType.trim().toLowerCase();
        this.newType = "";
        if (!v || this.state.types.includes(v) || v === "partner") return;
        this.state.types.push(v);
        this.state.edges[v] = [];
        this.sel = v;
        this.revalidate();
      },
      removeType(t) {
        if (t === "company") return;
        this.state.types = this.state.types.filter((x) => x !== t);
        delete this.state.edges[t];
        delete this.state.metadata[t];
        for (const k of Object.keys(this.state.edges)) this.state.edges[k] = this.state.edges[k].filter((x) => x !== t);
        this.state.sensors = this.state.sensors.filter((x) => x !== t);
        if (this.sel === t) this.sel = "company";
        this.revalidate();
      },
      toggleChild(p, c, on) {
        const set = new Set(this.state.edges[p] || []);
        on ? set.add(c) : set.delete(c);
        this.state.edges[p] = [...set];
        this.revalidate();
      },
      toggleSensor(t, on) {
        const set = new Set(this.state.sensors);
        on ? set.add(t) : set.delete(t);
        this.state.sensors = [...set];
        this.revalidate();
      },
      addField() {
        const t = this.sel;
        const a = (this.state.metadata[t] = this.state.metadata[t] || []);
        a.push({ name: "field" + (a.length + 1), type: "string", required: false });
        this.revalidate();
      },
      removeField(i) {
        this.state.metadata[this.sel].splice(i, 1);
        if (!this.state.metadata[this.sel].length) delete this.state.metadata[this.sel];
        this.revalidate();
      },
      setField(i, k, v) {
        this.state.metadata[this.sel][i][k] = v;
        this.revalidate();
      },
      dagPreview() {
        return this.state.types
          .map((x) => { const ch = this.state.edges[x] || []; return ch.length ? `${x} → ${ch.join(", ")}` : null; })
          .filter(Boolean)
          .join("\n");
      },
      reset() { this.state = blankState(); this.sel = "company"; this.revalidate(); },
      done() {
        this.revalidate();
        if (!this.result.ok) return;
        window.dispatchEvent(new CustomEvent("schema-updated", { detail: { schema: serialize(this.state), valid: true } }));
        document.getElementById("schema-designer-dialog").close();
      },
    }));
  });
</script>

<style>
  .sd-dialog { width: min(1100px, 94vw); border: none; border-radius: 12px; padding: 0; }
  .sd-root { padding: 1rem 1.25rem 1.25rem; }
  .sd-grid { display: grid; grid-template-columns: 180px 1fr 300px; gap: 14px; align-items: start; margin-top: 1rem; }
  .sd-pane { border: 1px solid var(--border-medium, #d4d9e0); border-radius: 8px; background: #fff; color: #1d2330; margin-bottom: 14px; }
  .sd-pane h4 { margin: 0; padding: 8px 12px; border-bottom: 1px solid #e3e8ef; font-size: 12px; text-transform: uppercase; letter-spacing: .04em; color: #5b6675; background: #f3f6fa; border-radius: 8px 8px 0 0; }
  .sd-body { padding: 10px 12px; }
  .sd-typeitem { display: flex; justify-content: space-between; align-items: center; padding: 6px 8px; border-radius: 6px; cursor: pointer; color: #1d2330; }
  .sd-typeitem:hover { background: #f3f6fa; }
  .sd-typeitem.sel { background: #2563eb; color: #fff; }
  .sd-rm { opacity: .6; font-weight: bold; padding: 0 4px; cursor: pointer; }
  .sd-rm:hover { opacity: 1; color: #dc2626; }
  .sd-addrow { display: flex; gap: 6px; margin-top: 8px; }
  .sd-addrow input { flex: 1; }
  .sd-root input[type=text], .sd-root input[type=number], .sd-root select { border: 1px solid #d4d9e0; border-radius: 6px; padding: 5px 7px; font-size: 13px; color: #1d2330; background: #fff; }
  .sd-grp { margin-bottom: 16px; }
  .sd-lbl { font-weight: 600; font-size: 13px; margin-bottom: 6px; }
  .sd-chk { display: inline-flex; align-items: center; gap: 5px; margin: 3px 10px 3px 0; font-size: 13px; }
  .sd-chk.disabled { color: #9aa3b0; }
  .sd-why { font-size: 11px; font-style: italic; }
  .sd-md { width: 100%; border-collapse: collapse; font-size: 13px; }
  .sd-md td { padding: 3px 4px; }
  .sd-md input[type=text] { width: 96px; }
  .sd-md input[type=number] { width: 64px; }
  .sd-dag { font-family: ui-monospace, Menlo, monospace; font-size: 12px; white-space: pre; color: #334; }
  .sd-hint { font-size: 11px; color: #5b6675; margin-top: 4px; }
  .sd-ok { color: #16a34a; font-size: 13px; }
  .sd-bad { color: #dc2626; font-size: 13px; margin: 2px 0; }
</style>
```

- [ ] **Step 2: Write the dev harness page (unauthenticated render target for Playwright)**

`frontend/src/pages/dev/schema-designer.astro`:
```astro
---
// Unauthenticated harness for testing the SchemaDesigner in isolation (Playwright).
// Not linked from anywhere; renders the designer and exposes the emitted schema on window.
import SchemaDesigner from "../../components/SchemaDesigner.astro";
---
<html>
  <head><title>SchemaDesigner harness</title></head>
  <body>
    <button id="open" onclick="document.getElementById('schema-designer-dialog').showModal()">open</button>
    <SchemaDesigner />
    <script>
      window.__lastSchema = null;
      window.addEventListener("schema-updated", (e) => { window.__lastSchema = e.detail.schema; });
    </script>
  </body>
</html>
```

- [ ] **Step 3: Verify the build compiles**

Run: `npm run build`
Expected: build succeeds, no errors. (Confirms the Astro component + script + import resolve.)

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/SchemaDesigner.astro frontend/src/pages/dev/schema-designer.astro
git commit -m "feat(frontend): SchemaDesigner Alpine component + dev harness"
```

---

### Task 4: masterdata integration — host designer, gate + submit company

**Files:**
- Modify: `frontend/src/pages/masterdata.astro`

The create dialog's structure is: `<dialog id="create-dialog">` → `.dialog-header` → `.dialog-body`
(holds `#node-form-error` + `<form id="create-node-form">`) → `.dialog-footer` (holds the "Gem" save
button, which targets the form via `form="create-node-form"`). **The save button is OUTSIDE
`.dialog-body`,** so the Alpine scope must live on the `<dialog id="create-dialog">` element to cover
both the form fields and the save button.

- [ ] **Step 1: Import the designer in the frontmatter**

In the frontmatter (the `---` block at the top, with the other imports) of
`frontend/src/pages/masterdata.astro`, add:
```astro
import SchemaDesigner from '../components/SchemaDesigner.astro';
```

- [ ] **Step 2: Define the `createNode()` Alpine component**

Add this `<script>` at the end of `frontend/src/pages/masterdata.astro` (after the closing markup;
a bare `<script>` in an `.astro` page is bundled and runs client-side):
```astro
<script>
  document.addEventListener("alpine:init", () => {
    window.Alpine.data("createNode", () => ({
      nodeType: "partner",
      schema: null,
      schemaValid: false,

      onSchema(detail) { this.schema = detail.schema; this.schemaValid = detail.valid; },

      // Rebuild nested data from the form's dotted `data.*` field names
      // (mirrors backend form_to_command_json), skipping empty values and data.type.
      formToData(form) {
        const data = {};
        for (const el of form.querySelectorAll("[name^='data.']")) {
          if (el.value === "" || el.value == null) continue;
          if (el.name === "data.type") continue;
          const path = el.name.slice("data.".length).split(".");
          let cur = data;
          for (let i = 0; i < path.length - 1; i++) cur = cur[path[i]] = cur[path[i]] || {};
          cur[path[path.length - 1]] = el.value;
        }
        return data;
      },

      async submitCompany() {
        const form = document.getElementById("create-node-form");
        const errBox = document.getElementById("node-form-error");
        errBox.style.display = "none";
        if (!this.schemaValid || !this.schema) {
          errBox.textContent = "Definér et gyldigt skema først.";
          errBox.style.display = "block";
          return;
        }
        const parent_id = sessionStorage.getItem("selectedNodePath") + "#" + sessionStorage.getItem("selectedNodeId");
        try {
          const resp = await fetch("/hierarchy/command", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ action: "create_node", parent_id, data: this.formToData(form), schema: this.schema }),
          });
          if (!resp.ok) {
            errBox.textContent = await resp.text();
            errBox.style.display = "block";
            return;
          }
          document.getElementById("create-dialog").close();
          location.reload();
        } catch (e) {
          errBox.textContent = "Fejl: " + e.message;
          errBox.style.display = "block";
        }
      },
    }));
  });
</script>
```

- [ ] **Step 3: Put the Alpine scope on the create dialog**

Change the create dialog's opening tag (keep its existing hyperscript `_=` attribute) to add the
Alpine scope and the schema event listener:
```astro
<dialog id="create-dialog" x-data="createNode()"
  @schema-updated.window="onSchema($event.detail)"
  _="on click if event.target == me then call me.close()">
```

On the type select `<select id="node-type-select" ...>`, add an Alpine `@change` next to the existing
hyperscript `_=` (keep the hyperscript — it toggles field visibility):
```astro
@change="nodeType = $event.target.value"
```

- [ ] **Step 4: Add the "Design skema" button inside the company fields**

Just after the opening `<div class="form node-fields node-fields-company" ...>`, add:
```astro
<div class="form-row-2col">
  <label class="form-label">Hierarki-skema</label>
  <button type="button" class="btn-secondary"
    @click="document.getElementById('schema-designer-dialog').showModal()">
    <span x-show="!schemaValid">Design skema *</span>
    <span x-show="schemaValid">✓ Skema defineret (rediger)</span>
  </button>
</div>
```

- [ ] **Step 5: Intercept the save button for the company case**

The save button in `.dialog-footer` is `<button type="submit" form="create-node-form" class="btn-primary">Gem</button>`.
Add an Alpine click handler so company submits go to JSON and every other type keeps the existing
HTMX form post:
```astro
<button type="submit" form="create-node-form" class="btn-primary"
  @click="if (nodeType === 'company') { $event.preventDefault(); submitCompany(); }">
```
(`preventDefault` on the submit button stops the native submit, so HTMX never fires for companies.
Non-company types are untouched.)

- [ ] **Step 6: Render the designer dialog once**

Immediately after the closing `</dialog>` of `#create-dialog`, add:
```astro
<SchemaDesigner />
```

- [ ] **Step 7: Verify build**

Run: `npm run build`
Expected: succeeds (component + script + import all resolve).

- [ ] **Step 8: Commit**

```bash
git add frontend/src/pages/masterdata.astro
git commit -m "feat(frontend): schema designer in company creation; JSON submit"
```

---

### Task 5: Playwright smoke against the harness

**Files:**
- Create: `frontend/playwright.config.ts`
- Create: `frontend/e2e/schema-designer.spec.ts`

- [ ] **Step 1: Add Playwright config (builds + previews the static site)**

`frontend/playwright.config.ts`:
```ts
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  use: { baseURL: "http://localhost:4321" },
  webServer: {
    command: "npm run build && npm run preview -- --port 4321",
    url: "http://localhost:4321/dev/schema-designer",
    timeout: 120_000,
    reuseExistingServer: true,
  },
});
```

- [ ] **Step 2: Write the smoke spec**

`frontend/e2e/schema-designer.spec.ts`:
```ts
import { test, expect } from "@playwright/test";

test("designer loads preset, blocks cycles, captures serialized schema", async ({ page }) => {
  await page.goto("/dev/schema-designer");
  await page.click("#open");

  // Preset types present
  await expect(page.locator(".sd-typeitem", { hasText: "building" })).toBeVisible();

  // Select "group"; its child checkbox for "company" is absent and "building" is checked.
  await page.locator(".sd-typeitem", { hasText: "group" }).click();
  // building is an allowed child of group in the preset → checked
  const buildingChk = page.locator(".sd-chk", { hasText: "building" }).locator("input[type=checkbox]");
  await expect(buildingChk).toBeChecked();

  // Select "building": "group" appears disabled (cycle: group→building exists)
  await page.locator(".sd-typeitem", { hasText: "building" }).first().click();
  const groupChk = page.locator(".sd-chk", { hasText: "group" }).locator("input[type=checkbox]");
  await expect(groupChk).toBeDisabled();

  // Done emits the schema
  await page.click("button:has-text('Done')");
  const schema = await page.evaluate(() => (window as any).__lastSchema);
  expect(schema.version).toBe(2);
  expect(schema.edges.company).toHaveProperty("group");
  expect(schema.metadata.building.lat).toMatchObject({ type: "number", required: true, min: -90, max: 90 });
  expect(schema.sensors).toContain("building");
});
```

- [ ] **Step 3: Install the browser and run**

Run: `npx playwright install chromium && npx playwright test`
Expected: 1 passed.

**Environment fallback:** if `playwright install` cannot fetch a browser in this environment, skip running and instead perform this manual check, recording the result in the commit message: `npm run preview` → open `/dev/schema-designer` → click open → confirm preset renders, selecting "building" disables the "group" child with "(cycle)", Done logs a `version:2` schema with `metadata.building.lat`. Do NOT delete the spec — it runs in CI/other envs.

- [ ] **Step 4: Commit**

```bash
git add frontend/playwright.config.ts frontend/e2e/schema-designer.spec.ts
git commit -m "test(frontend): playwright smoke for schema designer"
```

---

### Task 6: Final verification

- [ ] **Step 1: Unit tests + build green**

Run: `cd frontend && node --test src/lib/schema-serialize.test.js && npm run build`
Expected: 9 unit tests pass; build succeeds.

- [ ] **Step 2: Confirm clean tree**

Run: `git status --short`
Expected: clean (all committed).

---

## Deployment runbook (manual — NOT executed by this plan)

Per `CLAUDE.md` (frontend stack, account `339712745226`):
```bash
cd frontend && npm run build          # PUBLIC_* unchanged; designer calls same-origin /hierarchy/command
cd ../infra/frontend && unset GOROOT
AWS_PROFILE=stel-sb cdk deploy OcamlFrontendStack --require-approval never
```
Verify: log in → masterdata → select a partner → create → type "company" → "Design skema" opens the
designer → build/preset → Done → save → the company is created with its schema (check a child can be
added under it via the add-child dialog, which reads the schema we just wrote).
