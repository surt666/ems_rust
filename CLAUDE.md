# CLAUDE.md

Project-level guidance for working in `ems_ocaml`. Read before building/deploying.

## Branching & pushing

Work directly on `main`. Don't spin up feature/temporary branches by default — `main`
is the single source of truth and production deploys from it.

**Do not `git push`.** Commit to `main` locally, but leave pushing to the remote to the
user — they decide when `main` is published. Never run `git push` (or delete/replace remote
refs) unless the user explicitly asks for it in the moment.

## Deployment policy

The system spans **two AWS accounts**, both in **`eu-central-1`**. Use SSO admin creds for
the relevant account. **Always `unset GOROOT` before any `cdk` command** (the Go-based CDK
breaks with a stale nix `GOROOT` in the shell). **Always `cdk diff` before `cdk deploy`** and
confirm the changeset is non-destructive (no DynamoDB table / Kinesis stream / Flink app
*replacement* — only `[~]` updates).

**cdk credential recipe (when `cdk` fails with `Unable to parse environment specification
"aws:///eu-central-1"` or `no credentials have been configured`).** The SSO *role* creds
expired and the Go CDK's bundled SDK can't refresh them from the SSO token (the `aws` CLI
can, which is why `aws sts get-caller-identity` still works). Export fresh temporary creds
straight into the cdk process, in the same shell call as the `cdk` command:

```bash
unset GOROOT
export AWS_PROFILE=stel-sb                                   # or daq_dev
eval "$(aws configure export-credentials --profile stel-sb --format env)"
export CDK_DEFAULT_ACCOUNT=339712745226 CDK_DEFAULT_REGION=eu-central-1   # 891377204778 for daq
cdk diff  <Stack>
cdk deploy <Stack> --require-approval never
```

