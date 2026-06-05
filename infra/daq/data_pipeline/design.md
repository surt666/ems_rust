# Flink Meter Enrichment — Implementation Plan

## Context

Enity data platform. A Kinesis stream carries raw meter events identified by `serialnr` (physical device ID). The downstream sink requires `logical_id` (UUID), which represents the logical meter regardless of which physical device is installed. The mapping is 1:1 at any point in time but changes slowly as devices are replaced (SCD Type 2).

**Scale:** 2 million active meters.  
**Runtime:** Amazon Managed Service for Apache Flink 1.20 (Java).  
**Language:** Java (not PyFlink).

---

## Key Decisions (do not revisit these)

| Decision | Choice |
|---|---|
| Lookup store | DynamoDB — active mappings only |
| DDB access pattern | `Query` by hash partition, not `GetItem` per serialnr |
| Partition scheme | `PK = hash(serialnr) % 20000`, `SK = serialnr` — packs ~100 items per 4KB page, ~1 RCU per Query |
| `logical_id` storage | Binary (16 bytes), not String UUID |
| Cache invalidation | DynamoDB Streams → Flink broadcast (push, not TTL polling) |
| Cold start | Full DynamoDB scan to populate broadcast state before processing begins |
| Flink API | DataStream + `BroadcastProcessFunction` (not temporal join) |
| DDB Stream offset | `TRIM_HORIZON` — catch changes that arrive during bootstrap scan |

---

## Data Model

### DynamoDB Table: `meter-identity`

```
PK  (String): zero-padded partition bucket, e.g. "04821"  →  Math.abs(serialnr.hashCode()) % 20_000
SK  (String): serialnr
logical_id (Binary): 16-byte UUID
```

- Billing mode: PAY_PER_REQUEST
- DynamoDB Streams: `NEW_AND_OLD_IMAGES` (required — Flink needs both old and new serialnr on a replacement)
- No TTL attribute — item lifecycle managed by the replacement workflow

Partition sizing rationale:
- 2M meters / 20,000 partitions = ~100 items per partition
- ~60 bytes per item → ~6KB per partition → ~1 RCU per Query
- Full cache warm on job start: ~20,000 RCU ≈ $0.006

### Iceberg Table: `meter_identity` (source of truth, not read by Flink at runtime)

```sql
CREATE TABLE meter_identity (
    serialnr      STRING,
    logical_id    STRING,
    valid_from    TIMESTAMP(3),
    valid_to      TIMESTAMP(3),   -- NULL = currently active
    PRIMARY KEY (serialnr, valid_from) NOT ENFORCED
)
PARTITIONED BY (truncate(serialnr, 4));
```

---

## Project Structure

```
flink-meter-enrichment/
├── pom.xml
└── src/
    ├── main/java/com/enity/flink/
    │   ├── MeterEnrichmentJob.java          ← main(), job assembly
    │   ├── model/
    │   │   ├── MeterEvent.java              ← serialnr, value, eventTime
    │   │   ├── EnrichedEvent.java           ← logicalId, value, eventTime
    │   │   └── IdMappingChange.java         ← eventType (INSERT/MODIFY/REMOVE), serialnr, logicalId
    │   ├── source/
    │   │   └── DdbStreamDeserializer.java   ← deserialise DDB Streams JSON → IdMappingChange
    │   ├── enrichment/
    │   │   ├── MeterEnrichmentFunction.java ← BroadcastProcessFunction
    │   │   └── DdbBootstrapLoader.java      ← full DDB scan on open()
    │   └── util/
    │       └── UuidBytes.java               ← UUID ↔ byte[] helpers
    └── test/java/com/enity/flink/
        ├── MeterEnrichmentFunctionTest.java
        ├── DdbBootstrapLoaderTest.java
        └── DdbStreamDeserializerTest.java
```

---

## pom.xml Dependencies

```xml
<!-- Flink -->
<dependency>
  <groupId>org.apache.flink</groupId>
  <artifactId>flink-streaming-java</artifactId>
  <version>1.20.0</version>
  <scope>provided</scope>
</dependency>

<!-- Kinesis connector (covers both Kinesis and DDB Streams) -->
<dependency>
  <groupId>org.apache.flink</groupId>
  <artifactId>flink-connector-kinesis</artifactId>
  <version>4.3.0-1.18</version>  <!-- latest compatible with Flink 1.20 on MSF — verify -->
</dependency>

<!-- AWS SDK v2 for DynamoDB bootstrap scan -->
<dependency>
  <groupId>software.amazon.awssdk</groupId>
  <artifactId>dynamodb</artifactId>
  <version>2.25.0</version>
</dependency>
```

