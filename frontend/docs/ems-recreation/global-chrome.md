# Global chrome (app shell)

The shell that wraps every page. Three persistent regions + the routed page content.

```
┌───────────────┬───────────────────────────────────────────────────────────────┐
│ LEFT          │ HEADER (breadcrumb ......................... toolbar icons · user)│
│ hierarchy     ├───────────────────────────────────────────────────────────────┤
│ panel         │ MODULE BAR (horizontal top menu) .............. [export][refresh]│
│ (collapsible) ├───────────────────────────────────────────────────────────────┤
│               │                                                                 │
│  tree of      │   ROUTED PAGE CONTENT                                           │
│  buildings    │                                                                 │
│               │                                                                 │
└───────────────┴───────────────────────────────────────────────────────────────┘
                                                              "Powered by Enity" (footer)
```

## 1. Left hierarchy panel (`.hierarchy-navigation`)

Collapsible (toggle = `fa-chevrons-left`). Selects the **context** that scopes the whole app.

- **Scope tabs:** `Hierarki` (Hierarchy) · `Fælles grp.` (Shared groups) · `Privat grp.` (Private groups).
- **Department picker** — combobox, placeholder `Vælg afdeling` (Select department); searchable;
  current value e.g. `Enity Personale`. Caption "Listen viser 9999 ud af 25 mulige" (showing N of 25).
- **Company picker** — combobox, placeholder `Vælg firma` (Select company); searchable; selecting a
  company switches context → `/App/company/<id>/...` (e.g. *Bolag AB Janus* → `company/997`).
  44 companies available; options may carry a small icon (favourite/marker).
- **Asset search** — text input `Søg i ejendomshierarki...` (Search the property hierarchy).
- **Tree** below: nested **administrator(department) → company → building → area** nodes
  (source: `GET /api/getnavigationtree`, node shape `{id, name, type, children}`,
  `type ∈ administrator|company|building|area`). Clicking a node re-scopes the app.

### API
- `GET /api/getnavigationtree` → full hierarchy tree (large; 25 departments at root).
- `GET /api/groups/companyGroups`, `GET /api/groups/privateGroups` → the two "grp." tabs.

## 2. Header (top bar)

- Left: **collapse-panel** button (`fa-chevrons-left`) + **breadcrumb**:
  `<context name> › <module> › <page>` (e.g. `Enity Personale › Oversigt › Statistik`).
  A context link such as `Se alle bygninger (12)` ("See all buildings (N)") appears for a company.
- Right: icon toolbar →
  | icon | label | purpose |
  |---|---|---|
  | `fa-bell` | (notifications) | alarm/notification bell |
  | `fa-bullhorn` | **Nyheder** | news / announcements (`GET /api/announcement`, `/api/news/*`) |
  | `fa-bookmark` | **Favoritter** | saved favourites (`/api/common/custom-filter?filterName=favourites`) |
  | `fa-magnifying-glass` | (search) | global search |
  | `fa-grid-2` | (apps) | app/module launcher grid |
  | `fa-question` | (help) | help |
  | `fa-user` | **Steen** | user menu (account, sign out) |

## 3. Module bar (`.app__top-menu`, `.navigation--horizontal`)

- Horizontal top menu = the 9 modules; hovering a module reveals its pages (see `index.md` tree).
  Active module/page highlighted (`navigation-item--highlight-box`).
- Right end: **export** + **refresh** action buttons for the current page.
- Item markup: `.navigation-item--horizontal > .navigation-item__highlight > svg(icon) + .navigation-item__title__text`,
  optional badge (`Nyt` / `In progress` / `Udfases`).

## 4. Footer
- `Powered by Enity` with link to `https://enity.io`.

## Cross-cutting startup APIs (fire on every load)
- `GET /api/user/startup`, `GET /api/user/rights` (role/permissions → which modules render),
  `GET /api/yggdrasil/organization/system-theme?contextType=&contextId=` (theming),
  `GET /api/announcement`, `GET /api/alarm-management/alarm-runs/user/<userId>`.

## Recreation notes (our frontend)
- Shell = persistent 2-pane grid (left context tree + right routed area) with a sticky header and
  module bar. **Use CSS grid for the shell layout (project rule: grid, not flex).**
- Context (`contextType`/`contextId`) is global state driving every API call's `?contextType=&contextId=`.
- Role/rights gate which modules/pages appear — model menu visibility off `user/rights`.
- Hypermedia: our stack renders HTML fragments over HTMX (project rule) — the module bar and page
  bodies are good fragment boundaries.