If the SSO *token* itself is expired, the user must first run `aws sso login --profile <p>`
via the `! ` prefix (interactive; I can't run it).

**Shell-chaining gotcha (the Bash tool runs with `set -e`-style semantics).** Do NOT lead a
chained command with `pkill`/`pgrep` — or any command whose non-zero exit is normal — because
that non-zero exit aborts the rest of the chain, so the real work (build / commit / deploy)
silently never runs. Put cleanup in its own call, or neutralise it: `pkill -f 'astro preview' 2>/dev/null || true`.

| Account | Id | Owns |
|---|---|---|
| Hierarchy / backend | `339712745226` | `rust-lambda-hierarchy` API lambda (arm64), `hierarchy_new` table, the cross-account bridge lambda, the **frontend** (S3 + CloudFront) |
| DAQ / pipeline | `891377204778` | Flink (MSF) app, Glue late-recomputation, `sensor-identity` table, S3 Iceberg tables, `measurements_aggregate` + the aggregations Lambda |

### Stack 1 — Hierarchy service (account `339712745226`)

Builds a Rust Lambda (arm64/Graviton) with cargo-lambda, then a Go CDK stack. (The service was
ported from OCaml to Rust in 2026-06; the OCaml service + the Go cognito-sync stream lambda are
gone — the Rust lambda does Cognito provisioning synchronously in the request path.)

```bash
# 1. Build the arm64 bootstrap (-> target/lambda/hierarchy/bootstrap, which
#    infra/hierarchy/app.go references via Code.FromAsset("../../target/lambda/hierarchy")).
cargo lambda build --release --arm64 -p hierarchy

# 2. Deploy (Go CDK). Stack: OcamlHierarchyStack (name kept so the RETAIN table isn't replaced).
cd infra/hierarchy
unset GOROOT
cdk diff  OcamlHierarchyStack          # confirm only Lambda Code [~] updates
cdk deploy OcamlHierarchyStack --require-approval never
```

- Updates two lambdas: `rust-lambda-hierarchy` (the API; arm64, `provided.al2023`, synchronous
  Cognito create/delete with rollback + generated permanent password) and `sensor-identity-bridge`
  (the cross-account bridge; its Python is inlined in `infra/hierarchy/app.go`).
- The DynamoDB table is **not** touched by a normal deploy (verify in `cdk diff`).
- The frontend reaches the API via CloudFront, whose origin reads SSM `/api/rust-hierarchy-api-url`
  (set by this stack to the Rust HTTP API). Roll back by pointing `infra/frontend/frontend.go` at
  `/api/ocaml-hierarchy-api-url` — but note the OCaml lambda/API no longer exist, so a real rollback
  means restoring the OCaml resources from git history.
- Verify after: `aws lambda get-function-configuration --function-name rust-lambda-hierarchy`
  (`State=Active`, `LastUpdateStatus=Successful`) and a read against the Rust API
  (`RustApiUrl` output, currently `https://xbvb3nzp1h.execute-api.eu-central-1.amazonaws.com`) or
  the live frontend `https://d24beiqs2cj89y.cloudfront.net` (e.g. `GET /hierarchy/query/profiles`).

### Stack 2 — Data pipeline (account `891377204778`)

Builds the Flink fat JAR with sbt, then Go CDK. The canonical path is `build.sh`:

```bash
cd infra/daq/data_pipeline
# build.sh does: (cd flink_app_scala && sbt clean assembly) then the cdk deploy below.
# To run it manually / add a cdk diff gate:
( cd flink_app_scala && sbt clean assembly )      # -> target/scala-3.3.4/flink-app-scala-0.1.0.jar
unset GOROOT
npx cdk diff DaqPipelineStack LateRecomputationStack MeasurementsAggregateStack \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
npx cdk deploy DaqPipelineStack LateRecomputationStack OcamlBridgeWriterRoleStack MeasurementsAggregateStack \
  --require-approval never \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
```

- `DaqPipelineStack` updates the MSF Flink application (`flink-iceberg-processor`) **in place**
  — it snapshots and restarts (brief processing pause), restoring from snapshot. This only works
  while the operator `uid` and keyed-state descriptor names are unchanged (currently `"resample"`
  / `"resample-*"`); if those change, the restore fails and you must start fresh
  (`SKIP_RESTORE_FROM_SNAPSHOT`, state loss) — as the 2026-06-07 de-binning rename deliberately did.
- `RUN_NR` (a timestamp) is what forces the `DeployFlinkApp` custom resource to push a new JAR.
- `LateRecomputationStack` swaps the Glue script (`late-data-recomputation` job).
  `OcamlBridgeWriterRoleStack` (the IAM role the bridge assumes) is normally unchanged.
  `S3TablesStack` owns the Iceberg tables; deploying it with **changed columns replaces the table**
  (data loss) — that's how `logical_data` (columns `resample_value/resample_method/resample_timestamp`)
  gets recreated.
- `MeasurementsAggregateStack` owns the `measurements_aggregate` DynamoDB table (on-demand, TTL,
  `RETAIN`) + the hourly `measurements-aggregate` Glue job that rolls up `logical_data`
  counter consumption per node/energy_type/purpose/hour|day. `-c LookbackDays=N` sets the
  day-aligned recompute window (default `1` = today + yesterday). Spec/plan:
  `infra/daq/data_pipeline/docs/superpowers/specs/2026-06-07-measurements-rollup-view-design.md`
  and `docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md`.
  **The job holds no formula semantics** — it is one join and one grouped sum over the
  materialised coefficient matrix (`crates/model::logic::formulas::flatten`, written into
  `hierarchy_new`'s `W#HN2#<id>` gsi1 partition by the hierarchy service). Ancestry, both
  defaults and the `Unallocated` rows are all baked into the matrix; `ancestor_keys` is gone,
  and `test_the_job_holds_no_formula_logic` asserts (over the AST) that none of it creeps back.
  The job reads that matrix cross-account by assuming
  `arn:aws:iam::339712745226:role/HierarchyReaderRole`, which trusts the DAQ Glue role by its
  pinned name `MeasurementsAggregateGlueRole` — **do not let either name become CFN-generated**,
  or the trust breaks silently. Sort key is
  `<node_path>#<energy_type>#<purpose>#<gran>#<bucket>` (bucket last, so a series is one
  `BETWEEN`); `gsi1pk` is `HN2#<id>#<dimension>#<purpose>`.
  It also hosts **one Rust/arm64 lambda** (`measurements-aggregations-api`, `crates/services/aggregations`)
  behind an **API Gateway HTTP API** (`AggregationsHttpApi`, output `AggregationsApiUrl`). Swapped from a
  Lambda Function URL → HTTP API 2026-06-27 (access logs / throttling / WAF / future Cognito JWT
  authorizer; mirrors the hierarchy service). **CQRS query side only** (read-only — meter data is written
  by the Flink/Glue pipeline, not here), so it follows the hierarchy `GET /query/{action}` convention with
  **three explicit GET routes** (not a `/{proxy+}` catch-all — the gateway owns the surface and answers the
  OPTIONS CORS preflight; a catch-all `ANY` would hand OPTIONS to the lambda → 404 → CORS failure):
  `GET /meterdata/query/{action}` where `action` ∈ {`get_aggregations` (JSON, Resource-Insights chart,
  reads DynamoDB), `get_measurements` (Datatilegnelse, reads `all.raw_data` via **Amazon Athena** —
  `start → poll → get_query_results`, dedup `GROUP BY` + `max_by(value, ingested_time)` server-side,
  result-reuse caching on; ~4-6s vs ~12s for the abandoned iceberg-rust-direct path)}, plus
  `GET /meterdata/openapi.json` (OpenAPI 3.1 spec) and `GET /meterdata/docs` (self-hosted Swagger UI).
  Both query actions accept **`?format=html|json`** (get_measurements defaults html for HTMX,
  get_aggregations defaults json); representations + the typed error envelope live in the shared
  **`crates/api`** crate (`ApiResponse`/`ApiError`, utoipa `ToSchema`). The lambda emits **no CORS headers**
  (`api::Cors::None`) — the HTTP API `CorsPreflight` adds them. Kept to one function to limit
  Datadog-instrumented lambdas. Build with `cargo lambda build --release --arm64 -p aggregations` before
  `cdk deploy` (the stack reads `target/lambda/aggregations` via `Code.FromAsset`). The frontend uses
  `PUBLIC_AGG_API_BASE_URL` (the `AggregationsApiUrl` base, no trailing slash) for all routes. The
  `get_measurements` route needs **Athena** IAM
  (workgroup `daq-workgroup`, Glue catalog read on `s3tablescatalog`, R/W on
  `daq-athena-query-results-<acct>-<region>`, `lakeformation:GetDataAccess`) + Lake Formation SELECT
  on `raw_data` — all wired in the stack.
- Verify the live Flink JAR is the one you built:
  `aws kinesisanalyticsv2 describe-application --application-name flink-iceberg-processor`
  → download the `FileKey` jar from `s3://flink-code-891377204778-eu-central-1/...` and
  `unzip -l | grep ResampleFunction`. (Don't trust the sbt "Jar hash" line vs the S3 key sha256 —
  jar packaging has non-deterministic bytes, so they legitimately differ; check the class instead.)

#### Gotchas when renaming Iceberg columns / Flink operator state (learned 2026-06-07)

- **`AWS::S3Tables::Table` cannot be replaced in place.** Changing a column forces a CFN replace,
  which does create-before-delete → fails with `409 "table with an identical name already exists"`.
  **If you are RENAMING the table, none of this applies** — a different name means no collision,
  so one deploy creates the new table and leaves the old one behind (see below). The dance is only
  needed to change columns *under an unchanged name*.
- **The old two-step recipe no longer works as written** (corrected 2026-07-28). It said: remove
  the table resource, `cdk deploy` (CFN deletes it, clearing data), then restore with new columns
  and deploy again. Both tables gained `RemovalPolicy: RETAIN` with `ApplyToUpdateReplacePolicy`
  in `2d21c6c` (2026-07-18), so removing the resource now **retains** the table — step 1 silently
  clears nothing and step 2 still hits the 409. Relax the removal policy first, or rename.
- **`RETAIN` means renames orphan, they do not delete.** Renaming `logical_meter_data` →
  `logical_data` reported `DELETE_SKIPPED` and left the old table in the bucket; likewise
  `meter-identity` → `sensor-identity` in DynamoDB. Clean the orphan up by hand once verified,
  or it lingers with its storage and (for DynamoDB) its PITR bill.
- **Renaming a DynamoDB table breaks cross-stack exports.** `DaqPipelineStack` exports the table
  ARN/name/stream to `LateRecomputationStack` and `OcamlBridgeWriterRoleStack`; CloudFormation
  refuses to delete an export in use, and the consumers cannot move first because the new export
  does not exist yet. Break it with a transitional deploy that re-declares the legacy export
  **names** explicitly (exports are keyed by name, so a name present in both templates is neither
  deleted nor updated), move the consumers, then drop the shim. Full recipe in
  `docs/superpowers/plans/2026-07-28-node-formula-rollup.md` Task 9 Step 6.
- **Renaming the Flink operator `uid`/keyed-state forces a non-restorable snapshot.** On the next
  Flink deploy the running app keeps writing the old schema and goes into failure, so MSF can't
  snapshot it and the stack sticks in `UPDATE_ROLLBACK_FAILED`. Recover with:
  `aws kinesisanalyticsv2 stop-application --force` → `aws cloudformation continue-update-rollback
  --stack-name DaqPipelineStack --resources-to-skip FlinkApplication` → forward `cdk deploy` (app
  is stopped, so the update needs no snapshot) → then start it.
- **Starting after a uid/state rename:** the old state can't map to the renamed operators, so you
  must drop it. `AllowNonRestoredState` lives under **`FlinkRunConfiguration`** in the
  `start-application` run-config (NOT `ApplicationRestoreConfiguration`):
  `--run-configuration '{"FlinkRunConfiguration":{"AllowNonRestoredState":true},"ApplicationRestoreConfiguration":{"ApplicationRestoreType":"RESTORE_FROM_LATEST_SNAPSHOT"}}'`.
  Prefer `RESTORE_FROM_LATEST_SNAPSHOT` over `SKIP_RESTORE_FROM_SNAPSHOT` — the source is
  `TRIM_HORIZON`, so SKIP reprocesses the full Kinesis retention (24h) and **duplicates `raw_data`**.

### Stack 3 — Frontend (account `339712745226`)

Astro static site → S3 + CloudFront (`OcamlFrontendStack`, in `infra/frontend`). CloudFront proxies
`/command`, `/query/*`, `/hierarchy/*` to the hierarchy API, so those frontend calls are **relative**
(`PUBLIC_API_BASE_URL` stays **empty** by design). Cross-account endpoints (e.g. the daq
aggregations HTTP API) need an **absolute** URL baked in at build time.

```bash
# 1. Set frontend/.env — Astro inlines PUBLIC_* vars into the build:
#    PUBLIC_API_BASE_URL=                     # EMPTY — CloudFront proxies the hierarchy API
#    PUBLIC_AGG_API_BASE_URL=https://<api-id>.execute-api.eu-central-1.amazonaws.com   # AggregationsApiUrl, no trailing slash
#    PUBLIC_USER_POOL_ID / PUBLIC_USER_POOL_CLIENT_ID   (Cognito)
cd frontend && npm run build          # -> frontend/dist (env baked in; REBUILD after any .env change)

# 2. Deploy (Go CDK). Uploads dist to S3 + invalidates CloudFront /*.
cd ../infra/frontend
unset GOROOT
cdk deploy OcamlFrontendStack --require-approval never
```

- Content-only deploy: the `DeployFrontend` BucketDeployment asset changes; the S3 bucket (RETAIN)
  and CloudFront distribution are unchanged.
- Live URL = the `DistributionDomainName` output (currently `https://d24beiqs2cj89y.cloudfront.net`).
- **Rebuild before deploying** — `PUBLIC_*` values are compiled into the static JS, so a `.env`
  change only takes effect after `npm run build`.
- Verify a value is baked in: `curl -s https://<cf-domain>/_astro/AggregationChartWrapper.*.js | grep execute-api`.

### Cross-account ordering (important)

The bridge writes the per-sensor resample interval to `sensor-identity` as **`resample_minutes`**,
and the Flink/Glue pipeline reads **only** `resample_minutes`. There is **no `binning` fallback
anymore** (removed 2026-06-07), so the two sides are a strict contract:

- The bridge (hierarchy account) and the pipeline (daq account) must both be on the post-rename
  code. They are; keep them that way — a sensor whose `sensor-identity` row lacks `resample_minutes`
  simply gets no resampling (raw passthrough).
- Because there's no fallback, any future rename of this attribute must deploy both sides together
  and re-write the affected `sensor-identity` rows.

See `memory/cross_account_bridge.md` for the full field contract.

## Build & test (local)

- Hierarchy service (Rust): `cargo test` from the repo root (`cargo test -p model` + `-p hierarchy`
  + `-p aggregations` + `-p api`); `cargo build` + `cargo clippy` should be warning-free. Model layer
  is `crates/model`, the lambdas are `crates/services/hierarchy` + `crates/services/aggregations`, and
  `crates/api` is the shared HTTP layer (typed `ApiResponse`/`ApiError`, `?format` negotiation, utoipa
  schema). Dump either lambda's OpenAPI spec locally with `cargo run -p <aggregations|hierarchy> -- --openapi`.
- Flink app: `cd infra/daq/data_pipeline/flink_app_scala && sbt test`.