---

## Implementation Tasks

### Task 1 — DynamoDB CDK infrastructure

File: `infrastructure/lib/meter-identity-table.ts`

```typescript
const table = new dynamodb.Table(this, 'MeterIdentity', {
  tableName: 'meter-identity',
  partitionKey: { name: 'pk', type: dynamodb.AttributeType.STRING },
  sortKey:      { name: 'sk', type: dynamodb.AttributeType.STRING },
  billingMode:  dynamodb.BillingMode.PAY_PER_REQUEST,
  stream:       dynamodb.StreamViewType.NEW_AND_OLD_IMAGES,
  pointInTimeRecovery: true,
});
```

Grant the Flink execution role `dynamodb:Query` on the table and `dynamodb:DescribeStream`, `dynamodb:GetRecords`, `dynamodb:GetShardIterator`, `dynamodb:ListStreams` on the stream ARN.

---

### Task 2 — Model classes

**`MeterEvent`** — POJO with fields: `String serialnr`, `double value`, `Instant eventTime`. Must implement `Serializable`.

**`EnrichedEvent`** — POJO with fields: `String logicalId`, `double value`, `Instant eventTime`. Must implement `Serializable`.

**`IdMappingChange`** — POJO with fields:
```java
enum EventType { INSERT, MODIFY, REMOVE }
EventType eventType;
String serialnr;
String logicalId;  // null on REMOVE
```

---

### Task 3 — DDB Streams deserializer

File: `DdbStreamDeserializer.java`

Implement `KinesisDeserializationSchema<IdMappingChange>`. DynamoDB Streams records arrive as JSON when consumed via the Kinesis Streams adapter.

Parse the `eventName` field: `INSERT` / `MODIFY` / `REMOVE`.

For `INSERT` and `MODIFY`: extract `NewImage.sk.S` (serialnr) and `NewImage.logical_id.B` (base64 binary UUID).  
For `REMOVE`: extract `OldImage.sk.S` (serialnr), set `logicalId = null`.

Use Jackson for JSON parsing. The DDB Streams JSON envelope looks like:

```json
{
  "eventName": "INSERT",
  "dynamodb": {
    "NewImage": {
      "pk": { "S": "04821" },
      "sk": { "S": "DE000123456" },
      "logical_id": { "B": "<base64>" }
    }
  }
}
```

---

### Task 4 — Broadcast state descriptor

Define as a public static constant on `MeterEnrichmentFunction` so it can be shared with the bootstrap loader:

```java
public static final MapStateDescriptor<String, String> ID_MAP =
    new MapStateDescriptor<>(
        "meter-id-map",
        BasicTypeInfo.STRING_TYPE_INFO,
        BasicTypeInfo.STRING_TYPE_INFO
    );
```

The value is a String UUID (converted from the 16-byte binary after read from DDB). Keeping it as String in state avoids custom serializer complexity.

---

### Task 5 — Bootstrap loader

File: `DdbBootstrapLoader.java`

Called from `MeterEnrichmentFunction.open()` before processing starts.

```
Algorithm:
  partitions = 0..19999
  for each partition (run in parallel using a thread pool, ~20 threads):
    Query DDB: pk = zeroPad(partitionId, 5), paginate until LastEvaluatedKey is absent
    for each item: buffer.put(sk, uuidFromBytes(logical_id))
  after all partitions complete:
    buffer.forEach(state::put)   ← apply serially; BroadcastState is not thread-safe
```

Use `DynamoDbClient` (sync) with a fixed thread pool, not `DynamoDbAsyncClient`, to keep the bootstrap blocking (open() must complete before processing).

Log total items loaded and time taken. Fail fast if any partition Query throws — do not silently skip.

---

### Task 6 — BroadcastProcessFunction

File: `MeterEnrichmentFunction.java`

```
processElement(MeterEvent event, ReadOnlyContext ctx, Collector<EnrichedEvent> out):
  logicalId = ctx.getBroadcastState(ID_MAP).get(event.serialnr)
  if logicalId != null:
    out.collect(new EnrichedEvent(logicalId, event.value, event.eventTime))
  else:
    ctx.output(DEAD_LETTER_TAG, event)

processBroadcastElement(IdMappingChange change, Context ctx, Collector<EnrichedEvent> out):
  state = ctx.getBroadcastState(ID_MAP)
  switch change.eventType:
    INSERT, MODIFY → state.put(change.serialnr, change.logicalId)
    REMOVE         → state.remove(change.serialnr)
```

