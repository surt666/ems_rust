# Scenario Test Automation Design

**Date:** 2026-04-05
**Status:** Approved

## Overview

Automated test suite covering all 17 data ingestion scenarios documented in `data-ingestion-scenarios.html`. Three test layers provide fast feedback locally and confidence against real AWS infrastructure.

## Test Layers

| Layer | Runtime | What it tests | Trigger |
|-------|---------|---------------|---------|
| **Harness** | ~5s | Individual operator logic (delta computation, enrichment lookup, parsing) | `sbt test` |
| **Mini-cluster** | ~30s | Full Flink chain with watermarks, timers, side outputs, broadcast state | `sbt "testOnly *MiniCluster*"` |
| **Smoke** | ~5min | Real Kinesis → Flink → Iceberg → Athena round-trip | Manual / CI on deploy |

## Layer 1: Harness Tests

Unit-level tests using direct function calls or Flink's `KeyedOneInputStreamOperatorTestHarness` / `BroadcastOperatorTestHarness`. Manual watermark advancement for precise timer control.

### Scenarios Covered (7 tests)

| # | Scenario | Test approach |
|---|----------|---------------|
| 1 | Gauge pass-through | Call `CounterDeltaFunction.computeDelta` with `meterType="gauge"` |
| 2 | Counter delta (ordered pair) | Harness: feed two ordered counter records, advance watermark, assert delta |
| 3 | Negative delta → ANOMALY | Harness: feed decreasing counter, assert ANOMALY side output |
| 4 | Out-of-order within watermark | Harness: feed records out of order, advance watermark past both, assert corrected delta |
| 5 | Late arrival (predecessor in buffer) | Harness: advance watermark past record timestamp but within 6h retention, assert delta emitted |
| 6 | Late arrival (predecessor purged) | Harness: advance watermark + 7h, assert LATE_ARRIVAL side output |
| 7 | Dead letter (no mapping) | Broadcast harness: feed SensorRecord with unknown daqId, assert DEAD_LETTER side output |

### Harness Pattern

```scala
class CounterDeltaHarnessSpec extends AnyFlatSpec:
  "out-of-order records" should "produce corrected deltas" in:
    val harness = new KeyedOneInputStreamOperatorTestHarness(
      new CounterDeltaFunction(bufferRetentionMs = 6 * 3600 * 1000L),
      (r: (EnrichedRecord, String)) => r._1.meterId,
      TypeInformation.of(classOf[String])
    )
    harness.open()

    // Feed record at t=200, then t=100 (out of order)
    harness.processElement(counterRecord(ts = 200, value = 50.0), 200)
    harness.processElement(counterRecord(ts = 100, value = 30.0), 100)

    // Advance watermark past both
    harness.processWatermark(300)

    val output = harness.extractOutputValues()
    // Expect corrected delta: 50.0 - 30.0 = 20.0
    assert(output.asScala.last.value == 20.0)
    harness.close()
```

## Layer 2: Mini-cluster Tests

Full Flink job graph running in `MiniClusterWithClientResource`. Uses `env.fromCollection()` as source, `CollectSink` instead of Iceberg, bounded execution for deterministic completion.

### Shared Job Builder

```scala
object ScenarioTestHelper:
  case class ScenarioResult(
    enrichedRecords: List[EnrichedRecord],
    sideOutputs: Map[String, List[ErrorRecord]]  // tag name → errors
  )

  def buildAndRun(
    sensorRecords: List[SensorRecord],
    mappings: List[MeterMapping],
    config: ScenarioConfig = ScenarioConfig()
  ): ScenarioResult
```

The helper wires the same operator chain as `Main.scala` but replaces:
- Kinesis source → `env.fromCollection(sensorRecords)`
- DDB CDC source → `env.fromCollection(mappings)` broadcast
- Iceberg sink → `CollectSink[EnrichedRecord]`
- Error Kinesis sink → `CollectSink[ErrorRecord]` per side output tag

Bounded sources ensure the job terminates after processing all records. Watermarks are assigned identically to production (BoundedOutOfOrderness).

