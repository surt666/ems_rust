// Alpine entrypoint — registered via @astrojs/alpinejs `entrypoint` option in
// astro.config.mjs. This runs BEFORE Alpine.start(), which is the only reliable
// place to register named `Alpine.data()` components in Astro (component-script
// `alpine:init` listeners attach too late and silently never fire).
import {
  defaultPreset,
  blankState,
  serialize,
  validate,
  wouldCycle,
  FIELD_TYPES,
} from "./lib/schema-serialize.js";

/** @param {import("alpinejs").Alpine} Alpine */
export default (Alpine) => {
  // Two-panel schema designer (markup + styles live in components/SchemaDesigner.astro).
  Alpine.data("schemaDesigner", () => ({
    state: defaultPreset(),
    sel: "company",
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

  // Create-node dialog scope (company creation gates on a valid schema + JSON submit).
  Alpine.data("createNode", () => ({
    nodeType: "partner",
    schema: null,
    schemaValid: false,

    onSchema(detail) { this.schema = detail.schema; this.schemaValid = detail.valid; },

    // Rebuild nested data from the form's dotted `data.*` field names
    // (mirrors the backend form-to-command nesting), skipping empty values and data.type.
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
          body: JSON.stringify({ action: "add_node", parent_id, data: this.formToData(form), schema: this.schema }),
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
};