Define `DEAD_LETTER_TAG` as:
```java
public static final OutputTag<MeterEvent> DEAD_LETTER_TAG =
    new OutputTag<>("dead-letter", TypeInformation.of(MeterEvent.class));
```

---

### Task 7 — Job assembly

File: `MeterEnrichmentJob.java`

```
1. Create StreamExecutionEnvironment
2. Configure checkpointing: interval 5 min, min pause 30s, mode EXACTLY_ONCE
3. Build Kinesis source for meter events (LATEST or configurable offset)
4. Build Kinesis source for DDB Streams (TRIM_HORIZON — always, to cover bootstrap window)
5. Broadcast DDB stream: mappingChanges.broadcast(MeterEnrichmentFunction.ID_MAP)
6. Connect and process:
     meterEvents.connect(broadcastChanges).process(new MeterEnrichmentFunction())
7. Wire dead letter side output to dead letter sink (separate Kinesis stream or S3)
8. Wire main output to primary sink
9. env.execute("meter-enrichment")
```

Kinesis source for DDB Streams: use the DDB Streams endpoint, not the standard Kinesis endpoint. The stream ARN comes from the DDB table's `latestStreamArn`. Configure via MSF runtime properties so it does not need to be hardcoded.

---

### Task 8 — Meter replacement workflow (external to Flink)

This is not a Flink concern but must be implemented correctly or the broadcast state gets corrupted.

**Ordering is critical: INSERT new before DELETE old.**

```
1. Write new row to Iceberg: new serialnr, valid_from = now, valid_to = NULL
2. Update old row in Iceberg: valid_to = now
3. DynamoDB PutItem: pk = hash(newSerialnr) % 20000, sk = newSerialnr, logical_id = <binary UUID>
4. DynamoDB DeleteItem: pk = hash(oldSerialnr) % 20000, sk = oldSerialnr
```

Step 3 before step 4 ensures Flink's broadcast state always has at least one valid mapping for the logical meter. The DDB Stream will emit INSERT then REMOVE, which Flink processes in order.

---

### Task 9 — Dead letter replay

Implement a Lambda or separate Flink job that:
- Reads from the dead letter stream/S3 prefix
- Waits a configurable delay (default 60s) to allow broadcast state to catch up
- Re-publishes events to the main Kinesis stream

---

## Operational Notes

**Broadcast state parallelism:** The broadcast input is always consumed at parallelism 1 internally by Flink. The `process()` operator can scale freely. Set main job parallelism based on Kinesis shard count.

**State size:** ~100 bytes per entry × 2M entries ≈ 200MB per TaskManager slot. Set `taskmanager.memory.task.heap.size: 1g` in MSF application properties.

**Checkpoints:** Stored to S3. On recovery, broadcast state is restored from checkpoint — no re-bootstrap unless checkpoint is lost. Keep at least 3 retained checkpoints.

**Monitoring:** Alarm on dead letter stream depth > 0 sustained for > 5 minutes (indicates systematic resolution failure, not transient bootstrap race).

---

## Test Plan

### Unit tests (no AWS)

- `MeterEnrichmentFunctionTest` — use `TestHarnessUtils` / `BroadcastOperatorTestHarness` to feed synthetic events and mapping changes. Assert correct enrichment output and dead letter routing on missing mapping.
- `DdbBootstrapLoaderTest` — mock `DynamoDbClient`. Verify all 20,000 partitions queried, items correctly land in state, thread safety (serialize apply step).
- `DdbStreamDeserializerTest` — feed raw DDB Streams JSON fixtures (INSERT, MODIFY, REMOVE). Assert parsed `IdMappingChange` fields including binary UUID decoding.

### Integration tests (LocalStack)

- Start LocalStack with DynamoDB + DynamoDB Streams.
- Seed table with known mappings.
- Run `DdbBootstrapLoader` against LocalStack. Assert state contains all seeded items.
- Emit a replacement change. Assert broadcast state updates: new serialnr resolves, old serialnr routes to dead letter.
- Run full job graph using `MiniClusterWithClientResource` against LocalStack sources and a collecting sink. Assert end-to-end enrichment.
