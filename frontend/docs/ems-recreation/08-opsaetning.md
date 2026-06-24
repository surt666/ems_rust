# Module 8 — Opsætning (Setup / administration)

The administration module. Pages captured for company 997: **Stamdata** (company master data),
**Brugeradministration** (user administration), **Bygningselementer** (building elements). The
top-level "Opsætning" entry sits in the menu's settings/overflow group (gear icon) and its leaves
open the three admin surfaces below. Scope: company **Bolag AB Janus** (`/App/company/997/...`).

---

## 8.1 Stamdata — `setup/company_data?quickLink=true`
Screenshot: `screenshots/c997-op-stamdata.png`

**Company master data** — a tabbed company profile/configuration editor (read-only view + Redigér).

- **Heading:** "🏠 Bolag AB Janus", with a warning banner
  "Vær opmærksom på: Firmaet er under opsætning." (the company is under setup) and an **`Action`**
  dropdown (company-level actions).
- **Tabs:** `Firmadata` (company data), `Energipriser` (energy prices), `CO₂e faktorer`
  (CO₂e factors), `Frie Nøgletal- og Tags` (free KPIs & tags), `Frie Stamfelter`
  (free master-data fields), `Frie målertyper` (free meter types), `Generelt` (general),
  `Infoboards`, `Licens` (license).
- **Firmadata tab (read-only form):** labelled fields `Juridisk navn` (legal name), `Adresse`,
  `Postnr. / By` (zip / city), `Land` (country, `DK`), `Telefon`, `Email`,
  `Kontaktperson` (contact person, e.g. "Firmaansvarlig, Administrator, …"),
  `Status` (`Under opsætning`). A side panel "Her kan der uploades billeder og dokumenter"
  (image/document upload). **`Redigér …`** button switches the form to edit mode (read-only here —
  not exercised).
- **Charts/tables:** none.
- **APIs (page-specific):**
  `GET /api/company/getaddressdata/{companyId}`,
  `GET /api/company/getactiondropdown/{companyId}` (the Action menu),
  `GET /api/building/initialization-data`.

## 8.2 Brugeradministration — `user_administration`
Screenshot: `screenshots/c997-op-brugeradministration.png`

**User administration** — the company's users with profile, access and welcome-mail status.

- **Heading:** "Brugeradministration for: Bolag AB Janus" + `Filtre og visning`, help, recorder.
- **Toolbar:** **`Opret bruger`** (create user — read-only, not exercised),
  search "Søg efter navn, brugerprofil, bruger-id, e-mail, telefon eller sprog",
  **`Vis brugeraktivitet`** toggle (show user activity), `Eksport`, `…` overflow.
- **Grid (ag-Grid, 1 row for c997):** columns
  `Navn` (name; with a phone/login-status badge), `Brugerprofil` (user profile, e.g.
  `Firmaansvarlig`), `Bruger ID` (user id), `Email`, `Telefon`,
  `Er velkomstmail sendt` (welcome mail sent, `Nej`),
  `Dataadgang` (data access, e.g. "12 bygninger"). Header-checkbox + per-row select + row `⋮` menu.
- **Filter panel** (`Filtre og visning` → `Filtrér brugere`): `Brugerprofil` (8),
  `Velkomstmail sendt` (2), `Er låst` (locked, 2); `Nulstil`, `Visning` (columns).
- **In-page help block:** Overblik / Søg, filtrér og tilpas / Tilføjelse af nye brugere /
  Administration af eksisterende brugere / Eksport af brugerdata / Bedste praksis.
- **Charts:** none.
- **APIs (page-specific):** `POST /api/user/user-list` (the grid data).

## 8.3 Bygningselementer — `building_list`
Screenshot: `screenshots/c997-op-bygningselementer.png`

**Building elements** — the registry of buildings/properties/groupings (the things meters hang off).

- **Heading:** "Bygningselementer for: Bolag AB Janus — 16 bygningselementer (heraf 0 valgt)" +
  `Filtre og visning`.
- **Grid (ag-Grid, 16 rows):** columns
  `Betegnelse` (designation/name), `Bygningsanvendelse` (building use), `By` (city), `Land`
  (country), `Totalt areal` (total area), `Opvarmet areal` (heated area),
  `Kontaktperson` (contact person). Header-checkbox + per-row select; `Hurtig søgning...`.
- **Toolbar:** **`Opret bygningselement`** (create building element — read-only, not exercised),
  `Eksport`.
- **Filter panel** (`Filtre og visning` → `Bygningsfiltre`): `Vælg bygninganvendelser` (6),
  `Vælg byer` (cities, 11), `Vælg kontaktpersoner` (contacts, 1), `Vælg lande` (countries, 2),
  `Vælg tidszoner` (time zones, 2), `Vælg bygningstyper` (building types, 3),
  `Vælg energiklasser` (energy classes, 8); `Nulstil filtre`, `Visning`, `Luk`.
- **Charts:** none.
- **APIs (page-specific):**
  `GET /api/building/buildings` (the grid data),
  `GET /api/building/initialization-data` (filter option sources).

---

### Recreation notes
- **Stamdata** is the one **tabbed master-data editor** — distinct from the grid pages. Build a
  tabbed form shell (Firmadata + 8 config tabs); the read-only/`Redigér` toggle is the key pattern
  (server renders read-only fields, swaps to an edit form on Redigér — HTML-over-the-wire). The
  `Action` menu and Licens/Infoboards tabs are company-admin features.
- **Brugeradministration** & **Bygningselementer** reuse the **data-grid + `Filtre og visning`
  overlay** component (selectable, searchable, column-configurable) seen across the app; configure
  columns + filter groups + list endpoint (`user/user-list` resp. `building/buildings`).
- `building/initialization-data` is the shared option source for building filters (used here and on
  Bygningselementer; also seen on Stamdata) — load once, reuse.
- Create flows (`Opret bruger`, `Opret bygningselement`) and the Stamdata `Redigér` form are the
  module's only mutations — render as server HTML forms; out of scope for this read-only crawl.
- These are the **admin** surfaces; gate behind admin/company-responsible roles.
