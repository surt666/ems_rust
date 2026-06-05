# Flink DAQ Pipeline

Scala 3 streaming application running on Amazon Managed Flink (Flink 1.20). Ingests IoT sensor data from Kinesis, normalizes it into a unified format, writes raw records to Iceberg, enriches with meter identity and hierarchy context, computes counter deltas with out-of-order handling, and writes enriched readings to a second Iceberg table.

## Operator Topology

The job runs 9 operators organized into two branches from a shared parsed stream:

```
                                ┌─ raw-row-mapper ─► IcebergStreamWriter ─► IcebergFilesCommitter (raw_cdk)
                                │
Kinesis Source ─► JSON Parser ──┤
                                │
                                └─ Watermark Assigner ─► Co-Process-Broadcast (enrichment)
                                                              │
DDB Change Stream ─► DDB Deserializer ──────────── broadcast ─┘
                                                              │
                                                    KeyedProcess (counter delta) ─► enriched-row-mapper
                                                              │                          │
                                                              │                   IcebergStreamWriter
                                                              │                          │
                                                              │                   IcebergFilesCommitter
                                                              │                   (nested_meter_readings_cdk2)
                                                              │
                                              side outputs: PARSE_ERROR, DEAD_LETTER,
                                                           ANOMALY, LATE_ARRIVAL
                                                              │
                                                         union ─► Error Kinesis Sink
```

### Operator Descriptions

| # | Flink Operator Name | UID | Source File | Description |
|---|-------------------|-----|-------------|-------------|
| 1 | `Source: Custom Source → Process → Map → Map` | `kinesis-source`, `json-parser`, `raw-row-mapper` | `Main.scala` | Reads raw JSON from the DAQ Kinesis stream. The `ProcessFunction` routes each message through the appropriate device processor (EMU, FLOWIQ, etc.) based on `schematype`, producing `SensorRecord`s. Invalid messages are routed to the `PARSE_ERROR` side output. The map stages convert `SensorRecord` to an Iceberg `Row` (daq_id, timestamp, value, unit, created). |
| 2 | `Source: Custom Source → Map` | `ddb-streams-source` | `Main.scala`, `DdbStreamDeserializer.scala` | Reads DynamoDB CDC events from a Kinesis Data Stream (attached to the meter-identity DynamoDB table). `DdbStreamDeserializer` parses the DDB Streams JSON envelope, handles double-base64-encoded Binary attributes, and produces `IdMappingChange` records (INSERT/MODIFY/REMOVE + `MeterMapping`). |
| 3 | `IcebergStreamWriter` | (auto) | Iceberg library | Buffers incoming raw `Row` records and writes Iceberg data files to S3. Triggered by checkpoint boundaries. |
| 4 | `IcebergFilesCommitter → Sink: IcebergSink s3tablescatalog.all.raw_cdk` | (auto) | Iceberg library | Commits the data files written by operator 3 as a new Iceberg snapshot in the `raw_cdk` table (S3 Tables catalog). Runs on checkpoint completion for exactly-once semantics. |
| 5 | `Co-Process-Broadcast` | `meter-enrichment` | `MeterEnrichmentFunction.scala` | `BroadcastProcessFunction` that joins the watermarked `SensorRecord` stream with broadcast meter-identity state. On `open()`, performs a parallel DynamoDB scan (`DdbBootstrapLoader`) to bootstrap the local cache. CDC events from operator 2 update broadcast state and take priority over the bootstrap cache. For each `SensorRecord`, looks up the `MeterMapping` by `daqId` and emits `(EnrichedRecord, meterType)`. Unresolved records go to the `DEAD_LETTER` side output. |
| 6 | `KeyedProcess → Map → Map` | `counter-delta`, `enriched-row-mapper` | `CounterDeltaFunction.scala`, `Main.scala` | `KeyedProcessFunction` keyed by `meterId`. **Gauges** pass through immediately with the raw value. **Counters** use an event-time buffer (`MapState[Long, BufferedReading]`) to handle out-of-order arrivals within the watermark window (default 1 hour). When the watermark advances past a timer, `onTimer` sorts buffered readings by timestamp, computes deltas between consecutive entries using `sliding(2)`, and emits `EnrichedRecord`s with delta values. Negative deltas are flagged as anomalies (side output `ANOMALY`, record suppressed). Records arriving after the watermark go to `LATE_ARRIVAL` side output. Old buffer entries beyond the retention window (default 6 hours) are purged. The map stages convert `EnrichedRecord` to an Iceberg `Row`. |
| 7 | `IcebergStreamWriter` | (auto) | Iceberg library | Buffers enriched `Row` records and writes Iceberg data files to S3. Same mechanics as operator 3 but for the enriched table. |
| 8 | `IcebergFilesCommitter → Sink: IcebergSink s3tablescatalog.all.nested_meter_readings_cdk2` | (auto) | Iceberg library | Commits enriched data files as Iceberg snapshots in `nested_meter_readings_cdk2`. |
| 9 | `Sink: Unnamed` | `error-sink` | `Main.scala` | `FlinkKinesisProducer` that serializes all error side outputs (`PARSE_ERROR ∪ DEAD_LETTER ∪ ANOMALY ∪ LATE_ARRIVAL`) as JSON and writes to the error Kinesis stream. |

