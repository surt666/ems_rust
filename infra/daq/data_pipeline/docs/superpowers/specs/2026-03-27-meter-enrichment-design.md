# Meter Enrichment & Counter Delta — Design Spec

> **Note:** The binning section of this spec is **superseded by** `2026-05-01-binning-rules-design.md`. Counter delta computation and timestamp binning are now handled by a unified `BinningFunction` (replacing `CounterDeltaFunction`). See the newer spec for the full binning rules.


## Context

The Flink pipeline reads raw sensor data from Kinesis `DAQ_INPUT_STREAM`, parses device-specific JSON formats into `SensorRecord`s, and writes to the `raw_cdk` Iceberg table in S3 Tables. This spec extends the pipeline to enrich records with logical meter identity and hierarchy context, compute counter deltas, and write enriched records to a second Iceberg table `nested_meter_readings_cdk2`.

All data is append-only (event-sourced). No updates or deletes.

**Scale:** 2 million active meters.
**Runtime:** Amazon Managed Flink 1.20, Scala 3.
**Dependencies:** iceberg-flink-runtime-1.20:1.7.1, s3-tables-catalog:0.1.8, flink-connector-kinesis:5.0.0-1.20.

---

## Data Model

### DynamoDB `meter-identity` table

```
PK  (String): zero-padded hash bucket → abs(daqId.hashCode) % 20_000
SK  (String): daqId (e.g. "daq:std_json_v1:customer:meter:sensor")
logical_id      (Binary): 16-byte UUID
meter_type      (String): "gauge" | "counter"
hierarchy_path  (String): "P{id}#C{id}[#PR{id}][#G{id}]#B{id}[#A{id}]"
```

- Billing: PAY_PER_REQUEST
- Streams: NEW_AND_OLD_IMAGES
- No TTL — lifecycle managed externally

### Hierarchy path format

Encodes the full path from Partner to the meter's attachment point.

Valid hierarchy paths (Partner and Company always present):

```
P{id}#C{id}#B{id}              — Company → Building
P{id}#C{id}#B{id}#A{id}        — Company → Building → Area
P{id}#C{id}#G{id}#B{id}        — Company → Group → Building
P{id}#C{id}#G{id}#B{id}#A{id}  — Company → Group → Building → Area
P{id}#C{id}#PR{id}#B{id}       — Company → Property → Building
P{id}#C{id}#PR{id}#B{id}#A{id} — Company → Property → Building → Area
```

Hierarchy tree rules:
- Partner → Company (always)
- Company → Building | Group | Property
- Group → Building
- Property → Building
- Building → Area (optional)
- Meters attach to Building or Area only

Parsing: split on `#`, match prefix (`P`, `C`, `PR`, `G`, `B`, `A`) to extract integer IDs. Unmatched levels are `null`.

### Broadcast state model

```scala
case class MeterMapping(
  logicalId: String,          // UUID as string
  meterType: String,          // "gauge" | "counter"
  partnerId: Int,
  companyId: Int,
  propertyId: Option[Int],
  buildingId: Option[Int],
  areaId: Option[Int],
  groupId: Option[Int]
)
```

State descriptor: `MapStateDescriptor[String, MeterMapping]` keyed by `daqId`.

### Counter delta state

```scala
case class CounterState(
  lastValue: Double,
  lastTimestamp: Instant
)
```

`ValueState[CounterState]` per `daqId` in a `KeyedProcessFunction`.

---

## Stream Topology

```
Kinesis DAQ_INPUT_STREAM
  │
  ▼
Parse JSON + Validate
  ├─ [parse/validation errors] → PARSE_ERROR side output
  │
  ▼
Fork (via broadcast connect)
  │
  ├─── Raw branch ──→ raw_cdk Iceberg sink (unchanged)
  │
  └─── Enrichment branch
         │
         ▼
       BroadcastProcessFunction
       (broadcast: DDB Streams from meter-identity)
       (bootstrap: full DDB scan on open())
         │
         ├─ [unmapped daq_id] → DEAD_LETTER side output
         │
         ▼
       KeyedProcessFunction (keyed by daqId)
         │
         ├─ gauge  → emit value as-is with hierarchy context
         ├─ counter:
         │    ├─ no prior state → store baseline, emit nothing
         │    ├─ delta ≥ 0     → emit delta, update state
         │    └─ delta < 0     → ANOMALY side output, reset baseline
         │
         ▼
       nested_meter_readings_cdk2 Iceberg sink
```

### Sources

1. **Kinesis `DAQ_INPUT_STREAM`** — existing, unchanged
2. **DDB Streams from `meter-identity`** — new Kinesis source, TRIM_HORIZON, DDB Streams adapter

### Sinks

1. **`raw_cdk`** — existing Iceberg sink (unchanged)
2. **`nested_meter_readings_cdk2`** — new Iceberg sink, same S3 Tables catalog
3. **Error Kinesis stream** — single stream for all error types, discriminated by `type` field

### Side outputs

| Tag | Trigger | Content |
|-----|---------|---------|
| `PARSE_ERROR` | JSON parse failure or validation failure | Raw JSON string + error message + type="parse_error" |
| `DEAD_LETTER` | Valid record but no mapping in broadcast state | Serialized SensorRecord + type="dead_letter" |
| `ANOMALY` | Counter negative delta or other data oddity | Enriched record + delta + type="anomaly" |

All three route to the same error Kinesis stream as JSON with a `type` discriminator field.

