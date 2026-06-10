# Behavioural Specs (BDD guardrails)

Given / When / Then behaviour specifications for the **DAQ data pipeline**
(`infra/daq/data_pipeline`), written in [Gherkin](https://cucumber.io/docs/gherkin/).

## Why this exists

These files capture the **decisions and acceptance criteria** that are currently
encoded — sometimes implicitly — in the Scala/Python tests and the design specs under
`infra/daq/data_pipeline/docs/superpowers/specs/`. They are extracted into one place so that:

1. **Agents and humans have guardrails.** Before changing pipeline code, read the relevant
   feature file. Each scenario is a rule that must keep holding; if a change would violate
   one, that is a deliberate decision that needs the spec updated in the same change — not a
   silent regression.
2. **They can grow into real tests.** Gherkin is executable. The scenarios are written to map
   onto the existing `ResampleFunction`, `MeterEnrichmentFunction`, `HierarchyPathParser`,
   the Glue `measurements_aggregate` rollup, etc. A future step can wire a Cucumber-JVM (Scala)
   and `behave`/`pytest-bdd` (Python) runner to these `.feature` files and bind the steps to
   the same pure functions the unit tests already call.

## Status: guardrails, not yet wired to a runner

Today these are **documentation/guardrails** — no step definitions are bound, so `sbt test`
does not run them. The source of truth for executable behaviour is still:

- Scala: `infra/daq/data_pipeline/flink_app_scala/src/test/scala/...`
- Python: `infra/daq/data_pipeline/glue/tests/...`

Each scenario notes its backing test in a `# source:` comment so the two never silently drift.

## Map of features

| File | Covers | Backing tests / spec |
|---|---|---|
| `data_pipeline/parsing_and_validation.feature` | Device JSON → `SensorRecord`, validation filtering, parse-error routing | `*ProcessorSpec`, `ProcessUtilsSpec`, `EnrichmentMiniClusterSpec` |
| `data_pipeline/meter_enrichment.feature` | `SensorRecord` + `meter-identity` → `EnrichedRecord`, dead-letter | `MeterEnrichmentFunctionSpec`, `HierarchyPathParserSpec` |
| `data_pipeline/meter_identity_sync.feature` | DDB bootstrap + CDC stream of meter identities | `DdbStreamDeserializerSpec`, `DdbBootstrapLoaderSpec` |
| `data_pipeline/resampling.feature` | Gauge linear interpolation & counter time-proportional binning | `ResampleFunctionSpec`, `ResampleHarnessSpec`, resampling-rules spec |
| `data_pipeline/out_of_order_and_late_arrival.feature` | Event-time buffering, watermarks, late-arrival routing | `EnrichmentPipelineSpec`, `ResampleHarnessSpec` |
| `data_pipeline/anomaly_and_side_outputs.feature` | Counter anomalies, side-output tags, error-stream record shape | `ResampleHarnessSpec`, `SideOutputTagsSpec` |
| `data_pipeline/flink_glue_parity.feature` | Streaming/batch output-parity contract | resampling-rules spec ("output parity invariant") |
| `data_pipeline/measurements_rollup.feature` | DynamoDB rollup view: keys, dedup, idempotency, TTL | `glue/tests/test_helpers.py`, `glue/tests/test_rollups.py`, rollup spec |
| `data_pipeline/deployment_guardrails.feature` | Non-destructive deploy rules (Iceberg/Flink state) | resampling-rules spec, `CLAUDE.md` deployment policy |
| `hierarchy/schema_type_graph.feature` | v2 type-graph schema: variable-depth types, DAG validation, derived levels | `crates/model` schema/hierarchy/sensors tests, 2026-06-10 spec |

## Conventions

- One `Feature` per behavioural area; `Background` holds shared context.
- `Scenario Outline` + `Examples` for table-driven rules (bin enumeration, hierarchy paths).
- A `# source:` comment links each scenario to the authoritative test or spec section.
- Times are **UTC**. Bin boundaries are multiples of the bin size from the UTC epoch.