### Watermark Strategy

Watermarks are assigned on the enrichment branch only (not the raw branch, which writes all records unconditionally):

- **BoundedOutOfOrderness**: configurable via `MAX_OUT_OF_ORDERNESS_MS` (default 1 hour)
- **Timestamp extractor**: parses `SensorRecord.timestamp` as ISO-8601 (`Instant.parse` with `OffsetDateTime` fallback)
- **Idleness**: 24 hours — prevents idle partitions from holding back the watermark

### Side Outputs

| Tag | Source Operator | Trigger |
|-----|----------------|---------|
| `PARSE_ERROR` | JSON Parser (#1) | JSON parsing failure or no valid records produced |
| `DEAD_LETTER` | Meter Enrichment (#5) | No meter mapping found for `daqId` |
| `ANOMALY` | Counter Delta (#6) | Negative counter delta (counter rollback/reset) |
| `LATE_ARRIVAL` | Counter Delta (#6) | Record timestamp is behind current watermark |

## Supported Sensor Types

| Schema Type | Processor | Description |
|------------|-----------|-------------|
| `emu_profes_v1` | EmuProcessor | EMU Professional energy meters with LoRaWAN |
| `gwb143_json_v1` | Gwb143Processor | GWB143 gateway data |
| `std_jsonl_v1` | StdProcessor | Standard JSONL format sensors |
| `std_json_v1` | StdProcessor | Standard JSON format sensors |
| `bluemetering_json_v1` | BluemeteringProcessor | Bluemetering smart meters |
| `mivo_json_v1` | MivoProcessor | MIVO multi-meter readings |
| `mc603_v1` | Mc603Processor | Kamstrup MC603 heat meters |
| `flowiq2200_v1` | Flowiq2200Processor | Kamstrup FLOWIQ 2200 water meters (MBus protocol) |
| `pulse_v1` | PulseProcessor | Adeunis pulse counter meters |
| `ediel_json_v1` | EdielProcessor | EDIEL format energy data |

## Project Structure

```
flink_app_scala/
├── build.sbt
├── project/
│   ├── build.properties
│   └── plugins.sbt
└── src/
    ├── main/scala/com/enity/flink/
    │   ├── Main.scala                    # Entry point, job graph wiring
    │   ├── Models.scala                  # SensorRecord case class
    │   ├── processors/                   # Device-specific JSON → SensorRecord transforms
    │   │   ├── BluemeteringProcessor.scala
    │   │   ├── EdielProcessor.scala
    │   │   ├── EmuProcessor.scala
    │   │   ├── Flowiq2200Processor.scala
    │   │   ├── Gwb143Processor.scala
    │   │   ├── Mc603Processor.scala
    │   │   ├── MivoProcessor.scala
    │   │   ├── PulseProcessor.scala
    │   │   └── StdProcessor.scala
    │   ├── enrichment/                   # Meter enrichment pipeline
    │   │   ├── MeterMapping.scala        # Case classes: MeterMapping, EnrichedRecord, CounterState, etc.
    │   │   ├── HierarchyPathParser.scala # Parses "P1#C2#PR4#B8" → individual hierarchy IDs
    │   │   ├── SideOutputTags.scala      # OutputTag definitions for error routing
    │   │   ├── DdbStreamDeserializer.scala  # DDB Kinesis CDC → IdMappingChange
    │   │   ├── DdbBootstrapLoader.scala  # Parallel DynamoDB scan for bootstrap cache
    │   │   ├── MeterEnrichmentFunction.scala  # BroadcastProcessFunction (identity lookup)
    │   │   └── CounterDeltaFunction.scala     # KeyedProcessFunction (delta computation)
    │   └── utils/
    │       └── ProcessUtils.scala
    └── test/scala/com/enity/flink/
        ├── processors/
        ├── enrichment/                   # Enrichment pipeline tests
        │   ├── MeterEnrichmentFunctionSpec.scala
        │   ├── CounterDeltaFunctionSpec.scala
        │   ├── DdbBootstrapLoaderSpec.scala
        │   ├── DdbStreamDeserializerSpec.scala
        │   ├── HierarchyPathParserSpec.scala
        │   ├── SideOutputTagsSpec.scala
        │   └── EnrichmentPipelineSpec.scala  # End-to-end integration tests
        └── utils/
```

## Building

### Prerequisites

- Java 11 or later
- SBT 1.9.7 or later
- Scala 3.3.4

### Build Commands

```bash
sbt compile          # Compile
sbt test             # Run tests
sbt assembly         # Fat JAR for deployment
```

The assembled JAR is at `target/scala-3.3.4/flink-app-scala-assembly-0.1.0.jar`.

## Configuration

The application reads properties from `/etc/flink/application_properties.json` (Amazon Managed Flink convention):

| Property | Default | Description |
|----------|---------|-------------|
| `AWS_REGION` | `eu-central-1` | AWS region |
| `ACCOUNT_ID` | | AWS account ID (for S3 Tables ARN) |
| `TABLE_BUCKET_NAME` | `measurements` | S3 Tables bucket name |
| `INPUT_STREAM` | | DAQ Kinesis stream name |
| `DDB_CHANGE_STREAM` | | Meter-identity DDB Kinesis stream (enables enrichment) |
| `METER_IDENTITY_TABLE` | `meter-identity` | DynamoDB table for bootstrap scan |
| `ERROR_STREAM` | | Error Kinesis stream name |
| `MAX_OUT_OF_ORDERNESS_MS` | `3600000` | Watermark out-of-orderness (1 hour) |
| `BUFFER_RETENTION_MS` | `21600000` | Counter delta buffer retention (6 hours) |

The enrichment branch (operators 2, 5–9) is only activated when `DDB_CHANGE_STREAM` is non-empty. Without it, the job only writes raw records to `raw_cdk`.

## Iceberg Tables

| Table | Catalog | Schema |
|-------|---------|--------|
| `all.raw_cdk` | S3 Tables | `daq_id STRING, timestamp TIMESTAMPTZ, value DOUBLE, unit STRING, created TIMESTAMPTZ` |
| `all.nested_meter_readings_cdk2` | S3 Tables | `meter_id STRING, timestamp TIMESTAMPTZ, value DOUBLE, unit STRING, created TIMESTAMPTZ, partner_id INT, company_id INT, property_id INT, building_id INT, area_id INT, group_id INT` |

## Append-Only Correction Pattern

The `logical_meter_data` (enriched) Iceberg table is append-only. Counter delta records may be written more than once for the same `(logical_id, timestamp)` pair — each append carries a newer `ingested_time`. **Consumers must deduplicate by selecting `MAX(ingested_time)` per `(logical_id, timestamp)`.**

This happens in three scenarios:

1. **Immediate write + timer correction (out-of-order within watermark window):** When a counter record arrives, `CounterDeltaFunction.emitFromBuffer` immediately computes a delta against the nearest predecessor in the buffer and writes it. If an earlier-timestamped record arrives later (out-of-order), the event-time timer re-emits corrected deltas for affected pairs. The correction has a newer `ingested_time` than the original.

2. **Late arrival within buffer retention (1h–6h late):** Records arriving after the watermark but while the predecessor is still in the 6-hour retention buffer. The delta is computed against the buffered predecessor and appended with a current `ingested_time`.

3. **Late arrival beyond buffer retention (>6h late):** The predecessor has been purged. The record is routed to the `LATE_ARRIVAL` error stream. A separate Glue Spark job (`late-data-recomputation`) recomputes deltas from `raw_cdk` and appends corrected records with a new `ingested_time`.

The 1-hour watermark (`MAX_OUT_OF_ORDERNESS_MS`) keeps the buffer window open for out-of-order records to arrive so corrections can be computed — it does not delay writes. Data flows to the Iceberg sink at checkpoint speed (~5 minutes).

See `docs/superpowers/plans/2026-03-28-late-data-recomputation.md` for the batch recomputation plan.

## Deployment

Deployed via CDK as an Amazon Managed Flink application. See `lib/flink_transforms-stack.ts` for the infrastructure definition.

- Checkpoint interval: 5 minutes
- Restore mode: `RESTORE_FROM_LATEST_SNAPSHOT`
- Parallelism: 1 (configurable in CDK)

## Known Constraints

- **Flink Kryo + Scala `Option[Int]`**: Flink's Kryo serializer corrupts `Option[Int]` across broadcast state boundaries. All nullable integer fields use `java.lang.Integer` instead.
- **DDB Kinesis CDC double-encodes** Binary attributes (base64 inside base64). `DdbStreamDeserializer` handles this.
- **Scala 3 SAM ambiguity**: `executor.submit(() => ...)` is ambiguous between `Callable` and `Runnable`. Use explicit type annotation: `val task: Runnable = () => ...`.
