# Module 5 — Målere (Meters)

A **childless top-level** menu module: clicking "Målere" navigates straight to the meter list (no
flyout). Scope captured: company **Bolag AB Janus** (`/App/company/997/...`).

> Note: the top menu shows **two** "Målere" entries side by side — the second is badged **Nyt**
> (New), a newer variant of the same surface. Only the first (this page) was captured; treat the
> "Nyt" one as the same meter-list surface, newer iteration.

---

## 5.1 Målere — `meters`
Screenshot: `screenshots/c997-ma-maalere.png`

The company's **meter registry** — a filterable, paginated, column-configurable grid of all meters.

- **Heading:** "Målere for Bolag AB Janus — 69 målere (heraf 0 valgt)" (69 meters, 0 selected) +
  `Filtre og visning`.
- **Grid (ag-Grid, paginated; ~60 rows rendered per page of 69 total):** columns
  `Bygning` (building), `Målertype` (meter type — coloured energy icon + label, e.g. Naturgas /
  Fjernvarme / Manuel måler), `Hierarki` (hierarchy badge), `Målerbetegnelse` (meter designation),
  `Unikt Id` (unique id), `Tags`. Header-checkbox selection ("Column with Header Selection",
  per-row "Press Space to toggle row selection"); paginator (page size dropdown, default page 1).
- **Toolbar:** `Hurtig søgning…` placeholder "Søg efter unikt id, målerbeteg…" (search by unique id /
  designation), **`Opret måler`** (create meter — read-only, not exercised).
- **Filter panel** (`Filtre og visning`), grouped:
  - `Bygningsfiltre` (building filters): `Vælg bygninganvendelser` (building use, 4 options),
    `Bygninger` (12).
  - `Måler-filtre` (meter filters): `Vælg målertyper` (meter types, 11) with an **`og | eller`**
    (AND/OR) combinator, `Vælg tags` (3), `Vælg målerdetaljer` (meter details, 3),
    `Vælg adfærd` (behaviour, 3).
  - Each group `Nulstil`; panel actions `Filtre`, `Visning` (columns), `Nulstil filtre`,
    `Vis målere` (apply → "show meters"), `Nulstil visning` (reset view), `Luk`.
- **Charts:** none.
- **APIs (page-specific):**
  `POST /api/meters/list` (the grid data),
  `POST /api/filter/getbuildingsbyfilter` (building-filter resolution),
  `GET /api/common/custom-filter` (saved filters).

---

### Recreation notes
- Same **data-grid + `Filtre og visning` overlay** component as Resource Insights / Overvågning;
  here the filter groups are building + meter, the list endpoint is `POST /api/meters/list`.
- Grid is selectable + paginated + searchable + column-configurable (ag-Grid-class) → reuse the one
  grid component flagged across the other modules.
- `Målertype` cell renders an energy-type icon + label; drive it from the meter's resource/type
  (same icon vocabulary as the dashboard's Forbrugsoverblik and Emissionsfaktorer).
- The `og | eller` combinator on meter-types is a server-side filter mode (AND vs OR) — expose it as
  part of the POSTed filter body, not client logic.
- Two nav entries (`Målere` + `Målere [Nyt]`): treat as one route family; the "Nyt" variant is the
  forward-looking version of this registry. Recreate the stable one first.