### Scenarios Covered (15 tests)

| # | Scenario | Key assertions |
|---|----------|----------------|
| 1 | EMU gauge end-to-end | Parse EMU JSON → enriched record with correct value, unit, hierarchy IDs |
| 2 | EMU counter end-to-end | Parse → delta computation → enriched record with delta value |
| 3 | FLOWIQ water meter | Parse MBus frames → multiple SensorRecords per message |
| 4 | Bluemetering multi-register | Parse multi-register payload → one SensorRecord per register |
| 5 | MIVO multi-meter | Parse MIVO JSON → one SensorRecord per sub-meter |
| 6 | MC603 heat meter | Parse MC603 payload → energy + volume records |
| 7 | StdProcessor JSON/JSONL | Parse both `std_json_v1` and `std_jsonl_v1` formats |
| 8 | Pulse counter | Parse Adeunis pulse payload → counter record with cumulative value |
| 9 | EDIEL energy data | Parse EDIEL format → gauge records |
| 10 | GWB143 gateway | Parse GWB143 JSON → SensorRecords |
| 11 | Counter out-of-order correction | Two counter records out of order → corrected delta after watermark |
| 12 | Mapping update via CDC | Initial mapping → records enriched → CDC updates mapping → new records use updated mapping |
| 13 | Multiple meters same DAQ | Two mappings for different logical meters on same daqId → records routed correctly |
| 14 | Parse error routing | Malformed JSON → PARSE_ERROR side output, no enriched records |
| 15 | Mixed gauge + counter batch | Batch of interleaved gauges and counters → gauges pass through, counters get deltas |

### Mini-cluster Pattern

```scala
class EmuGaugeEndToEndSpec extends AnyFlatSpec with MiniClusterTest:
  "EMU gauge message" should "flow through to enriched output" in:
    val emuJson = """{"schematype":"emu_profes_v1","daq_id":"emu-001",...}"""
    val mapping = MeterMapping(daqId = "emu-001", meterId = "meter-001",
      meterType = "gauge", unit = "kWh", ...)

    val result = ScenarioTestHelper.buildAndRun(
      sensorRecords = List(parseSensorRecord(emuJson)),
      mappings = List(mapping)
    )

    assert(result.enrichedRecords.size == 1)
    assert(result.enrichedRecords.head.meterId == "meter-001")
    assert(result.enrichedRecords.head.unit == "kWh")
    assert(result.sideOutputs.values.flatten.isEmpty)
```

## Layer 3: Smoke Tests

Python scripts that inject real data into Kinesis, wait for Flink processing + Iceberg commit, then query Athena to verify results. Run against shared production infrastructure with test-prefixed data for isolation.

### Test Data Isolation

- **Kinesis**: Inject test records with `daq_id` prefixed `test_smoke_` (e.g., `test_smoke_emu_001`)
- **DynamoDB meter-identity**: Insert test mappings with `test_smoke_` prefixed `daqId` and `meterId`
- **Iceberg tables**: Test records land in the same tables but are identifiable by the `test_smoke_` prefix on `daq_id` / `meter_id`
- **Cleanup**: Delete DDB test entries after verification. Iceberg records are append-only and negligible in size; no cleanup needed.

### Scenarios Covered (4 smoke tests)

| # | Scenario | Verification |
|---|----------|-------------|
| 1 | Gauge end-to-end | Athena: `SELECT * FROM nested_meter_readings_cdk2 WHERE meter_id = 'test_smoke_gauge_001'` returns record with correct value |
| 2 | Counter delta end-to-end | Athena: two counter records → query returns delta value with latest `ingested_time` |
| 3 | Raw record write | Athena: `SELECT * FROM raw_cdk WHERE daq_id = 'test_smoke_raw_001'` returns raw record |
| 4 | Dead letter routing | Inject record with unmapped daqId → verify it appears in error Kinesis stream (read with `get-records`) |

### Smoke Test Runner

