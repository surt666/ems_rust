# Raw Device microfrontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a standalone static HTML+htmx "Raw Device verifier" microfrontend (daqid form) hosted in the daq_dev account, composed into the main app shell via `hx-get` + `hx-select`, reachable from a new "3rd Party → Raw Device" nav item.

**Architecture:** The MFE is a self-contained `index.html` (its own portal) with a `#raw-device` main div. It's deployed to a private S3 bucket behind CloudFront (HTTPS + OAC + CORS response-headers policy) in the daq_dev account. The shell's `/rawdevice` page fetches that page cross-origin and `hx-select`s just `#raw-device` into its content area. The query wiring (DuckDB/Athena) is a later iteration — the form submit is stubbed.

**Tech Stack:** static HTML + htmx (vendored, no build); Go AWS CDK v2 (S3 + CloudFront); Astro shell (existing).

## Global Constraints

- MFE bucket + CloudFront live in **daq_dev account 891377204778**, region **eu-central-1**; deploy with the `daq_dev` SSO creds + the CLAUDE.md cdk recipe (`unset GOROOT`, export creds, `CDK_DEFAULT_ACCOUNT=891377204778`).
- Shell (Astro) lives in account **339712745226** (`stel-sb`); `PUBLIC_*` env vars are inlined at build — rebuild after any `.env` change.
- Cross-origin fetch must stay a **CORS-simple GET**: shell uses `hx-request='{"noHeaders":true}'`; CloudFront CORS `Access-Control-Allow-Origin: https://d24beiqs2cj89y.cloudfront.net`, methods `GET`.
- The MFE main container id is exactly **`raw-device`** (shell `hx-select="#raw-device"` depends on it).
- Never `git push`; commit locally only.

---

### Task 1: Raw Device MFE static files

**Files:**
- Create: `frontend-raw-device/index.html`
- Create: `frontend-raw-device/htmx.min.js` (vendored)

**Interfaces:**
- Produces: a standalone HTML page served at the bucket root; its `#raw-device` div is the compose target consumed by Task 3. The form uses `hx-on:submit` (processed by whichever htmx runtime owns it — the MFE's when standalone, the shell's when composed).

- [ ] **Step 1: Vendor htmx**

```bash
mkdir -p /home/sla/projects/ems_rust/frontend-raw-device
curl -fsSL https://unpkg.com/htmx.org@2.0.4/dist/htmx.min.js \
  -o /home/sla/projects/ems_rust/frontend-raw-device/htmx.min.js
test -s /home/sla/projects/ems_rust/frontend-raw-device/htmx.min.js && echo "htmx vendored"
```

- [ ] **Step 2: Write `index.html`**

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Raw Device verifier</title>
  <script src="htmx.min.js"></script>
  <style>
    :root { font-family: system-ui, sans-serif; }
    #raw-device { max-width: 32rem; margin: 2rem auto; padding: 1.5rem;
      border: 1px solid #d0d5dd; border-radius: 12px; }
    #raw-device h1 { font-size: 1.25rem; margin: 0 0 1rem; }
    #rd-form { display: grid; gap: .75rem; }
    #rd-form label { font-weight: 600; font-size: .875rem; }
    #rd-form input { padding: .5rem .625rem; border: 1px solid #d0d5dd;
      border-radius: 8px; font: inherit; }
    #rd-form button { justify-self: start; padding: .5rem 1rem; border: 0;
      border-radius: 8px; background: #2563eb; color: #fff; font: inherit; cursor: pointer; }
    #rd-result { margin-top: 1rem; font-size: .875rem; color: #475467; min-height: 1.25rem; }
  </style>
</head>
<body>
  <!-- The shell composes THIS div via hx-select="#raw-device"; keep the id stable. -->
  <div id="raw-device">
    <h1>Raw Device verifier</h1>
    <form id="rd-form"
          hx-on:submit="event.preventDefault();
            document.getElementById('rd-result').textContent =
              'Query wiring coming next — daqid: ' + this.daqid.value;">
      <label for="rd-daqid">Daq ID</label>
      <input id="rd-daqid" name="daqid" required autocomplete="off"
             placeholder="e.g. daq:std_json_v1:…" />
      <button type="submit">Verify</button>
    </form>
    <div id="rd-result"></div>
  </div>
