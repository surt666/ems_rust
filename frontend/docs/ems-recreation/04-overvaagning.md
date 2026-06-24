# Module 4 — Overvågning (Monitoring)

The alarm/monitoring module. Pages present for company 997: **Alarmer** (alarms list),
**Alarm opsætning** (alarm setup). A third menu entry **AI Alarmcenter** is **role/feature-gated** —
the leaf exists in the nav DOM but does not navigate for this company (clicking it stays on the
dashboard); capture it from an enabled context if we ever need it.

Both live pages share the same shell as the other grid pages: a heading with a count
("N alarmer (heraf 0 valgt)" = N alarms, 0 selected), a **`Filtre og visning`** (filters & view)
overlay panel (`Filtre`, `Visning`, `Nulstil`/`Nulstil filtre`, `Luk`), an **ag-Grid** result area
with header-checkbox selection, a quick-search, an `Eksport` action, and a screen-recorder/help icon.

---

## 4.1 Alarmer — `monitoring/monitoring_consumption`
Screenshot: `screenshots/c997-ov-alarmer.png`

**Unacknowledged-alarms list** ("Ukvitterede alarmer").

- **Heading:** "Ukvitterede alarmer — 0 alarmer (heraf 0 valgt)".
- **Grid (ag-Grid, 0 rows for c997 → empty-state "Listen er tom"):** columns
  `Bygning` (building), `Energiform` (energy form), `Måler` (meter), `Målerid` (meter id), `Tags`,
  `Navn` (name), `Type`, `Alarmmodtagere` (alarm recipients), `Afvigelse` (deviation),
  `Meromkostning` (extra cost; sorted desc by default). Header checkbox + quick-search + `Eksport`.
- **Filter panel** (`Filtre og visning`): grouped into
  `Alarm-filtre` (Alarmperiode = alarm period; Alarmstatus, default `Ukvitterede alarmer`;
  "Listen viser 50 ud af 3 mulige" = showing 50 of 3 possible) · `Bygningsfiltre` (building filters) ·
  `Måler-filtre` (meter filters). Each group has its own `Nulstil`.
- **Charts:** none.
- **APIs (page-specific):**
  `GET /api/alarm-management/alarm-runs/user/{userId}`,
  `GET /api/common/custom-filter` (saved filters).

## 4.2 Alarm opsætning — `monitoring/monitoring_setup`
Screenshot: `screenshots/c997-ov-alarmopsaetning.png`

**Alarm-configuration list** ("Alarmopsætninger") — the registry of configured alarms.

- **Heading:** "Alarmopsætninger for Bolag AB Janus — 0 alarmopsætninger (heraf 0 valgt)".
- **Grid (ag-Grid, 0 rows for c997 → "Listen er tom"):** columns
  `Målertype` (meter type), `Måler` (meter; sorted asc), `Navn` (name), `Type`,
  `Alarmmodtagere` (alarm recipients). Header checkbox + `Hurtig søgning...` (quick search).
- **Key buttons:** **`Opret alarm`** (create alarm — read-only, not exercised), `…` (overflow).
- **Filter panel** (`Filtre og visning`): `Alarm-filtre`
  (`Alarmstatus` default `Alle`; **`Visning af alarmer`** = alarm view, segmented
  `Alle` / `Almindelige` (ordinary) / `Backoffice`) · `Bygningsfiltre` (`Vælg bygninger`) ·
  `Måler-filtre` (`Vælg målertyper`, `Vælg målere`). Each group `Nulstil`.
- **Charts:** none.
- **APIs (page-specific):**
  `GET /api/alarm-management/alarm-runs/user/{userId}`,
  `POST /api/filter/getmetersbyfilter` (meter filter resolution),
  `GET /api/common/custom-filter`.

## 4.3 AI Alarmcenter  *(gated / not reachable for c997)*
Screenshot: `screenshots/c997-ov-ai-alarmcenter.png` *(shows the dashboard — the leaf did not
navigate for this company).*

The nav leaf "AI Alarmcenter" is present in the menu markup but is **role/feature-gated** for Bolag
AB Janus: activating it does not change the route (no dedicated `/api/...` call fires; the dashboard
stays mounted). Re-capture from a context where the AI alarm feature is licensed. Likely an
AI-assisted anomaly/alarm triage surface (name suggests an ML-ranked alarm centre), but its real
layout/endpoints are unknown from this scope.

---

### Recreation notes
- Alarmer & Alarm opsætning are the **same grid-page pattern** as Resource Insights / Målere:
  one reusable **data-grid + `Filtre og visning` overlay** component, parameterised by columns,
  filter groups, and the backing list endpoint — build once, configure per page.
- Both backed by the shared `alarm-management/*` endpoints (already treated as boilerplate elsewhere:
  `alarmruns`, `alarmconfigurations`, `alarm-feedback/missing-feedback-count`); the page-specific
  additions are the per-user `alarm-runs/user/{id}` and (on setup) `filter/getmetersbyfilter`.
- Empty company → grids show "Listen er tom"; design a first-class empty-state and the
  `Opret alarm` CTA path (server-rendered create form, HTML-over-the-wire — project rule).
- `Visning af alarmer` (Alle/Almindelige/Backoffice) is a role-scoped server filter, not a client tab.
- AI Alarmcenter: gate the nav entry on a feature flag; render nothing/redirect when unlicensed
  (matches observed behaviour).
