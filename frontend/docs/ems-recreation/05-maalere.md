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
- **Per-row `...` action menu** (pinned-right `actionColumn`, `fa-ellipsis` toggle → `meter-list-button-menu`).
  Screenshot: `screenshots/c997-ma-row-actionmenu.png`. Items (icon · label):
  `fa-list` **Vis flere detaljer**, `fa-pencil` **Redigér måler**, `fa-tags` **Rediger målertags**,
  `fa-chart-bar` **Gå til forbrug**, `fa-database` **Gå til datatilegnelse** (→ §5.2),
  `fa-plugSlash` **Deaktivér måler** (destructive), `fa-trash-can` **Slet måler** (destructive).
- **Charts:** none.
- **APIs (page-specific):**
  `POST /api/meters/list` (the grid data),
  `POST /api/filter/getbuildingsbyfilter` (building-filter resolution),
  `GET /api/common/custom-filter` (saved filters).

---

## 5.2 Datatilegnelse (data acquisition) — `datasource?maalerId=<id>`
Screenshot: `screenshots/c997-ma-datatilegnelse.png` · reached from the row menu's **Gå til datatilegnelse**
(`/App/company/997/datasource?maalerId=593466&quickLink=true`). Title "Enity EMS - Datatilegnelse".

The per-meter **data-source + counter-reading** workbench: which logger/source feeds the meter, its
counter registers (incl. **start value**), and the full reading/odometer time-series with per-reading
corrections (meter change, rollover, manual readings).

- **Header:** meter name + EMS id + **physical meter number**, e.g. "Gas Billing 593466 (Fysisk
  målernummer: 1111113)"; status toggle **`I drift` / `Under installation`**.
- **Filter bar:** `Fra` / `Til` (datasource validity date range), `Fysisk målernummer`, `Type`.
- **Datasource block:** header `electrocom (Box: 11848302)` + validity `1.1.2015, 01.00 – 31.12.2099,
  01.00`; **Type** dropdown = the logger/source vocabulary: `Electrocom, Danfoss, Danfoss_v2,
  lichtwart, Techem, Datahub, Kinect, ME1 import, CsvFile, ME1 Conversion, Manual, Import Historical,
  Brunata, DS En…`. (This is the *datatilegnelse* concept the Oversigt "Fjernaflæst datatilegnelse"
  count rolls up — Datalogger / API / Filer.)
- **Counter-registers grid (Tællerdele):** columns `Enhed` (unit), `Gange-faktor` (multiplier),
  `Omregningsfaktor` (conversion factor), **`Startværdi`** (start value), `DAQ ID`, `Indberettes i
  forbrug` (reported in consumption Y/N). Example row `T1`: `m³-Ngas | 1 | 1 | 0 |
  electrocom:11848302:11848302:1 | Nej` (DAQ ID = `logger:box:box:register`). `Gem opsætning` saves.
- **Readings grid** (per register tab, e.g. `T1 Energi`): columns `Dato` (date+hour), **`Ans.
  tællerst.`** (anslået tællerstand = estimated counter reading / **odometer**), `Aflæsning` (actual
  reading), `Forbrug` (consumption). Hourly rows. Period selector `Vælg periode` (date range) +
  `Antal rækker` (row cap, "100 / N") + export. Toolbar: **`Indsæt ny aflæsning`**, `Slet datakilde`,
  `Luk`.
- **Per-reading `...` menu** (the important one): `Indsæt aflæsning over` / `…under`, `Rediger
  aflæsning`, `Slet aflæsning`, **`Justér anslået tællerstand`** (adjust estimated odometer),
  `Gå til forbrug`, **`Tællervending`** (counter rollover/wrap), **`Målerskifte`** (meter change /
  device swap), `Split datakilde her`, `Vis ændringslog (N)` (change log).
- **Charts:** none (grids only).
- **APIs:** `GET /api/meters/{id}/datasources/entities`, `GET
  /api/meters/{id}/datasources/counters?isActive=true`, `POST /api/sensorMeasurements/getmeasurements`.

> **Relevance to the base-value plan** (`docs/superpowers/specs/2026-06-26-physical-meter-reading-base-value.md`):
> the real EMS already models this domain explicitly — **`Startværdi`** per counter register = the
> per-meter base; **`Ans. tællerst.`** = the reconstructed odometer shown to users; **`Målerskifte`**
> and **`Tællervending`** are the meter-change / rollover events that reset the base; **`Justér anslået
> tællerstand`** is a manual odometer correction. Strong naming + requirements reference for when that
> plan resumes.

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