</body>
</html>
```

- [ ] **Step 3: Verify the file is well-formed and has the compose anchor**

Run:
```bash
grep -c 'id="raw-device"' /home/sla/projects/ems_rust/frontend-raw-device/index.html
```
Expected: `1`

- [ ] **Step 4: Commit**

```bash
cd /home/sla/projects/ems_rust
git add frontend-raw-device/index.html frontend-raw-device/htmx.min.js
git commit -m "feat(raw-device): standalone htmx MFE portal (daqid form, stubbed submit)"
```

---

### Task 2: RawDevicePortalStack (S3 + CloudFront, daq_dev)

**Files:**
- Create: `infra/daq/data_pipeline/raw_device_portal_stack.go`
- Modify: `infra/daq/data_pipeline/main.go` (register the stack)

**Interfaces:**
- Consumes: `frontend-raw-device/` (Task 1) as the BucketDeployment source.
- Produces: CloudFront distribution serving the MFE over HTTPS; stack output `RawDeviceMfeUrl` = `https://<dist>.cloudfront.net` (consumed by Task 3 as `PUBLIC_RAWDEVICE_MFE_URL`).

- [ ] **Step 1: Write the stack**

`infra/daq/data_pipeline/raw_device_portal_stack.go`:
```go
package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awscloudfront"
	"github.com/aws/aws-cdk-go/awscdk/v2/awscloudfrontorigins"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3deployment"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type RawDevicePortalStackProps struct {
	awscdk.StackProps
	// ShellOrigin is the main app's CloudFront origin, allowed to hx-get this MFE.
	ShellOrigin string
}

// NewRawDevicePortalStack hosts the Raw Device htmx microfrontend: a private S3 bucket
// behind CloudFront (HTTPS/OAC), with a CORS response-headers policy so the main app
// shell can compose it via a cross-origin hx-get + hx-select. Demo → bucket DESTROYs.
func NewRawDevicePortalStack(scope constructs.Construct, id string, props *RawDevicePortalStackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, &props.StackProps)

	bucket := awss3.NewBucket(stack, jsii.String("RawDeviceBucket"), &awss3.BucketProps{
		RemovalPolicy:     awscdk.RemovalPolicy_DESTROY,
		AutoDeleteObjects: jsii.Bool(true),
		BlockPublicAccess: awss3.BlockPublicAccess_BLOCK_ALL(),
	})

	// CORS so the HTTPS shell's cross-origin hx-get is allowed. noHeaders on the client
	// keeps it a simple GET (no preflight), so the response header alone suffices.
	cors := awscloudfront.NewResponseHeadersPolicy(stack, jsii.String("RawDeviceCors"), &awscloudfront.ResponseHeadersPolicyProps{
		CorsBehavior: &awscloudfront.ResponseHeadersCorsBehavior{
			AccessControlAllowOrigins:     jsii.Strings(props.ShellOrigin),
			AccessControlAllowMethods:     jsii.Strings("GET"),
			AccessControlAllowHeaders:     jsii.Strings("*"),
			AccessControlAllowCredentials: jsii.Bool(false),
			OriginOverride:                jsii.Bool(true),
		},
	})

	dist := awscloudfront.NewDistribution(stack, jsii.String("RawDeviceDistribution"), &awscloudfront.DistributionProps{
		DefaultRootObject: jsii.String("index.html"),
		DefaultBehavior: &awscloudfront.BehaviorOptions{
			Origin:                awscloudfrontorigins.S3BucketOrigin_WithOriginAccessControl(bucket, nil),
			ViewerProtocolPolicy:  awscloudfront.ViewerProtocolPolicy_REDIRECT_TO_HTTPS,
			ResponseHeadersPolicy: cors,
		},
	})

	awss3deployment.NewBucketDeployment(stack, jsii.String("DeployRawDevice"), &awss3deployment.BucketDeploymentProps{
		Sources:           &[]awss3deployment.ISource{awss3deployment.Source_Asset(jsii.String("../../../frontend-raw-device"), nil)},
		DestinationBucket: bucket,
		Distribution:      dist,
		DistributionPaths: jsii.Strings("/*"),
	})

	awscdk.NewCfnOutput(stack, jsii.String("RawDeviceMfeUrl"), &awscdk.CfnOutputProps{
		Value:       jsii.String("https://" + *dist.DistributionDomainName()),
		Description: jsii.String("Raw Device MFE base URL — set as PUBLIC_RAWDEVICE_MFE_URL in the shell"),
	})

	return stack
}
```

