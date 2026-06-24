# Module 7 — Aktiv Styring, Rapportering & Call to action

Three top-level menu modules grouped here: **Aktiv Styring** (Active control — childless,
upsell), **Rapportering** (Reporting — 3 pages), and **Call to action** (childless). Scope:
company **Bolag AB Janus** (`/App/company/997/...`).

---

## 7.1 Aktiv Styring — `active_management`  *(childless top-level; upsell landing)*
Screenshot: `screenshots/c997-as-aktivstyring.png`

A **marketing / upsell landing page** for the Active Control product — **not** an operational tool
for this company.

- **Content:** heading `Aktiv Styring` + tagline
  "Opnå massive besparelser med Aktiv Styring. Læs mere om dine muligheder her."
  (Achieve massive savings with Active Control. Read more about your options here.)
- **Components:** none (no charts, tables, filters, forms) — static promo text/links only.
- **APIs:** boilerplate only (no page-specific endpoint).

## 7.2 Rapportering — Brugertilpassede rapporter — `reports/custom`
Screenshot: `screenshots/c997-ra-brugertilpassede.png`

**Custom reports** — user-defined report builder/registry.

- **Regions (two sections):** `Rapporter jeg har oprettet` (reports I created) and
  `Rapporter andre har oprettet` (reports others created). Both empty for c997 →
  empty-state "Der blev ikke fundet nogle rapporter" (no reports found).
- **Key button:** **`Opret ny rapport`** (create new report — read-only, not exercised).
- **Charts/tables:** none in the empty state (each section becomes a report list when populated).
- **APIs (page-specific):**
  `GET /api/customreports/getCompanyUsers/{companyId}/company/true`,
  `GET /api/misc/companyIdAndName`,
  `GET /api/common/languages`, `GET /api/common/measurement-systems`
  (report-builder option sources).

## 7.3 Rapportering — Bygningsrapporten — `reports/buildings`
Screenshot: `screenshots/c997-ra-bygningsrapporten.png`

**The building report** — per-building report **subscriptions** (who receives the scheduled report).

- **Filtre bar** (`Filtre`, `Nulstil filtre`): `Brugere` (users; default `Administrator`),
  `Bygninger` (buildings; "+8 bygninger valgt" = +8 selected).
- **`Abonnementer`** (subscriptions) **grid (ag-Grid, 12 rows — grouped by building):** columns
  `Abonnent` (subscriber), `Gruppering` (grouping), `Kontaktperson` (contact),
  `Brugerprofil` (user profile, e.g. Firmaansvarlig), `Afsendes` (sent/schedule, e.g.
  "Man. kl. 07:00"), `Årlig omk. (FA)` (annual cost, Firmaansvarlig basis, e.g. "2.587.200 kr"),
  `Tilmeldt` (subscribed). Cells show "0 ud af 1 Abonnenter", and cost may read "Intet forbrug"
  (no consumption). Row-grouping affordance ("Drag here to set column labels").
- **Tabs (subscription filter):** `Alle` / `Frameldt` (unsubscribed) / `Tilmeldt` (subscribed).
- **Note banner:** Enity-staff info that, lacking direct data access to the element, rows are grouped
  by building and the "brugere" filter is forced to "alle".
- **Charts:** none.
- **APIs (page-specific):**
  `POST /api/reports/getbuildingsubscriptionbyfilter/`,
  `POST /api/reports/getreportfilter/`,
  `POST /api/analysis/meterdata/filtergroupby` (the annual-cost figures),
  `POST /api/filter/getbuildingsbyfilter/`, `POST /api/filter/getmetersbyfilter/`.

## 7.4 Rapportering — Eksporter forbrugsdata — `consumption_export`
Screenshot: `screenshots/c997-ra-eksporter.png`

**Consumption-data export** — build a filtered export job and download the generated file.

