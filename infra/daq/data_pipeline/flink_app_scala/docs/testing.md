# Running Tests

Four test layers provide fast local feedback and confidence against real AWS infrastructure.

## Unit Tests (~5s)

Standard ScalaTest specs covering individual functions and classes in isolation. No Flink runtime needed.

```bash
# All unit tests
sbt "testOnly com.enity.flink.processors.* com.enity.flink.enrichment.* com.enity.flink.utils.*"

# Individual processor (EMU, FLOWIQ, Bluemetering, MIVO, MC603, Std, Pulse, EDIEL, GWB143)
sbt "testOnly *EmuProcessorSpec"
sbt "testOnly *StdProcessorSpec"

# Enrichment logic (counter delta, meter enrichment, hierarchy path parsing, side output tags, DDB deserializer, DDB bootstrap)
sbt "testOnly *CounterDeltaFunctionSpec"
sbt "testOnly *MeterEnrichmentFunctionSpec"
sbt "testOnly *HierarchyPathParserSpec"

# Utilities
sbt "testOnly *ProcessUtilsSpec"
```

Covers: all parser format correctness, counter delta computation, enrichment field mapping, hierarchy path parsing, side output tag definitions, DynamoDB stream deserialization, DynamoDB bootstrap loading, and utility functions.

## Harness Tests (~5s)

Unit-level tests for individual operator logic (delta computation, enrichment lookup, late arrival handling). Uses Flink's `KeyedOneInputStreamOperatorTestHarness` with manual watermark advancement for precise timer control.

```bash
sbt "testOnly *Harness*"
```

Covers: gauge pass-through, counter delta, negative delta anomaly, out-of-order correction, late arrival with/without predecessor, dead letter routing.

## Layer 2: Mini-cluster Tests (~30s)

Full Flink job graph running in an embedded MiniCluster. Uses `env.fromCollection()` as source and `CollectSink` instead of Iceberg/Kinesis. Bounded execution ensures deterministic completion.

```bash
# All mini-cluster tests
sbt "testOnly com.enity.flink.scenarios.*"

# Parser scenarios only (EMU, FLOWIQ, Bluemetering, MIVO, MC603, Std, Pulse, EDIEL, GWB143)
sbt "testOnly *ParserMiniCluster*"

# Enrichment scenarios only (counter correction, CDC mapping, multi-meter, parse errors, mixed batch)
sbt "testOnly *EnrichmentMiniCluster*"
```

Covers: 15 scenarios including all parser formats, counter out-of-order correction, mapping updates via CDC, multi-meter routing, parse error routing, and mixed gauge+counter batches.

## Layer 3: Smoke Tests (~7min)

End-to-end tests against real AWS infrastructure. Injects data into Kinesis, waits for Flink processing + Iceberg commit, then queries Athena to verify results.

**Prerequisites:**
- Active AWS credentials for `eu-central-1` (IAM access to Kinesis, DynamoDB, Athena, S3 Tables)
- Running Flink application (KDA) consuming from the input stream
- Python with `uv` installed

```bash
cd scripts/smoke_test

# Run all smoke tests
uv run test_smoke.py --input-stream <DAQ_STREAM_NAME> --error-stream <ERROR_STREAM_NAME>

# With verbose output (shows checkpoint wait progress)
uv run test_smoke.py --input-stream <DAQ_STREAM_NAME> --error-stream <ERROR_STREAM_NAME> --verbose
```

Covers: gauge end-to-end, counter delta end-to-end, raw record write, dead letter routing.

**Data isolation:** All test data uses a `test_smoke_` prefix with a unique run ID. DynamoDB mappings are deleted after each scenario. Iceberg rows are cleaned up via Athena `DELETE FROM` after verification.

**Reports:** Console output shows pass/fail per scenario. A JSON report is saved to `target/test-reports/`.

## Running All Tests

```bash
# All unit + harness + mini-cluster tests (everything except smoke)
sbt test
```