- [ ] **Step 2: Register in `main.go`**

Add after the `NewQueryRawStack(...)` block in `infra/daq/data_pipeline/main.go`:
```go
	// Raw Device htmx microfrontend (S3 + CloudFront) — composed into the main app shell.
	NewRawDevicePortalStack(app, "RawDevicePortalStack", &RawDevicePortalStackProps{
		StackProps:  awscdk.StackProps{Env: defaultEnv()},
		ShellOrigin: "https://d24beiqs2cj89y.cloudfront.net",
	})
```

- [ ] **Step 3: Compile the CDK app**

Run:
```bash
cd /home/sla/projects/ems_rust/infra/daq/data_pipeline && unset GOROOT && go build ./...
```
Expected: no output (success). Fix any construct API mismatch until it compiles.

- [ ] **Step 4: Diff (must be a brand-new additive stack)**

Run:
```bash
cd /home/sla/projects/ems_rust/infra/daq/data_pipeline
unset GOROOT; export AWS_PROFILE=daq_dev
eval "$(aws configure export-credentials --profile daq_dev --format env)"
export CDK_DEFAULT_ACCOUNT=891377204778 CDK_DEFAULT_REGION=eu-central-1
npx cdk diff RawDevicePortalStack --exclusively -c TableBucketName=measurements 2>&1 | grep -E "^\[[+~-]\]|Number of stacks"
```
Expected: only `[+]` create lines (bucket, distribution, CORS policy, deployment, output); no `[~]`/`[-]` on existing resources.

- [ ] **Step 5: Deploy**

Run (same env as Step 4):
```bash
npx cdk deploy RawDevicePortalStack --exclusively -c TableBucketName=measurements --require-approval never
```
Expected: `✅ RawDevicePortalStack`, and an output line `RawDevicePortalStack.RawDeviceMfeUrl = https://<dist>.cloudfront.net`. Record that URL.

- [ ] **Step 6: Verify the MFE serves over HTTPS with the form + CORS header**

Run (substitute the URL from Step 5):
```bash
URL="https://<dist>.cloudfront.net"
curl -s "$URL/" | grep -c 'id="raw-device"'          # expect 1
curl -s -I -H "Origin: https://d24beiqs2cj89y.cloudfront.net" "$URL/" \
  | grep -i "access-control-allow-origin"             # expect the shell origin
```
Expected: `1`, and an `access-control-allow-origin: https://d24beiqs2cj89y.cloudfront.net` header.

- [ ] **Step 7: Commit**

```bash
cd /home/sla/projects/ems_rust
git add infra/daq/data_pipeline/raw_device_portal_stack.go infra/daq/data_pipeline/main.go
git commit -m "feat(raw-device): S3+CloudFront portal stack in daq_dev (CORS for shell compose)"
```

---

### Task 3: Shell — 3rd Party nav + /rawdevice compose page

**Files:**
- Create: `frontend/src/pages/rawdevice.astro`
- Modify: `frontend/src/components/Navbar.astro` (add "3rd Party" dropdown)
- Modify: `frontend/.env`, `frontend/.env.example` (add `PUBLIC_RAWDEVICE_MFE_URL`)

**Interfaces:**
- Consumes: `RawDeviceMfeUrl` (Task 2) as `PUBLIC_RAWDEVICE_MFE_URL`; the MFE's `#raw-device` div (Task 1) via `hx-select`.
- Produces: the user-facing composition (no downstream consumers).

- [ ] **Step 1: Add the env var**

Append to `frontend/.env.example`:
```
# Raw Device htmx microfrontend (daq_dev account) — the RawDeviceMfeUrl output of
# RawDevicePortalStack, HTTPS, no trailing slash. The /rawdevice page hx-gets it and
# hx-selects #raw-device. Leave blank to show only the "Loading…" placeholder.
PUBLIC_RAWDEVICE_MFE_URL=https://your-dist-id.cloudfront.net
```
Append to `frontend/.env` (substitute the real URL from Task 2 Step 5):
```
# Raw Device microfrontend (daq_dev) — RawDevicePortalStack RawDeviceMfeUrl
PUBLIC_RAWDEVICE_MFE_URL=https://<dist>.cloudfront.net
```

- [ ] **Step 2: Create the compose page `frontend/src/pages/rawdevice.astro`**