- **`Filtre` panel** (`Nulstil filtre`):
  - `Periode` (date range, `01-06-2025 - 31-05-2026`); `Opløsning` (resolution dropdown:
    `Valgte periode` / `15 min` / `Time` / `Dag` / `Uge` / `Måned`).
  - Multi-selects (each with `Fravælg alle` = deselect all): `Bygninger` (buildings),
    `Bygningsanvendelse` (building use), `Målertype` (meter type; with `og | eller`),
    `Vælg tags`, `Målere` (meters; "+66 målere valgt"), plus checkbox `Inkludér bimålere`
    (include sub-meters, checked).
  - `Indgå i summeringssammenhænge` (include in summation contexts; default `Alle`).
  - `Tællerenhed` (counter unit: `Tællerens nuværende enhed` / `Energiform`),
    `Klimakorrigér forbrug` (climate-correct consumption: `Ja` / `Nej`, default `Nej`).
  - **Live preview row:** `Antal bygninger 12`, `Antal målere 67`, `Antal datalinier 67`,
    `Estimeret eksporttid: få sekunder` (estimated export time).
  - **`Eksport`** button (queues the export job — read-only, not exercised).
- **`Eksportfiler`** (export files) **table:** columns `Dato`, `Antal bygninger`, `Antal målere`,
  `Antal datalinier`, `Status` (0 rows → "Der er ingen tilgængelige eksportfiler i dit depot");
  footnotes: you may leave EMS while files generate; files are deleted after 7 days.
- **Charts:** none.
- **APIs (page-specific):**
  `GET /api/export/getjobstatus/user-{userId}` (export-job polling),
  `POST /api/filter/getmetersbyfilter/`.

## 7.5 Call to action — `call_to_action`  *(childless top-level)*
Screenshot: `screenshots/c997-cta.png`

The full **Call-to-action** surface (the dashboard's "5 største energispild" card, expanded) — the
biggest energy-waste / extra-cost opportunities, sourced from unacknowledged alarms.

- **Totals header:** `Total est. meromk. pr. år` (total est. extra cost/yr, `0,0`),
  `Total meromk. dd.` (extra cost to date, `0,0`), `Ukvitterede alarmer` (unacknowledged alarms).
- **Grid (ag-Grid, row-groupable; 0 rows for c997):** columns
  `Bygning / Målertype / Måler / Alarmtype / Alarm` (the group/path column),
  `Ukvit. alarmer` (unacknowledged alarms), `Seneste alarm` (latest alarm),
  `Meromk. dd.` (extra cost to date), `Est. meromk. pr. år` (est. extra cost per year).
  Empty-state: "Listen er tom" / "Der er endnu ingen opsatte alarme" (no alarms configured yet).
- **Charts:** none.
- **APIs (page-specific):** `POST /api/calltoaction/getcalltoaction` (also boilerplate everywhere).

---

### Recreation notes
- **Aktiv Styring** = static upsell page; gate behind a feature flag and render a promo block
  (server-rendered HTML, no data calls).
- **Brugertilpassede rapporter** = report builder/registry — two server-rendered report lists +
  "Opret ny rapport" flow (HTML-over-the-wire). Option sources (`languages`,
  `measurement-systems`, company users) load up front.
- **Bygningsrapporten** & **Call to action** both reuse the **groupable ag-Grid** (row-group +
  column labels) seen in Bygningsbenchmark / CSRD — one grid component, different columns + list
  endpoint. Bygningsrapporten's cost column comes from `analysis/meterdata/filtergroupby`.
- **Eksporter forbrugsdata** = a **filter-form + live-count preview + async job + results table**
  pattern (queue export → poll `export/getjobstatus` → list files). Reuse the FilterBar's
  multi-selects/`Fravælg alle`/`og|eller`; design the export job as a background task with polling.
- Call to action is driven entirely by alarm data (`getcalltoaction`); empty company → first-class
  empty-state pointing at alarm setup.
