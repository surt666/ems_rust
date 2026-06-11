---
tags:
  - slides
  - ems
  - DAQ
theme: white
height: 1200
width: 1600
margin: 0
maxScale: 4
---
<!-- slide template="[[tpl-kc-title]]" -->
::: title1
EMS Product Vision

:::

::: title2
Steen Larsen
::: 

---
<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Customers
:::

::: block
| Customer Segment | Primary Use |
|---|---|
| **EMS** | Licensed customers operating the platform |
| **Intelligence** | ML pipelines, BI dashboards |
| **Technicians** | Device commissioning, meter identity, diagnostics |
| **Energy Advisors** | BI reports, consumption analysis beyond EMS defaults |
| **Marketing** | User activity analytics |
| **Billing** | Meter inventory, hierarchy-based cost allocation |
| **External API users** | Third-party integrations |
:::

---
<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Extensibility and Simplicity
:::

::: block
- Everything is a **graph** — a hierarchy is just a DAG
- Nodes carry **typed metadata** governed by per-company schema rules
- Only the traversal mechanism is rigid — everything else is configurable:
  - Node types allowed at each level
  - Parent → child edge cardinality and labels
  - Metadata fields per node type (required vs. optional, e.g. lat/lon)
- **Access = entry-point(s)** into the graph — a user sees only the sub-graph they're entitled to
- **Sensors are leaf nodes** — no special-casing needed
- Edges encode both **structure** and **permissions**
- Graph on DynamoDB: fast, cheap, regionally available
:::

---
<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Hierarchy — Structure
:::

::: block
```
hn0  root          (singleton — platform root)
 └─ hn1  partner   (business tenant, e.g. "Acme Partner")
     └─ hn2  company   (owns the schema that shapes its entire subtree)
         └─ hn3  group / property / building   (schema-defined)
             └─ hn4  area / floor              (schema-defined)
                 └─ hn5  room / zone           (schema-defined)
                     └─ sensor  (leaf node, attached at any schema-declared level)
```

- **hn0–hn1–hn2** edges are hard-coded (partner, company)
- **hn2+ edges** are declared by the company schema
- Level-skipping (e.g. `hn2 → hn4`) is allowed if the schema declares that edge
- Each company can have a **completely different tree shape** — sibling companies are independent
- Node ids are `HN<n>#<int>` — unique within a level, not globally
:::

---
<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Hierarchy — User Types & Permissions
:::

::: block
| Role | Entry point | What they can do |
|---|---|---|
| **Platform admin** | hn0 (root) | Full access to all partners, companies, nodes, users |
| **Partner admin** | hn1 | Manages companies under their partner; creates company admins |
| **Company admin** | hn2 | Creates/edits nodes, defines schema, manages users in their company |
| **Write user** | hn3+ (configured) | Edits **metadata** on nodes they have access to; cannot add/delete nodes |
| **Read user** | hn3+ (configured) | Read-only view of nodes and measurements in their sub-graph |
| **Blocked** | — | A specific node (and its subtree) is invisible — e.g. a restricted lab |

**Permission rules:**
- Permissions **inherit top-down** from the entry-point node unless explicitly overridden
- **Admin** = full structural control (add/delete children, manage users)
- **Write** = update metadata on existing nodes, but cannot restructure
- **Read** = read all data from entry-point downward
- **Blocked** = hard exclude of a specific node, even inside an otherwise-accessible sub-graph
- Field-level access (CEDAR rules) available if needed, but generally not required
:::

---
<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Per-Company Schema
:::

::: block
The schema lives on `hn2` and governs **only** that company's subtree.

```json
{
  "version": 1,
  "edges": {
    "hn2": [
      { "child": "hn3", "label": "property", "min": 1, "max": null },
      { "child": "hn3", "label": "group" }
    ],
    "hn3": [
      { "child": "hn4", "label": "building" },
      { "child": "sensor" }
    ]
  },
  "metadata": {
    "hn3": [
      { "field": "address",  "type": "string", "required": true },
      { "field": "lat",      "type": "float",  "required": true },
      { "field": "lon",      "type": "float",  "required": true }
    ]
  },
  "sensor_levels": ["hn3", "hn4"]
}
```

- **Edges** declare parent→child relationships with cardinality (`min`/`max`)
- **Metadata** declares typed fields per level — validated on every write
- **sensor_levels** declares where sensors may be attached
- Two companies → two schemas → two completely different tree shapes
:::

---
<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
The Data Pipeline
:::

::: block
- All measurement data flows through a **single unified pipeline**
- Common error handling applies to all sources
- Source-specific parsing (API, physical meter, file) is isolated in individual parsers

**[→ See full pipeline presentation](../docs/system-design-presentation.html)**

```
IoT devices / APIs / Files
       │
       ▼
   AWS IoT Core / HTTPS
       │
       ▼
    Kinesis  ──▶  Flink  ──┬──▶ raw_data (Iceberg)
                           │
                           └──▶ enrich ──▶ resample ──▶ logical_meter_data (Iceberg)
                                                                │
                                                         hourly Glue job
                                                                │
                                                                ▼
                                                    measurements_aggregate (DynamoDB)
```
:::

---

<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Conclusion
:::

::: block
- **Thinking in graphs** unlocks simplicity without sacrificing power
  - Hierarchy = DAG; access = sub-graph entrypoint; sensors = leaf nodes
  - One model covers all customer shapes — no bespoke schemas per product
- **Performance & cost benefits**
  - DynamoDB graph: sub-millisecond traversals, no joins, pay-per-read
  - All Lambdas — no long-running servers, no synchronous bottlenecks
  - Reports run as async jobs (WebSocket / SSE callbacks)
- **Scalability & operations**
  - 2–3 true microservices + a few helpers (vs. many today)
  - Data enrichment and validation in Flink/Spark — never blocks user requests
  - Regional deployment by design
- **Isolation by design**
  - Per-company schemas prevent cross-tenant data bleed
  - Permission model is structural, not a bolt-on — it's the graph itself
:::