```python
# test_smoke.py
class SmokeTestRunner:
    def __init__(self, region="eu-central-1"):
        self.kinesis = boto3.client("kinesis", region_name=region)
        self.athena = boto3.client("athena", region_name=region)
        self.dynamodb = boto3.resource("dynamodb", region_name=region)

    def run_all(self) -> SmokeReport:
        results = []
        for scenario in SMOKE_SCENARIOS:
            result = self._run_scenario(scenario)
            results.append(result)
            self._cleanup(scenario)
        return SmokeReport(results)

    def _run_scenario(self, scenario):
        # 1. Insert DDB mapping (if needed)
        # 2. Put records to Kinesis
        # 3. Wait for Flink checkpoint cycle (~6 min)
        # 4. Query Athena with retries
        # 5. Assert expected values
```

## Report Format

All three layers produce a unified report format, printed to stdout and saved as JSON.

### Console Output (pass)

```
=== Scenario Test Report ===
Layer: mini-cluster | 15 scenarios | 14 passed | 1 failed | 32.4s

  [PASS] EMU gauge end-to-end                          (1.2s)
  [PASS] EMU counter end-to-end                        (1.8s)
  ...
  [FAIL] Counter out-of-order correction               (2.1s)
  ...

--- FAILURE DETAILS ---

Counter out-of-order correction:
  Expected: enrichedRecords contains record with value=20.0
  Actual:   enrichedRecords = [EnrichedRecord(meterId=m1, value=50.0, ...)]
  Side outputs: {ANOMALY: [], DEAD_LETTER: [], LATE_ARRIVAL: [], PARSE_ERROR: []}
  Input records:
    SensorRecord(daqId=d1, timestamp=2026-01-01T00:01:00Z, value=30.0)
    SensorRecord(daqId=d1, timestamp=2026-01-01T00:02:00Z, value=50.0)
  Mappings:
    MeterMapping(daqId=d1, meterId=m1, meterType=counter)
```

### JSON Artifact

Each test run produces `target/test-reports/scenario-<layer>-<timestamp>.json`:

```json
{
  "layer": "mini-cluster",
  "timestamp": "2026-04-05T14:30:00Z",
  "duration_ms": 32400,
  "summary": {"total": 15, "passed": 14, "failed": 1},
  "scenarios": [
    {
      "name": "Counter out-of-order correction",
      "status": "FAIL",
      "duration_ms": 2100,
      "expected": {"enrichedRecords": [{"value": 20.0}]},
      "actual": {"enrichedRecords": [{"value": 50.0}]},
      "sideOutputs": {},
      "inputs": {"sensorRecords": [...], "mappings": [...]}
    }
  ]
}
```

Failed scenarios include full input records, expected vs actual output, and all side output contents. Passed scenarios include only name, status, and duration.

## File Organization

```
src/test/scala/com/enity/flink/
├── scenarios/
│   ├── ScenarioTestHelper.scala       # Shared job builder + CollectSink
│   ├── ScenarioReport.scala           # Report model + JSON serialization
│   ├── harness/
│   │   ├── CounterDeltaHarnessSpec.scala
│   │   ├── MeterEnrichmentHarnessSpec.scala
│   │   └── ParserHarnessSpec.scala
│   └── minicluster/
│       ├── MiniClusterTest.scala       # Trait with MiniClusterResource
│       ├── EmuEndToEndSpec.scala
│       ├── FlowiqEndToEndSpec.scala
│       ├── CounterCorrectionSpec.scala
│       ├── MappingUpdateSpec.scala
│       ├── ParseErrorRoutingSpec.scala
│       └── ... (one file per scenario group)
scripts/
└── smoke_test/
    ├── test_smoke.py                   # Runner + scenarios
    ├── smoke_report.py                 # Report formatting
    └── requirements.txt                # boto3, tabulate
```

## Running

```bash
# All unit + harness tests
sbt test

# Only harness tests
sbt "testOnly *Harness*"

# Only mini-cluster tests
sbt "testOnly *MiniCluster*"

# Smoke tests (requires AWS credentials)
cd scripts/smoke_test
uv run test_smoke.py

# Smoke with verbose output
uv run test_smoke.py --verbose
```