```astro
---
import Layout from '../layouts/Layout.astro';

// The Raw Device MFE lives in the daq_dev account; we compose it cross-origin via htmx.
const mfeUrl = import.meta.env.PUBLIC_RAWDEVICE_MFE_URL || "";
---

<Layout title="Raw Device">
  <div class="page-container">
    <div class="view-header"><h1>Raw Device</h1></div>
    <!-- Microfrontend composition: fetch the whole MFE page from its own origin and
         swap in ONLY its #raw-device main div. noHeaders keeps it a CORS-simple GET. -->
    <div id="rd-content"
         hx-get={mfeUrl}
         hx-select="#raw-device"
         hx-swap="innerHTML"
         hx-trigger="load"
         hx-request='{"noHeaders":true}'
         hx-on::response-error="this.innerHTML = 'Raw Device portal unavailable.'">
      Loading Raw Device portal…
    </div>
  </div>
</Layout>
```

- [ ] **Step 3: Add the "3rd Party → Raw Device" nav item**

In `frontend/src/components/Navbar.astro`, add a new top-level `<li>` inside `<ul class="nav-menu">` (mirror the existing dropdown pattern, e.g. the "Resource Insights" block). Insert it before the closing `</ul>` of `nav-menu`:
```astro
        <li class="nav-item nav-dropdown">
          <button class={`nav-link nav-dropdown-indicator ${isParentActive(['/rawdevice']) ? 'active' : ''}`} data-i18n="nav.third_party">3rd Party</button>
          <ul class="nav-dropdown-menu">
            <li><a href="/rawdevice" class={`nav-link ${currentPath === '/rawdevice' ? 'active' : ''}`} data-i18n="nav.raw_device">Raw Device</a></li>
          </ul>
        </li>
```
Match the exact class names / wrapper structure used by the sibling dropdowns in this file (the grep showed `nav-dropdown-indicator` buttons + a nested `<ul>`); copy a sibling's markup and change the labels/paths if the class names differ from the snippet above.

- [ ] **Step 4: Build the shell and verify the URL is baked in**

Run:
```bash
cd /home/sla/projects/ems_rust/frontend && npm run build
grep -oE "https://[a-z0-9]+\.cloudfront\.net" dist/rawdevice/index.html | sort -u
```
Expected: the MFE CloudFront URL appears in the built `/rawdevice` page.

- [ ] **Step 5: Deploy the shell (frontend account)**

Run (needs `aws sso login --profile stel-sb` first if the token is stale):
```bash
cd /home/sla/projects/ems_rust/infra/frontend
unset GOROOT; export AWS_PROFILE=stel-sb
eval "$(aws configure export-credentials --profile stel-sb --format env)"
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1
npx cdk deploy OcamlFrontendStack --require-approval never
```
Expected: `✅ OcamlFrontendStack` (content-only BucketDeployment change).

- [ ] **Step 6: Verify end-to-end**

Run:
```bash
curl -s "https://d24beiqs2cj89y.cloudfront.net/rawdevice/" | grep -oE 'hx-get="https://[^"]+"' | head -1
```
Expected: `hx-get="https://<dist>.cloudfront.net"`.
Then in a browser: open `https://d24beiqs2cj89y.cloudfront.net/rawdevice`, confirm the daqid form composes into the shell (nav + layout intact), and the DevTools Network tab shows one cross-origin `GET` to the MFE URL → `200` with `access-control-allow-origin`. The "3rd Party" dropdown shows "Raw Device".

- [ ] **Step 7: Commit**

```bash
cd /home/sla/projects/ems_rust
git add frontend/src/pages/rawdevice.astro frontend/src/components/Navbar.astro frontend/.env.example
git commit -m "feat(raw-device): compose the MFE into the shell via 3rd Party > Raw Device"
```
(`frontend/.env` is gitignored — not committed.)

---

## Notes for the implementer

- **i18n:** the nav uses `data-i18n` keys with literal fallback text (already present in the elements above). Adding `nav.third_party` / `nav.raw_device` to the i18n dictionaries is optional polish; the literal text renders regardless.
- **htmx availability:** the shell `Layout.astro` already loads htmx, so `/rawdevice` gets it for free; the MFE vendors its own for standalone use.
- **Ordering:** Task 2 must deploy before Task 3 Step 1 (you need the real `RawDeviceMfeUrl`). Task 1 must precede Task 2 (it's the deployment source).
