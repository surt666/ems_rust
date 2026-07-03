# Brand assets — logo, colors, font (captured from live ems.enity.io, 2026-07-03)

Extracted from the live app (company 997) + the standalone login SPA at `/auth/`.
Raw SVGs saved in `./brand/` and copied into `frontend/public/images/`.

## Logo

**One wordmark, two colour variants** (identical `viewBox="0 0 1536.8 325"`, Adobe
Illustrator export). Pick the variant by background:

| File | Where it's used | Text fill | Accent | For background |
|---|---|---|---|---|
| `logo-app-topleft.svg` | in-app header, top of the hierarchy panel (`.me2-logo`, rendered ~132×32, `background-size: contain`) | `#fff` (white) | `#f69d3a` (orange) | **dark** (`#003644`) |
| `logo-login.svg` | login page (`<img alt="Logo">`, native 300×63, shown `max-w-[400px]`) | `#003644` (teal) | `#f69d3a` (orange) | **light** |

- Source URLs (Enity CDN):
  - app/top-left: `https://cdn.minenergi2.dk/uploads/upload_03cc4d68c59a58213af799e83c627838.svg`
  - login: `https://cdn.minenergi2.dk/uploads/upload_90771144eb350338205b71a19344e184.svg`
- Both also use `opacity: .4` on a secondary element (a faded part of the mark).
- The app logo sits **inside** the dark hierarchy panel's header strip (no separate
  header bar background) — that's why it ships as the white variant.

## Colors

| Token | Value (hex) | rgb | Where |
|---|---|---|---|
| **Brand teal** (primary) | `#003644` | `0,54,68` | **hierarchy panel background**; login logo text fill |
| **Brand orange** (accent) | `#f69d3a` | `246,157,58` | logo accent (both variants) |
| Content background | `#F3F4F6` | `243,244,246` | main app `body` / page canvas (light gray) |
| Body text | `#4E4E4E` | `78,78,78` | main app default text color |
| Logo-on-dark | `#FFFFFF` | `255,255,255` | app logo text fill (on the teal panel) |

- **Hierarchy panel background = `#003644`** (`.hierarchy-navigation`, computed
  `rgb(0, 54, 68)`, panel width ~300px). This is the primary brand teal.

## Font

- **Main app (use this — most widely used):**
  `"Source Sans Pro", Arial, Helvetica, sans-serif`
  (computed on `<body>` and the hierarchy panel across the whole authenticated app).
- Login/auth SPA only: `"Public Sans", sans-serif` (a separate Vue+Tailwind SPA under
  `/auth/`, not the main app). Ignore for the app UI; **Source Sans Pro** is the app face.

## Login page extras
- Left hero background image: `/auth/assets/background-RAWRQ3N7.jpg` (glass building at
  dusk with an on-brand orange bar-chart overlay, 1920×1080). Downloaded to
  `./brand/login-background.jpg` and wired in as `frontend/public/images/login-building.jpg`
  (the `.login-brand-bg` source).
- The `/auth/` app is a distinct SPA (PrimeIcons + Tailwind, `Public Sans`), separate
  from the main authenticated app (`Source Sans Pro`).
