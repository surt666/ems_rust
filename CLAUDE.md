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

| Account | Id | Owns |
|---|---|---|
| Hierarchy / backend | `339712745226` | `ocaml-lambda-hierarchy` API lambda, `hierarchy_new` table, the cross-account bridge lambda |
| DAQ / pipeline | `891377204778` | Flink (MSF) app, Glue late-recomputation, `meter-identity` table, S3 Iceberg tables |

### Stack 1 — Hierarchy service (account `339712745226`)

Builds an OCaml Lambda (static, x86_64) via Docker, then a Go CDK stack.

```bash
# 1. Build the deploy zip (Docker; produces services/hierarchy/ocaml-lambda-hierarchy.zip,
#    which infra/hierarchy/app.go references as ../../services/hierarchy/...zip)
cd services/hierarchy && make build

# 2. Deploy (Go CDK). Stack: OcamlHierarchyStack.
cd ../../infra/hierarchy
unset GOROOT
cdk diff  OcamlHierarchyStack          # confirm only Lambda Code [~] updates
cdk deploy OcamlHierarchyStack --require-approval never
```

- Updates two lambdas: `ocaml-lambda-hierarchy` (the API) and `ocaml-meter-identity-bridge`
  (the cross-account bridge; its Python is inlined in `infra/hierarchy/app.go`).
- The DynamoDB table is **not** touched by a normal deploy (verify in `cdk diff`).
- Verify after: `aws lambda get-function-configuration --function-name ocaml-lambda-hierarchy`
  (`State=Active`, `LastUpdateStatus=Successful`) and a read against the public API
  `https://doztw28ic6.execute-api.eu-central-1.amazonaws.com` (e.g. `GET /query/list_users`).

### Stack 2 — Data pipeline (account `891377204778`)

Builds the Flink fat JAR with sbt, then Go CDK. The canonical path is `build.sh`:

```bash
cd infra/daq/data_pipeline
# build.sh does: (cd flink_app_scala && sbt clean assembly) then the cdk deploy below.
# To run it manually / add a cdk diff gate:
( cd flink_app_scala && sbt clean assembly )      # -> target/scala-3.3.4/flink-app-scala-0.1.0.jar
unset GOROOT
npx cdk diff DaqPipelineStack LateRecomputationStack \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements
npx cdk deploy DaqPipelineStack LateRecomputationStack OcamlBridgeWriterRoleStack \
  --require-approval never \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements
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
  (data loss) — that's how `logical_meter_data` (columns `resample_value/resample_method/resample_timestamp`)
  gets recreated.
- Verify the live Flink JAR is the one you built:
  `aws kinesisanalyticsv2 describe-application --application-name flink-iceberg-processor`
  → download the `FileKey` jar from `s3://flink-code-891377204778-eu-central-1/...` and
  `unzip -l | grep ResampleFunction`. (Don't trust the sbt "Jar hash" line vs the S3 key sha256 —
  jar packaging has non-deterministic bytes, so they legitimately differ; check the class instead.)

### Cross-account ordering (important)

The bridge writes the per-sensor resample interval to `meter-identity` as **`resample_minutes`**,
and the Flink/Glue pipeline reads **only** `resample_minutes`. There is **no `binning` fallback
anymore** (removed 2026-06-07), so the two sides are a strict contract:

- The bridge (hierarchy account) and the pipeline (daq account) must both be on the post-rename
  code. They are; keep them that way — a sensor whose `meter-identity` row lacks `resample_minutes`
  simply gets no resampling (raw passthrough).
- Because there's no fallback, any future rename of this attribute must deploy both sides together
  and re-write the affected `meter-identity` rows.

See `memory/cross_account_bridge.md` for the full field contract.

## Build & test (local)

- Hierarchy service: `cd services/hierarchy && dune build && dune runtest` (Alcotest;
  warnings are errors).
- Flink app: `cd infra/daq/data_pipeline/flink_app_scala && sbt test`.