---

## Bootstrap & Cache Invalidation

### Cold start (open())

1. Create `DynamoDbClient` (sync) with fixed thread pool (~20 threads)
2. Query all 20,000 partitions in parallel: `PK = zeroPad(i, 5)`, paginate fully
3. For each item: parse `logical_id` (binary → UUID string), `meter_type`, `hierarchy_path` → `MeterMapping`
4. Buffer all results in a `ConcurrentHashMap`
5. After all partitions complete: apply to broadcast state serially (not thread-safe)
6. Log total items loaded and elapsed time
7. Fail fast if any partition query throws

Estimated: ~2M items, ~300MB state, ~20,000 RCU, ~$0.006.

### DDB Streams updates

```scala
case class IdMappingChange(
  eventType: String,              // INSERT, MODIFY, REMOVE
  daqId: String,
  mapping: Option[MeterMapping]   // None on REMOVE
)
```

- `INSERT` / `MODIFY` → `state.put(daqId, mapping)`
- `REMOVE` → `state.remove(daqId)`

DDB Streams source uses TRIM_HORIZON to catch changes during bootstrap.

---

## Counter Delta Logic

### Gauge meters
Emit value as-is. No state.

### Counter meters

Per-`daqId` `ValueState[CounterState]`:

1. **No prior state** → store `(value, timestamp)`, emit nothing
2. **delta = newValue - lastValue ≥ 0** → emit record with `value = delta`, update state
3. **delta < 0** → route to ANOMALY side output, update state with new value as baseline

### Future: out-of-order handling

Not implemented now. Design accommodates future addition of:
- Event-time watermarks with 24h allowed lateness
- Sorted buffer in keyed state to recompute deltas on late arrivals
- The append-only data model means corrections are new events, not updates

---

## Output Schema

### nested_meter_readings_cdk2

```
meter_id      STRING        ← logical_id (UUID)
timestamp     TIMESTAMPTZ   ← from SensorRecord
value         DOUBLE        ← raw for gauge, delta for counter
unit          STRING        ← from SensorRecord
created       TIMESTAMPTZ   ← processing time
partner_id    INT           ← from hierarchy_path
company_id    INT           ← from hierarchy_path
property_id   INT (nullable) ← from hierarchy_path
building_id   INT (nullable) ← from hierarchy_path
area_id       INT (nullable) ← from hierarchy_path
group_id      INT (nullable) ← from hierarchy_path
```

Iceberg sink: `FlinkSink.forRow()` with `DistributionMode.NONE`, `writeParallelism(1)`, same S3 Tables catalog pattern as `raw_cdk`.

### Error stream record

```json
{
  "type": "parse_error|dead_letter|anomaly",
  "timestamp": "2026-03-27T18:00:00Z",
  "daq_id": "daq:...",
  "payload": "...",
  "error": "description of what went wrong"
}
```

---

## Infrastructure Changes (CDK)

1. **`meter-identity` DynamoDB table** — new CDK construct with streams enabled
2. **Error Kinesis stream** — new stream for error/dead-letter/anomaly output
3. **IAM permissions** — add to existing KDA service role:
   - `dynamodb:Query` on `meter-identity` table
   - `dynamodb:DescribeStream`, `GetRecords`, `GetShardIterator`, `ListStreams` on its stream ARN
   - `kinesis:PutRecord`, `PutRecords` on the error stream
   - `s3tables:*` actions already granted for `nested_meter_readings_cdk2` (same bucket)
4. **Environment properties** — add to Flink app config:
   - `METER_IDENTITY_TABLE`: DynamoDB table name
   - `DDB_STREAM_ARN`: meter-identity stream ARN
   - `ERROR_STREAM`: error Kinesis stream name

---

## New Files

```
src/main/scala/com/enity/flink/
  enrichment/
    MeterMapping.scala            ← MeterMapping, CounterState, IdMappingChange case classes
    MeterEnrichmentFunction.scala ← BroadcastProcessFunction (enrichment + dead letter)
    CounterDeltaFunction.scala    ← KeyedProcessFunction (gauge passthrough, counter delta)
    DdbBootstrapLoader.scala      ← full DDB scan on open()
    DdbStreamDeserializer.scala   ← DDB Streams JSON → IdMappingChange
    HierarchyPathParser.scala     ← "P1#C2#B8#A1" → individual IDs
    SideOutputTags.scala          ← PARSE_ERROR, DEAD_LETTER, ANOMALY tag definitions
```

Existing `Main.scala` modified to wire up the enrichment branch, DDB Streams source, error sink, and side output routing.

---

## Testing

### Unit tests
- `HierarchyPathParserTest` — all valid path formats, missing optional levels, malformed paths
- `CounterDeltaFunctionTest` — first message drop, positive delta, negative delta anomaly, gauge passthrough
- `DdbStreamDeserializerTest` — INSERT/MODIFY/REMOVE event parsing, binary UUID decoding
- `MeterEnrichmentFunctionTest` — mapped lookup, unmapped dead letter routing

### Integration tests
- Full pipeline with mock DynamoDB (LocalStack or in-memory) — end-to-end from parsed SensorRecord through enrichment to output Row

---

## Out of Scope

- Out-of-order message handling (24h window) — future
- Historical data backfill / recalculation — future
- Dead letter replay — future
- Populating `meter-identity` table — external workflow
- Table API / SQL migration for sinks — future cleanup
