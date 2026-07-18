# Raw Device microfrontend — design

**Date:** 2026-07-18
**Status:** approved (design), pending spec review

## Goal

Demonstrate htmx-based **microfrontends**: an independently-built, independently-deployed
frontend (the "Raw Device verifier portal") whose content is composed *at runtime* into the
existing main app shell via htmx `hx-get` + `hx-select`. First iteration is scaffolding only:
a form to enter a `daqid`. Querying device data (via the DuckDB lambda or Athena, against a
device data source that is **not** the `raw_data` table) is a **later** iteration, out of scope here.

## Decisions

- **Composition model:** htmx compose-into-shell (not a full-page link, not an iframe). The shell
  fetches the whole MFE page and selects just its main container.
- **MFE tech:** plain static HTML + htmx, **no build step** — makes the "a microfrontend can be any
  independent tech" point and keeps it decoupled from the Astro shell.
- **Hosting:** **daq_dev account (891377204778)** — co-located with the meter data it will later
  query. S3 (private) behind **CloudFront** (HTTPS + OAC + default root object + CORS via a
  response-headers policy). CloudFront is required because the shell is HTTPS and an S3 *website*
  endpoint is HTTP-only (mixed-content blocked); CloudFront also serves `index.html` at `/`.
- **Shell nav:** new top-level **"3rd Party"** dropdown with one sub-item **"Raw Device"**.
- **Removal policy:** bucket `DESTROY` + auto-delete (it's a demo).

## Components

### 1. `frontend-raw-device/` (new top-level folder — the MFE)

Static, no build. Deployed as-is.

- `index.html` — a **full standalone portal page** (own `<head>`, minimal self-contained styles,
  htmx vendored). Its body contains exactly one main container:
  ```html
  <div id="raw-device">
    <h1>Raw Device verifier</h1>
    <form id="rd-form">
      <label for="rd-daqid">Daq ID</label>
      <input id="rd-daqid" name="daqid" required autocomplete="off" />
      <button type="submit">Verify</button>
    </form>
    <div id="rd-result"><!-- stubbed this iteration --></div>
  </div>
  ```
  Visiting the CloudFront URL directly → full portal. Pulled into the shell → just `#raw-device`.
- `htmx.min.js` — vendored (no external CDN; CSP-safe, and the MFE self-hosts its own htmx).
- **Form submit is stubbed this iteration:** on submit, render a placeholder into `#rd-result`
  ("Query wiring coming next — daqid: <value>") with no backend call. No data source is wired yet.

### 2. `RawDevicePortalStack` (new Go CDK stack in `infra/daq/data_pipeline`, daq_dev account)

- **S3 bucket** — private, `RemovalPolicy: DESTROY`, `AutoDeleteObjects: true`.
- **CloudFront distribution** — S3 origin via **OAC**, `DefaultRootObject: index.html`, HTTPS.
- **Response-headers policy (CORS)** — on the default behavior: `Access-Control-Allow-Origin:
  https://d24beiqs2cj89y.cloudfront.net` (the shell origin), `Access-Control-Allow-Methods: GET`.
  The shell suppresses htmx's `HX-*` headers (`hx-request` `noHeaders`), so the fetch is a
  CORS-simple GET and **no `OPTIONS` preflight** is needed — the response header alone satisfies it.
- **BucketDeployment** — syncs `frontend-raw-device/` to the bucket + invalidates `/*`.
- **Output** `RawDeviceMfeUrl` = `https://<dist>.cloudfront.net` (the value baked into the shell).
- Registered in `infra/daq/data_pipeline/main.go` alongside the other stacks
  (`Env: defaultEnv()`, deploys with the daq_dev creds).

### 3. Shell changes (`frontend/`, account 339712745226)

- **`src/components/Navbar.astro`** — add a top-level "3rd Party" dropdown (following the existing
  `nav-dropdown-indicator` + nested `<ul class="nav-dropdown">` pattern used by "Resource Insights"
  etc.), with one sub-item `<a href="/rawdevice">Raw Device</a>`. `data-i18n` keys
  `nav.third_party` / `nav.raw_device` added to the i18n dictionaries (with literal fallback text).
- **`src/pages/rawdevice.astro`** — thin page using the shared `Layout`. Its content region composes
  the MFE on load:
  ```astro
  const mfeUrl = import.meta.env.PUBLIC_RAWDEVICE_MFE_URL || "";
  ...
  <div id="rd-content"
       hx-get={mfeUrl}
       hx-select="#raw-device"
       hx-swap="innerHTML"
       hx-trigger="load"
       hx-request='{"noHeaders":true}'
       hx-on::response-error="this.innerHTML = 'Raw Device portal unavailable.'">
    Loading Raw Device portal…
  </div>
  ```
  `hx-select="#raw-device"` plucks just the MFE's main div out of the fetched full page.
  `hx-request='{"noHeaders":true}'` suppresses htmx's `HX-*` request headers so the cross-origin
  fetch stays a **CORS-simple GET** (no `OPTIONS` preflight) — same precedent as the measurements
  page. Empty `mfeUrl` (env not set) degrades to the "Loading…" placeholder rather than erroring.
- **`frontend/.env` + `.env.example`** — new `PUBLIC_RAWDEVICE_MFE_URL` (the `RawDeviceMfeUrl`
  output, HTTPS, no trailing slash). Astro inlines it at build; rebuild after changes.

## Data flow

1. User clicks **3rd Party → Raw Device** → browser navigates to the shell page `/rawdevice`.
2. On load, `#rd-content` issues a cross-origin `hx-get` to the MFE CloudFront URL.
3. CloudFront serves the MFE `index.html` with the CORS header allowing the shell origin.
4. htmx selects `#raw-device` from the response and swaps it into `#rd-content`.
5. The daqid form now renders inside the shell (same nav/layout). Submitting is stubbed.
6. Direct visit to the MFE CloudFront URL → the same page, standalone.

## Error handling

- Cross-origin fetch failure (network / CORS / missing env) → `hx-on::response-error` (and the empty
  `mfeUrl` fallback) leave a plain "unavailable" message in `#rd-content`; the shell stays usable.
- Bad/empty daqid → native form `required` validation client-side (no submit).

## Testing (manual, this iteration)

- Visit the MFE CloudFront URL directly → full standalone portal renders, form present.
- Visit shell `/rawdevice` → the `#raw-device` form composes into the shell content area (verify via
  network tab: one cross-origin GET to the MFE URL, 200 + `Access-Control-Allow-Origin`).
- Nav: "3rd Party" dropdown shows "Raw Device"; active-state styling matches siblings.

## Explicitly out of scope (later iterations)

- Wiring the daqid form to a real query (DuckDB lambda or Athena) against the device data source
  (which is **not** `raw_data`). The submit is a client-side stub for now.
- Auth/Cognito on the MFE (it's a public demo portal for now).
- A custom domain / WAF on the MFE CloudFront.
