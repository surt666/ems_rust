# Meter Enrichment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the Flink pipeline to enrich raw sensor records with logical meter identity, hierarchy context, and counter delta computation, writing enriched records to a second Iceberg table.

**Architecture:** Fork the parsed record stream: one branch writes to `raw_cdk` (unchanged), the other passes through a `BroadcastProcessFunction` (DynamoDB-backed meter identity lookup) then a `KeyedProcessFunction` (gauge passthrough / counter delta) before writing to `nested_meter_readings_cdk2`. Side outputs route errors to a single Kinesis error stream.

**Tech Stack:** Scala 3.3.4, Flink 1.20.3, Iceberg 1.7.1, s3-tables-catalog 0.1.8, AWS DynamoDB SDK v2, ScalaTest

**Spec:** `docs/superpowers/specs/2026-03-27-meter-enrichment-design.md`

---

## File Map

### New files (src/main/scala/com/enity/flink/enrichment/)

| File | Responsibility |
|------|---------------|
| `MeterMapping.scala` | `MeterMapping`, `CounterState`, `IdMappingChange`, `EnrichedRecord`, `ErrorRecord` case classes |
| `HierarchyPathParser.scala` | Parse `"P1#C2#PR4#B8#A1"` → individual integer IDs |
| `SideOutputTags.scala` | `PARSE_ERROR`, `DEAD_LETTER`, `ANOMALY` `OutputTag` definitions |
| `DdbStreamDeserializer.scala` | `KinesisDeserializationSchema[IdMappingChange]` — DDB Streams JSON → `IdMappingChange` |
| `DdbBootstrapLoader.scala` | Full DynamoDB scan on `open()` to populate broadcast state |
| `MeterEnrichmentFunction.scala` | `BroadcastProcessFunction` — lookup `daqId` in broadcast state, emit enriched or dead letter |
| `CounterDeltaFunction.scala` | `KeyedProcessFunction` — gauge passthrough, counter delta computation |

### New files (src/test/scala/com/enity/flink/enrichment/)

| File | Tests |
|------|-------|
| `HierarchyPathParserSpec.scala` | All valid path formats, optional levels, malformed paths |
| `DdbStreamDeserializerSpec.scala` | INSERT/MODIFY/REMOVE parsing, binary UUID decoding |
| `MeterEnrichmentFunctionSpec.scala` | Mapped lookup, unmapped dead letter, broadcast updates |
| `CounterDeltaFunctionSpec.scala` | Gauge passthrough, counter first-message drop, positive delta, negative delta anomaly |
| `SideOutputTagsSpec.scala` | Error record JSON serialization |

### Modified files

| File | Change |
|------|--------|
| `build.sbt` | Add AWS DynamoDB SDK, Flink test utils dependencies |
| `Main.scala` | Wire up side outputs, DDB Streams source, enrichment branch, second Iceberg sink, error sink |
| `lib/flink_transforms-stack.ts` | Add DynamoDB table, error Kinesis stream, IAM permissions, env properties |

---

## Task 1: Add dependencies to build.sbt

**Files:**
- Modify: `build.sbt`

- [ ] **Step 1: Add AWS SDK and Flink test dependencies**

Add to `libraryDependencies` in `build.sbt`:

```scala
  // AWS SDK for DynamoDB bootstrap scan
  "software.amazon.awssdk" % "dynamodb" % "2.25.0",

  // Flink test utils for harness-based unit tests
  "org.apache.flink" % "flink-test-utils" % flinkVersion % Test,
  "org.apache.flink" % "flink-streaming-java" % flinkVersion % Test classifier "tests",
  "org.apache.flink" % "flink-runtime" % flinkVersion % Test classifier "tests",
```

- [ ] **Step 2: Verify it compiles**

Run: `sbt compile`
Expected: BUILD SUCCESS

- [ ] **Step 3: Commit**

```bash
git add build.sbt
git commit -m "build: add DynamoDB SDK and Flink test utils dependencies"
```

---

## Task 2: Case classes and side output tags

**Files:**
- Create: `src/main/scala/com/enity/flink/enrichment/MeterMapping.scala`
- Create: `src/main/scala/com/enity/flink/enrichment/SideOutputTags.scala`
- Test: `src/test/scala/com/enity/flink/enrichment/SideOutputTagsSpec.scala`

- [ ] **Step 1: Write the test for ErrorRecord JSON serialization**

Create `src/test/scala/com/enity/flink/enrichment/SideOutputTagsSpec.scala`:

```scala
package com.enity.flink.enrichment

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.scala.DefaultScalaModule
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class SideOutputTagsSpec extends AnyFlatSpec with Matchers {
  private val mapper = new ObjectMapper()
  mapper.registerModule(DefaultScalaModule)

  "ErrorRecord" should "serialize to JSON with all fields" in {
    val error = ErrorRecord(
      errorType = "parse_error",
      timestamp = "2026-03-27T18:00:00Z",
      daqId = "daq:std_json_v1:customer:meter:sensor",
      payload = """{"bad":"json""",
      error = "Unexpected end of input"
    )
    val json = mapper.writeValueAsString(error)
    val parsed = mapper.readValue(json, classOf[Map[String, Any]])

    parsed("type") shouldBe "parse_error"
    parsed("daq_id") shouldBe "daq:std_json_v1:customer:meter:sensor"
    parsed("error") shouldBe "Unexpected end of input"
  }

  "ErrorRecord" should "use 'type' as the JSON field name, not 'errorType'" in {
    val error = ErrorRecord("dead_letter", "2026-03-27T18:00:00Z", "daq:x", "{}", "no mapping")
    val json = mapper.writeValueAsString(error)
    json should include("\"type\"")
    json should not include("\"errorType\"")
  }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `sbt "testOnly com.enity.flink.enrichment.SideOutputTagsSpec"`
Expected: FAIL — class not found

- [ ] **Step 3: Create MeterMapping.scala with all case classes**

Create `src/main/scala/com/enity/flink/enrichment/MeterMapping.scala`:

```scala
package com.enity.flink.enrichment

import com.fasterxml.jackson.annotation.JsonProperty
import java.time.Instant

/** Broadcast state entry: one per daqId */
case class MeterMapping(
  logicalId: String,
  meterType: String,
  partnerId: Int,
  companyId: Int,
  propertyId: Option[Int],
  buildingId: Option[Int],
  areaId: Option[Int],
  groupId: Option[Int]
) extends Serializable

/** Keyed state for counter delta computation */
case class CounterState(
  lastValue: Double,
  lastTimestamp: Instant
) extends Serializable

/** DDB Streams change event */
case class IdMappingChange(
  eventType: String,
  daqId: String,
  mapping: Option[MeterMapping]
) extends Serializable

/** Enriched record ready for Iceberg sink */
case class EnrichedRecord(
  meterId: String,
  timestamp: String,
  value: Double,
  unit: String,
  created: String,
  partnerId: Int,
  companyId: Int,
  propertyId: Option[Int],
  buildingId: Option[Int],
  areaId: Option[Int],
  groupId: Option[Int]
) extends Serializable

/** Error/dead-letter record for the error Kinesis stream.
  * Uses @JsonProperty to serialize errorType as "type" and daqId as "daq_id". */
case class ErrorRecord(
  @JsonProperty("type") errorType: String,
  timestamp: String,
  @JsonProperty("daq_id") daqId: String,
  payload: String,
  error: String
) extends Serializable
```

- [ ] **Step 4: Create SideOutputTags.scala**

Create `src/main/scala/com/enity/flink/enrichment/SideOutputTags.scala`:

```scala
package com.enity.flink.enrichment

import org.apache.flink.api.common.typeinfo.TypeInformation
import org.apache.flink.util.OutputTag

object SideOutputTags:
  val PARSE_ERROR: OutputTag[ErrorRecord] =
    new OutputTag[ErrorRecord]("parse-error")(TypeInformation.of(classOf[ErrorRecord]))

  val DEAD_LETTER: OutputTag[ErrorRecord] =
    new OutputTag[ErrorRecord]("dead-letter")(TypeInformation.of(classOf[ErrorRecord]))

  val ANOMALY: OutputTag[ErrorRecord] =
    new OutputTag[ErrorRecord]("anomaly")(TypeInformation.of(classOf[ErrorRecord]))
```

- [ ] **Step 5: Run test to verify it passes**

Run: `sbt "testOnly com.enity.flink.enrichment.SideOutputTagsSpec"`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/main/scala/com/enity/flink/enrichment/MeterMapping.scala \
        src/main/scala/com/enity/flink/enrichment/SideOutputTags.scala \
        src/test/scala/com/enity/flink/enrichment/SideOutputTagsSpec.scala
git commit -m "feat: add enrichment case classes and side output tags"
```

---

## Task 3: HierarchyPathParser

**Files:**
- Create: `src/main/scala/com/enity/flink/enrichment/HierarchyPathParser.scala`
- Test: `src/test/scala/com/enity/flink/enrichment/HierarchyPathParserSpec.scala`

- [ ] **Step 1: Write the tests**

Create `src/test/scala/com/enity/flink/enrichment/HierarchyPathParserSpec.scala`:

```scala
package com.enity.flink.enrichment

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class HierarchyPathParserSpec extends AnyFlatSpec with Matchers {

  "HierarchyPathParser" should "parse Company → Building path" in {
    val result = HierarchyPathParser.parse("P1#C2#B8")
    result.partnerId shouldBe 1
    result.companyId shouldBe 2
    result.propertyId shouldBe None
    result.groupId shouldBe None
    result.buildingId shouldBe Some(8)
    result.areaId shouldBe None
  }

  it should "parse Company → Building → Area path" in {
    val result = HierarchyPathParser.parse("P1#C2#B8#A3")
    result.partnerId shouldBe 1
    result.companyId shouldBe 2
    result.buildingId shouldBe Some(8)
    result.areaId shouldBe Some(3)
    result.propertyId shouldBe None
    result.groupId shouldBe None
  }

  it should "parse Company → Group → Building path" in {
    val result = HierarchyPathParser.parse("P10#C20#G5#B8")
    result.partnerId shouldBe 10
    result.companyId shouldBe 20
    result.groupId shouldBe Some(5)
    result.buildingId shouldBe Some(8)
    result.propertyId shouldBe None
    result.areaId shouldBe None
  }

  it should "parse Company → Group → Building → Area path" in {
    val result = HierarchyPathParser.parse("P1#C2#G5#B8#A1")
    result.partnerId shouldBe 1
    result.companyId shouldBe 2
    result.groupId shouldBe Some(5)
    result.buildingId shouldBe Some(8)
    result.areaId shouldBe Some(1)
    result.propertyId shouldBe None
  }

  it should "parse Company → Property → Building path" in {
    val result = HierarchyPathParser.parse("P1#C2#PR4#B8")
    result.partnerId shouldBe 1
    result.companyId shouldBe 2
    result.propertyId shouldBe Some(4)
    result.buildingId shouldBe Some(8)
    result.groupId shouldBe None
    result.areaId shouldBe None
  }

  it should "parse Company → Property → Building → Area path" in {
    val result = HierarchyPathParser.parse("P1#C2#PR4#B8#A1")
    result.partnerId shouldBe 1
    result.companyId shouldBe 2
    result.propertyId shouldBe Some(4)
    result.buildingId shouldBe Some(8)
    result.areaId shouldBe Some(1)
    result.groupId shouldBe None
  }

  it should "handle large IDs" in {
    val result = HierarchyPathParser.parse("P9999#C12345#B67890#A111")
    result.partnerId shouldBe 9999
    result.companyId shouldBe 12345
    result.buildingId shouldBe Some(67890)
    result.areaId shouldBe Some(111)
  }

  it should "throw on missing Partner" in {
    an[IllegalArgumentException] should be thrownBy {
      HierarchyPathParser.parse("C2#B8")
    }
  }

  it should "throw on missing Company" in {
    an[IllegalArgumentException] should be thrownBy {
      HierarchyPathParser.parse("P1#B8")
    }
  }

  it should "throw on empty string" in {
    an[IllegalArgumentException] should be thrownBy {
      HierarchyPathParser.parse("")
    }
  }

  it should "throw on unrecognized prefix" in {
    an[IllegalArgumentException] should be thrownBy {
      HierarchyPathParser.parse("P1#C2#X99#B8")
    }
  }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `sbt "testOnly com.enity.flink.enrichment.HierarchyPathParserSpec"`
Expected: FAIL — class not found

- [ ] **Step 3: Implement HierarchyPathParser**

Create `src/main/scala/com/enity/flink/enrichment/HierarchyPathParser.scala`:

```scala
package com.enity.flink.enrichment

/** Intermediate result of parsing a hierarchy path string. */
case class HierarchyIds(
  partnerId: Int,
  companyId: Int,
  propertyId: Option[Int],
  groupId: Option[Int],
  buildingId: Option[Int],
  areaId: Option[Int]
)

object HierarchyPathParser:

  /** Parse a hierarchy path like "P1#C2#PR4#B8#A1" into individual IDs.
    *
    * Recognized prefixes: P (partner), C (company), PR (property), G (group), B (building), A (area).
    * Partner and Company are required. All others are optional.
    */
  def parse(path: String): HierarchyIds =
    require(path.nonEmpty, "Hierarchy path must not be empty")

    var partnerId: Option[Int] = None
    var companyId: Option[Int] = None
    var propertyId: Option[Int] = None
    var groupId: Option[Int] = None
    var buildingId: Option[Int] = None
    var areaId: Option[Int] = None

    for segment <- path.split("#") do
      if segment.startsWith("PR") then
        propertyId = Some(segment.drop(2).toInt)
      else if segment.startsWith("P") then
        partnerId = Some(segment.drop(1).toInt)
      else if segment.startsWith("C") then
        companyId = Some(segment.drop(1).toInt)
      else if segment.startsWith("G") then
        groupId = Some(segment.drop(1).toInt)
      else if segment.startsWith("B") then
        buildingId = Some(segment.drop(1).toInt)
      else if segment.startsWith("A") then
        areaId = Some(segment.drop(1).toInt)
      else
        throw IllegalArgumentException(s"Unrecognized hierarchy segment: $segment")

    require(partnerId.isDefined, s"Hierarchy path missing Partner (P): $path")
    require(companyId.isDefined, s"Hierarchy path missing Company (C): $path")

    HierarchyIds(
      partnerId = partnerId.get,
      companyId = companyId.get,
      propertyId = propertyId,
      groupId = groupId,
      buildingId = buildingId,
      areaId = areaId
    )
```

Note: `PR` is matched before `P` to avoid `P` consuming `PR` prefixes.

- [ ] **Step 4: Run tests to verify they pass**

Run: `sbt "testOnly com.enity.flink.enrichment.HierarchyPathParserSpec"`
Expected: all 11 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/scala/com/enity/flink/enrichment/HierarchyPathParser.scala \
        src/test/scala/com/enity/flink/enrichment/HierarchyPathParserSpec.scala
git commit -m "feat: add hierarchy path parser with tests"
```

---

## Task 4: DDB Streams Deserializer

**Files:**
- Create: `src/main/scala/com/enity/flink/enrichment/DdbStreamDeserializer.scala`
- Test: `src/test/scala/com/enity/flink/enrichment/DdbStreamDeserializerSpec.scala`

- [ ] **Step 1: Write the tests**

Create `src/test/scala/com/enity/flink/enrichment/DdbStreamDeserializerSpec.scala`:

```scala
package com.enity.flink.enrichment

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers
import java.nio.charset.StandardCharsets
import java.util.{Base64, UUID}
import java.nio.ByteBuffer

class DdbStreamDeserializerSpec extends AnyFlatSpec with Matchers {

  private val deserializer = new DdbStreamDeserializer()

  private def uuidToBase64(uuid: UUID): String =
    val bb = ByteBuffer.wrap(new Array[Byte](16))
    bb.putLong(uuid.getMostSignificantBits)
    bb.putLong(uuid.getLeastSignificantBits)
    Base64.getEncoder.encodeToString(bb.array())

  private def makeJson(eventName: String, image: String, imageKey: String = "NewImage"): Array[Byte] =
    s"""{
       |  "eventName": "$eventName",
       |  "dynamodb": {
       |    "$imageKey": $image
       |  }
       |}""".stripMargin.getBytes(StandardCharsets.UTF_8)

  "DdbStreamDeserializer" should "parse INSERT event" in {
    val uuid = UUID.fromString("550e8400-e29b-41d4-a716-446655440000")
    val b64 = uuidToBase64(uuid)
    val image = s"""{
      "pk": {"S": "04821"},
      "sk": {"S": "daq:std_json_v1:cust:meter1:temp"},
      "logical_id": {"B": "$b64"},
      "meter_type": {"S": "gauge"},
      "hierarchy_path": {"S": "P1#C2#B8#A3"}
    }"""
    val result = deserializer.deserialize(makeJson("INSERT", image))

    result.eventType shouldBe "INSERT"
    result.daqId shouldBe "daq:std_json_v1:cust:meter1:temp"
    result.mapping shouldBe defined
    val m = result.mapping.get
    m.logicalId shouldBe "550e8400-e29b-41d4-a716-446655440000"
    m.meterType shouldBe "gauge"
    m.partnerId shouldBe 1
    m.companyId shouldBe 2
    m.buildingId shouldBe Some(8)
    m.areaId shouldBe Some(3)
  }

  it should "parse MODIFY event" in {
    val uuid = UUID.randomUUID()
    val b64 = uuidToBase64(uuid)
    val image = s"""{
      "pk": {"S": "00001"},
      "sk": {"S": "daq:emu:cust:m2:energy"},
      "logical_id": {"B": "$b64"},
      "meter_type": {"S": "counter"},
      "hierarchy_path": {"S": "P5#C10#PR3#B7"}
    }"""
    val result = deserializer.deserialize(makeJson("MODIFY", image))

    result.eventType shouldBe "MODIFY"
    result.daqId shouldBe "daq:emu:cust:m2:energy"
    result.mapping.get.meterType shouldBe "counter"
    result.mapping.get.propertyId shouldBe Some(3)
    result.mapping.get.buildingId shouldBe Some(7)
  }

  it should "parse REMOVE event using OldImage" in {
    val image = s"""{
      "pk": {"S": "00001"},
      "sk": {"S": "daq:std:cust:m3:temp"}
    }"""
    val result = deserializer.deserialize(makeJson("REMOVE", image, "OldImage"))

    result.eventType shouldBe "REMOVE"
    result.daqId shouldBe "daq:std:cust:m3:temp"
    result.mapping shouldBe None
  }

  it should "correctly decode binary UUID" in {
    val uuid = UUID.fromString("12345678-1234-1234-1234-123456789abc")
    val b64 = uuidToBase64(uuid)
    val image = s"""{
      "pk": {"S": "00001"},
      "sk": {"S": "daq:x:c:m:s"},
      "logical_id": {"B": "$b64"},
      "meter_type": {"S": "gauge"},
      "hierarchy_path": {"S": "P1#C1#B1"}
    }"""
    val result = deserializer.deserialize(makeJson("INSERT", image))
    result.mapping.get.logicalId shouldBe "12345678-1234-1234-1234-123456789abc"
  }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `sbt "testOnly com.enity.flink.enrichment.DdbStreamDeserializerSpec"`
Expected: FAIL — class not found

- [ ] **Step 3: Implement DdbStreamDeserializer**

Create `src/main/scala/com/enity/flink/enrichment/DdbStreamDeserializer.scala`:

```scala
package com.enity.flink.enrichment

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.scala.DefaultScalaModule
import org.apache.flink.api.common.serialization.DeserializationSchema
import org.apache.flink.api.common.typeinfo.TypeInformation
import org.slf4j.LoggerFactory

import java.nio.ByteBuffer
import java.util.{Base64, UUID}

/** Deserializes DynamoDB Streams JSON records (via Kinesis adapter) into IdMappingChange. */
class DdbStreamDeserializer extends DeserializationSchema[IdMappingChange]:
  @transient private lazy val logger = LoggerFactory.getLogger(getClass)
  @transient private lazy val mapper =
    val m = new ObjectMapper()
    m.registerModule(DefaultScalaModule)
    m

  override def deserialize(message: Array[Byte]): IdMappingChange =
    val record = mapper.readValue(message, classOf[Map[String, Any]])
    val eventName = record("eventName").toString
    val dynamodb = record("dynamodb").asInstanceOf[Map[String, Any]]

    eventName match
      case "REMOVE" =>
        val oldImage = dynamodb("OldImage").asInstanceOf[Map[String, Any]]
        val daqId = extractString(oldImage, "sk")
        IdMappingChange("REMOVE", daqId, None)

      case "INSERT" | "MODIFY" =>
        val newImage = dynamodb("NewImage").asInstanceOf[Map[String, Any]]
        val daqId = extractString(newImage, "sk")
        val logicalId = extractBinaryUuid(newImage, "logical_id")
        val meterType = extractString(newImage, "meter_type")
        val hierarchyPath = extractString(newImage, "hierarchy_path")
        val ids = HierarchyPathParser.parse(hierarchyPath)

        val mapping = MeterMapping(
          logicalId = logicalId,
          meterType = meterType,
          partnerId = ids.partnerId,
          companyId = ids.companyId,
          propertyId = ids.propertyId,
          buildingId = ids.buildingId,
          areaId = ids.areaId,
          groupId = ids.groupId
        )
        IdMappingChange(eventName, daqId, Some(mapping))

      case other =>
        logger.warn(s"Unknown DDB Streams event type: $other")
        IdMappingChange(other, "", None)

  override def isEndOfStream(nextElement: IdMappingChange): Boolean = false

  override def getProducedType: TypeInformation[IdMappingChange] =
    TypeInformation.of(classOf[IdMappingChange])

  private def extractString(image: Map[String, Any], field: String): String =
    image(field).asInstanceOf[Map[String, Any]]("S").toString

  private def extractBinaryUuid(image: Map[String, Any], field: String): String =
    val b64 = image(field).asInstanceOf[Map[String, Any]]("B").toString
    val bytes = Base64.getDecoder.decode(b64)
    val bb = ByteBuffer.wrap(bytes)
    new UUID(bb.getLong, bb.getLong).toString
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `sbt "testOnly com.enity.flink.enrichment.DdbStreamDeserializerSpec"`
Expected: all 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/scala/com/enity/flink/enrichment/DdbStreamDeserializer.scala \
        src/test/scala/com/enity/flink/enrichment/DdbStreamDeserializerSpec.scala
git commit -m "feat: add DDB Streams deserializer with tests"
```

---

## Task 5: CounterDeltaFunction

**Files:**
- Create: `src/main/scala/com/enity/flink/enrichment/CounterDeltaFunction.scala`
- Test: `src/test/scala/com/enity/flink/enrichment/CounterDeltaFunctionSpec.scala`

- [ ] **Step 1: Write the tests**

Create `src/test/scala/com/enity/flink/enrichment/CounterDeltaFunctionSpec.scala`:

```scala
package com.enity.flink.enrichment

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class CounterDeltaFunctionSpec extends AnyFlatSpec with Matchers {

  private def makeEnriched(value: Double, meterType: String = "gauge", ts: String = "2026-03-27T10:00:00Z"): EnrichedRecord =
    EnrichedRecord(
      meterId = "550e8400-e29b-41d4-a716-446655440000",
      timestamp = ts,
      value = value,
      unit = "kWh",
      created = "2026-03-27T10:00:01Z",
      partnerId = 1, companyId = 2,
      propertyId = None, buildingId = Some(8), areaId = None, groupId = None
    )

  "CounterDeltaFunction.computeDelta" should "pass through gauge values unchanged" in {
    val record = makeEnriched(100.0, "gauge")
    val (result, anomaly) = CounterDeltaFunction.computeDelta(record, "gauge", None)
    result shouldBe defined
    result.get.value shouldBe 100.0
    anomaly shouldBe false
  }

  it should "drop the first counter message (no baseline)" in {
    val record = makeEnriched(100.0, "counter")
    val (result, anomaly) = CounterDeltaFunction.computeDelta(record, "counter", None)
    result shouldBe None
    anomaly shouldBe false
  }

  it should "compute positive delta for counter" in {
    val record = makeEnriched(150.0, "counter", "2026-03-27T10:05:00Z")
    val prevState = Some(CounterState(100.0, java.time.Instant.parse("2026-03-27T10:00:00Z")))
    val (result, anomaly) = CounterDeltaFunction.computeDelta(record, "counter", prevState)
    result shouldBe defined
    result.get.value shouldBe 50.0
    anomaly shouldBe false
  }

  it should "compute zero delta for counter (same value)" in {
    val record = makeEnriched(100.0, "counter", "2026-03-27T10:05:00Z")
    val prevState = Some(CounterState(100.0, java.time.Instant.parse("2026-03-27T10:00:00Z")))
    val (result, anomaly) = CounterDeltaFunction.computeDelta(record, "counter", prevState)
    result shouldBe defined
    result.get.value shouldBe 0.0
    anomaly shouldBe false
  }

  it should "flag negative delta as anomaly" in {
    val record = makeEnriched(50.0, "counter", "2026-03-27T10:05:00Z")
    val prevState = Some(CounterState(100.0, java.time.Instant.parse("2026-03-27T10:00:00Z")))
    val (result, anomaly) = CounterDeltaFunction.computeDelta(record, "counter", prevState)
    result shouldBe None
    anomaly shouldBe true
  }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `sbt "testOnly com.enity.flink.enrichment.CounterDeltaFunctionSpec"`
Expected: FAIL — class not found

- [ ] **Step 3: Implement CounterDeltaFunction**

Create `src/main/scala/com/enity/flink/enrichment/CounterDeltaFunction.scala`:

```scala
package com.enity.flink.enrichment

import org.apache.flink.api.common.state.{ValueState, ValueStateDescriptor}
import org.apache.flink.api.common.typeinfo.TypeInformation
import org.apache.flink.configuration.Configuration
import org.apache.flink.streaming.api.functions.KeyedProcessFunction
import org.apache.flink.util.Collector
import org.slf4j.LoggerFactory

import java.time.Instant

/** Keyed by daqId. Receives (EnrichedRecord, meterType) tuples.
  * Gauge: pass through. Counter: compute delta, drop first message, flag negative deltas. */
class CounterDeltaFunction
    extends KeyedProcessFunction[String, (EnrichedRecord, String), EnrichedRecord]:

  @transient private lazy val logger = LoggerFactory.getLogger(getClass)
  @transient private var counterState: ValueState[CounterState] = _

  override def open(parameters: Configuration): Unit =
    val descriptor = new ValueStateDescriptor[CounterState](
      "counter-state",
      TypeInformation.of(classOf[CounterState])
    )
    counterState = getRuntimeContext.getState(descriptor)

  override def processElement(
      value: (EnrichedRecord, String),
      ctx: KeyedProcessFunction[String, (EnrichedRecord, String), EnrichedRecord]#Context,
      out: Collector[EnrichedRecord]
  ): Unit =
    val (record, meterType) = value
    val prevState = Option(counterState.value())

    val (result, isAnomaly) = CounterDeltaFunction.computeDelta(record, meterType, prevState)

    // Update state for counters
    if meterType == "counter" then
      counterState.update(CounterState(
        lastValue = record.value,
        lastTimestamp = Instant.parse(record.timestamp)
      ))

    if isAnomaly then
      val errorRecord = ErrorRecord(
        errorType = "anomaly",
        timestamp = record.timestamp,
        daqId = ctx.getCurrentKey,
        payload = s"value=${record.value}, previous=${prevState.map(_.lastValue).getOrElse("none")}",
        error = s"Negative counter delta: ${record.value} - ${prevState.map(_.lastValue).getOrElse(0.0)}"
      )
      ctx.output(SideOutputTags.ANOMALY, errorRecord)

    result.foreach(out.collect)

object CounterDeltaFunction:
  /** Pure function for testability. Returns (Option[record to emit], isAnomaly). */
  def computeDelta(
      record: EnrichedRecord,
      meterType: String,
      prevState: Option[CounterState]
  ): (Option[EnrichedRecord], Boolean) =
    meterType match
      case "gauge" =>
        (Some(record), false)
      case "counter" =>
        prevState match
          case None =>
            (None, false) // first message, no baseline
          case Some(prev) =>
            val delta = record.value - prev.lastValue
            if delta >= 0 then
              (Some(record.copy(value = delta)), false)
            else
              (None, true) // negative delta = anomaly
      case _ =>
        (Some(record), false) // unknown type, pass through
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `sbt "testOnly com.enity.flink.enrichment.CounterDeltaFunctionSpec"`
Expected: all 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/scala/com/enity/flink/enrichment/CounterDeltaFunction.scala \
        src/test/scala/com/enity/flink/enrichment/CounterDeltaFunctionSpec.scala
git commit -m "feat: add counter delta function with tests"
```

---

## Task 6: MeterEnrichmentFunction

**Files:**
- Create: `src/main/scala/com/enity/flink/enrichment/MeterEnrichmentFunction.scala`
- Test: `src/test/scala/com/enity/flink/enrichment/MeterEnrichmentFunctionSpec.scala`

- [ ] **Step 1: Write the tests**

Create `src/test/scala/com/enity/flink/enrichment/MeterEnrichmentFunctionSpec.scala`:

```scala
package com.enity.flink.enrichment

import com.enity.flink.SensorRecord
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class MeterEnrichmentFunctionSpec extends AnyFlatSpec with Matchers {

  private val testMapping = MeterMapping(
    logicalId = "550e8400-e29b-41d4-a716-446655440000",
    meterType = "gauge",
    partnerId = 1, companyId = 2,
    propertyId = None, buildingId = Some(8), areaId = Some(3), groupId = None
  )

  private val testRecord = SensorRecord(
    daqId = "daq:std_json_v1:cust:meter1:temp",
    `type` = "std_json_v1", gatewayId = "gw1", meterId = "meter1",
    timestamp = "2026-03-27T10:00:00Z", created = "2026-03-27T10:00:01Z",
    sensorId = "temp", value = "21.5", unit = "C"
  )

  "MeterEnrichmentFunction.enrich" should "produce EnrichedRecord when mapping exists" in {
    val result = MeterEnrichmentFunction.enrich(testRecord, Some(testMapping))
    result shouldBe defined
    val enriched = result.get
    enriched.meterId shouldBe "550e8400-e29b-41d4-a716-446655440000"
    enriched.value shouldBe 21.5
    enriched.unit shouldBe "C"
    enriched.partnerId shouldBe 1
    enriched.companyId shouldBe 2
    enriched.buildingId shouldBe Some(8)
    enriched.areaId shouldBe Some(3)
    enriched.propertyId shouldBe None
    enriched.groupId shouldBe None
  }

  it should "return None when no mapping exists" in {
    val result = MeterEnrichmentFunction.enrich(testRecord, None)
    result shouldBe None
  }

  it should "correctly parse string value to double" in {
    val record = testRecord.copy(value = "123.456")
    val result = MeterEnrichmentFunction.enrich(record, Some(testMapping))
    result.get.value shouldBe 123.456
  }

  it should "preserve all hierarchy fields from mapping" in {
    val fullMapping = testMapping.copy(
      propertyId = Some(4), groupId = Some(5), buildingId = Some(8), areaId = Some(3)
    )
    val result = MeterEnrichmentFunction.enrich(testRecord, Some(fullMapping))
    val e = result.get
    e.propertyId shouldBe Some(4)
    e.groupId shouldBe Some(5)
    e.buildingId shouldBe Some(8)
    e.areaId shouldBe Some(3)
  }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `sbt "testOnly com.enity.flink.enrichment.MeterEnrichmentFunctionSpec"`
Expected: FAIL — class not found

- [ ] **Step 3: Implement MeterEnrichmentFunction**

Create `src/main/scala/com/enity/flink/enrichment/MeterEnrichmentFunction.scala`:

```scala
package com.enity.flink.enrichment

import com.enity.flink.SensorRecord
import org.apache.flink.api.common.state.MapStateDescriptor
import org.apache.flink.api.common.typeinfo.{BasicTypeInfo, TypeInformation}
import org.apache.flink.streaming.api.functions.co.BroadcastProcessFunction
import org.apache.flink.util.Collector
import org.slf4j.LoggerFactory

import java.time.Instant

/** BroadcastProcessFunction that enriches SensorRecords with meter identity and hierarchy context.
  *
  * Main input: SensorRecord stream
  * Broadcast input: IdMappingChange stream (from DDB Streams)
  * Output: (EnrichedRecord, meterType) tuples — meterType passed along for CounterDeltaFunction */
class MeterEnrichmentFunction
    extends BroadcastProcessFunction[SensorRecord, IdMappingChange, (EnrichedRecord, String)]:

  @transient private lazy val logger = LoggerFactory.getLogger(getClass)

  override def processElement(
      record: SensorRecord,
      ctx: BroadcastProcessFunction[SensorRecord, IdMappingChange, (EnrichedRecord, String)]#ReadOnlyContext,
      out: Collector[(EnrichedRecord, String)]
  ): Unit =
    val state = ctx.getBroadcastState(MeterEnrichmentFunction.ID_MAP)
    val mapping = Option(state.get(record.daqId))

    MeterEnrichmentFunction.enrich(record, mapping) match
      case Some(enriched) =>
        out.collect((enriched, mapping.get.meterType))
      case None =>
        val errorRecord = ErrorRecord(
          errorType = "dead_letter",
          timestamp = Instant.now().toString,
          daqId = record.daqId,
          payload = s"value=${record.value}, unit=${record.unit}, ts=${record.timestamp}",
          error = "No meter mapping found in broadcast state"
        )
        ctx.output(SideOutputTags.DEAD_LETTER, errorRecord)

  override def processBroadcastElement(
      change: IdMappingChange,
      ctx: BroadcastProcessFunction[SensorRecord, IdMappingChange, (EnrichedRecord, String)]#Context,
      out: Collector[(EnrichedRecord, String)]
  ): Unit =
    val state = ctx.getBroadcastState(MeterEnrichmentFunction.ID_MAP)
    change.eventType match
      case "INSERT" | "MODIFY" =>
        change.mapping.foreach(m => state.put(change.daqId, m))
        logger.debug(s"Updated mapping for ${change.daqId}")
      case "REMOVE" =>
        state.remove(change.daqId)
        logger.debug(s"Removed mapping for ${change.daqId}")
      case other =>
        logger.warn(s"Unknown DDB event type: $other for ${change.daqId}")

object MeterEnrichmentFunction:

  val ID_MAP: MapStateDescriptor[String, MeterMapping] =
    new MapStateDescriptor[String, MeterMapping](
      "meter-id-map",
      BasicTypeInfo.STRING_TYPE_INFO,
      TypeInformation.of(classOf[MeterMapping])
    )

  /** Pure function for testability. Returns Some(enriched) if mapping exists, None otherwise. */
  def enrich(record: SensorRecord, mapping: Option[MeterMapping]): Option[EnrichedRecord] =
    mapping.map { m =>
      EnrichedRecord(
        meterId = m.logicalId,
        timestamp = record.timestamp,
        value = record.value.toDouble,
        unit = record.unit,
        created = record.created,
        partnerId = m.partnerId,
        companyId = m.companyId,
        propertyId = m.propertyId,
        buildingId = m.buildingId,
        areaId = m.areaId,
        groupId = m.groupId
      )
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `sbt "testOnly com.enity.flink.enrichment.MeterEnrichmentFunctionSpec"`
Expected: all 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/scala/com/enity/flink/enrichment/MeterEnrichmentFunction.scala \
        src/test/scala/com/enity/flink/enrichment/MeterEnrichmentFunctionSpec.scala
git commit -m "feat: add meter enrichment broadcast function with tests"
```

---

## Task 7: DdbBootstrapLoader

**Files:**
- Create: `src/main/scala/com/enity/flink/enrichment/DdbBootstrapLoader.scala`
- Test: `src/test/scala/com/enity/flink/enrichment/DdbBootstrapLoaderSpec.scala`

- [ ] **Step 1: Write the tests**

Create `src/test/scala/com/enity/flink/enrichment/DdbBootstrapLoaderSpec.scala`:

```scala
package com.enity.flink.enrichment

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class DdbBootstrapLoaderSpec extends AnyFlatSpec with Matchers {

  "DdbBootstrapLoader.zeroPad" should "pad single digit to 5 chars" in {
    DdbBootstrapLoader.zeroPad(1) shouldBe "00001"
  }

  it should "pad large number" in {
    DdbBootstrapLoader.zeroPad(19999) shouldBe "19999"
  }

  it should "pad zero" in {
    DdbBootstrapLoader.zeroPad(0) shouldBe "00000"
  }

  "DdbBootstrapLoader.partitionKey" should "compute consistent partition" in {
    val daqId = "daq:std_json_v1:customer:meter:sensor"
    val pk = DdbBootstrapLoader.partitionKey(daqId)
    pk.length shouldBe 5
    pk.toInt should be >= 0
    pk.toInt should be < 20000
  }

  it should "produce same result for same input" in {
    val daqId = "daq:test:cust:m1:s1"
    DdbBootstrapLoader.partitionKey(daqId) shouldBe DdbBootstrapLoader.partitionKey(daqId)
  }

  "DdbBootstrapLoader.parseDdbItem" should "parse a DynamoDB item map" in {
    val uuid = java.util.UUID.fromString("550e8400-e29b-41d4-a716-446655440000")
    val bb = java.nio.ByteBuffer.wrap(new Array[Byte](16))
    bb.putLong(uuid.getMostSignificantBits)
    bb.putLong(uuid.getLeastSignificantBits)
    val sdkBytes = software.amazon.awssdk.core.SdkBytes.fromByteArray(bb.array())

    import software.amazon.awssdk.services.dynamodb.model.AttributeValue
    val item = java.util.Map.of(
      "pk", AttributeValue.builder().s("04821").build(),
      "sk", AttributeValue.builder().s("daq:std:cust:m1:temp").build(),
      "logical_id", AttributeValue.builder().b(sdkBytes).build(),
      "meter_type", AttributeValue.builder().s("gauge").build(),
      "hierarchy_path", AttributeValue.builder().s("P1#C2#B8#A3").build()
    )

    val (daqId, mapping) = DdbBootstrapLoader.parseDdbItem(item)
    daqId shouldBe "daq:std:cust:m1:temp"
    mapping.logicalId shouldBe "550e8400-e29b-41d4-a716-446655440000"
    mapping.meterType shouldBe "gauge"
    mapping.partnerId shouldBe 1
    mapping.companyId shouldBe 2
    mapping.buildingId shouldBe Some(8)
    mapping.areaId shouldBe Some(3)
  }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `sbt "testOnly com.enity.flink.enrichment.DdbBootstrapLoaderSpec"`
Expected: FAIL — class not found

- [ ] **Step 3: Implement DdbBootstrapLoader**

Create `src/main/scala/com/enity/flink/enrichment/DdbBootstrapLoader.scala`:

```scala
package com.enity.flink.enrichment

import org.slf4j.LoggerFactory
import software.amazon.awssdk.core.SdkBytes
import software.amazon.awssdk.services.dynamodb.DynamoDbClient
import software.amazon.awssdk.services.dynamodb.model.{AttributeValue, QueryRequest}

import java.nio.ByteBuffer
import java.util.UUID
import java.util.concurrent.{ConcurrentHashMap, Executors, TimeUnit}
import scala.jdk.CollectionConverters.*

object DdbBootstrapLoader:
  private val logger = LoggerFactory.getLogger(getClass)
  private val NUM_PARTITIONS = 20_000
  private val THREAD_POOL_SIZE = 20

  def zeroPad(n: Int): String = f"$n%05d"

  def partitionKey(daqId: String): String =
    zeroPad(Math.abs(daqId.hashCode) % NUM_PARTITIONS)

  def parseDdbItem(item: java.util.Map[String, AttributeValue]): (String, MeterMapping) =
    val daqId = item.get("sk").s()
    val logicalIdBytes = item.get("logical_id").b().asByteArray()
    val bb = ByteBuffer.wrap(logicalIdBytes)
    val logicalId = new UUID(bb.getLong, bb.getLong).toString
    val meterType = item.get("meter_type").s()
    val hierarchyPath = item.get("hierarchy_path").s()
    val ids = HierarchyPathParser.parse(hierarchyPath)

    val mapping = MeterMapping(
      logicalId = logicalId,
      meterType = meterType,
      partnerId = ids.partnerId,
      companyId = ids.companyId,
      propertyId = ids.propertyId,
      buildingId = ids.buildingId,
      areaId = ids.areaId,
      groupId = ids.groupId
    )
    (daqId, mapping)

  /** Scan all partitions in parallel and return a map of daqId → MeterMapping. */
  def loadAll(client: DynamoDbClient, tableName: String): Map[String, MeterMapping] =
    val startTime = System.currentTimeMillis()
    val buffer = new ConcurrentHashMap[String, MeterMapping]()
    val executor = Executors.newFixedThreadPool(THREAD_POOL_SIZE)

    val futures = (0 until NUM_PARTITIONS).map { partitionId =>
      executor.submit(new Runnable:
        override def run(): Unit =
          var lastEvaluatedKey: java.util.Map[String, AttributeValue] = null
          do
            val requestBuilder = QueryRequest.builder()
              .tableName(tableName)
              .keyConditionExpression("pk = :pk")
              .expressionAttributeValues(
                java.util.Map.of(":pk", AttributeValue.builder().s(zeroPad(partitionId)).build())
              )
            if lastEvaluatedKey != null then
              requestBuilder.exclusiveStartKey(lastEvaluatedKey)

            val response = client.query(requestBuilder.build())
            response.items().asScala.foreach { item =>
              val (daqId, mapping) = parseDdbItem(item)
              buffer.put(daqId, mapping)
            }
            lastEvaluatedKey = if response.hasLastEvaluatedKey then response.lastEvaluatedKey() else null
          while lastEvaluatedKey != null
      )
    }

    // Wait for all partitions — fail fast on any exception
    futures.foreach(_.get())
    executor.shutdown()
    executor.awaitTermination(5, TimeUnit.MINUTES)

    val elapsed = System.currentTimeMillis() - startTime
    logger.info(s"Bootstrap loaded ${buffer.size()} meter mappings in ${elapsed}ms")
    buffer.asScala.toMap
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `sbt "testOnly com.enity.flink.enrichment.DdbBootstrapLoaderSpec"`
Expected: all 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/scala/com/enity/flink/enrichment/DdbBootstrapLoader.scala \
        src/test/scala/com/enity/flink/enrichment/DdbBootstrapLoaderSpec.scala
git commit -m "feat: add DynamoDB bootstrap loader with tests"
```

---

## Task 8: Wire up Main.scala

**Files:**
- Modify: `src/main/scala/com/enity/flink/Main.scala`

This is the largest change. It modifies `createFlinkJob()` to:
1. Refactor parsing into a `ProcessFunction` that emits `PARSE_ERROR` side outputs
2. Fork the parsed stream: raw branch (unchanged) and enrichment branch
3. Add DDB Streams source + broadcast
4. Connect enrichment → counter delta → second Iceberg sink
5. Collect all side outputs → error Kinesis sink

- [ ] **Step 1: Add imports to Main.scala**

Add these imports at the top of `Main.scala` after the existing imports:

```scala
import com.enity.flink.enrichment.*
import org.apache.flink.streaming.api.functions.ProcessFunction
import org.apache.flink.api.common.typeinfo.TypeInformation
import org.apache.flink.streaming.connectors.kinesis.FlinkKinesisProducer
```

- [ ] **Step 2: Replace the `flatMap` + sink section in `createFlinkJob()`**

Replace lines 175-206 (from `// Build the processing pipeline` to `env.execute(...)`) with:

```scala
    // Read new environment properties
    val meterIdentityTable = properties.getOrElse("METER_IDENTITY_TABLE", "meter-identity")
    val ddbStreamArn = properties.getOrElse("DDB_STREAM_ARN", "")
    val errorStreamName = properties.getOrElse("ERROR_STREAM", "")

    // ── Step 1: Parse JSON with side output for errors ──

    val parsedStream = env
      .addSource(source).returns(Types.STRING)
      .uid("kinesis-source")
      .process(new ProcessFunction[String, SensorRecord] {
        override def processElement(
            value: String,
            ctx: ProcessFunction[String, SensorRecord]#Context,
            out: Collector[SensorRecord]
        ): Unit =
          try
            val records = processJsonToRecords(value)
            if records.isEmpty then
              ctx.output(SideOutputTags.PARSE_ERROR, ErrorRecord(
                errorType = "parse_error",
                timestamp = Instant.now().toString,
                daqId = "",
                payload = value.take(1000),
                error = "No valid records produced"
              ))
            else
              records.foreach(out.collect)
          catch
            case e: Exception =>
              ctx.output(SideOutputTags.PARSE_ERROR, ErrorRecord(
                errorType = "parse_error",
                timestamp = Instant.now().toString,
                daqId = "",
                payload = value.take(1000),
                error = e.getMessage
              ))
      })
      .uid("json-parser")

    val parseErrors = parsedStream.getSideOutput(SideOutputTags.PARSE_ERROR)

    // ── Step 2: Raw Iceberg sink (unchanged logic) ──

    val rawStream = parsedStream
      .map { record =>
        Row.of(
          record.daqId,
          parseInstant(record.timestamp),
          java.lang.Double.valueOf(record.value.toDouble),
          record.unit,
          parseInstant(record.created)
        )
      }(Types.ROW_NAMED(
        Array("daq_id", "timestamp", "value", "unit", "created"),
        Types.STRING, Types.INSTANT, Types.DOUBLE, Types.STRING, Types.INSTANT
      ))
      .uid("raw-row-mapper")

    FlinkSink.forRow(rawStream, tableSchema)
      .tableLoader(tableLoader)
      .distributionMode(DistributionMode.NONE)
      .writeParallelism(1)
      .append()

    // ── Step 3: DDB Streams source for broadcast ──

    val ddbConsumerConfig = new Properties()
    ddbConsumerConfig.setProperty("aws.region", region)
    ddbConsumerConfig.setProperty(ConsumerConfigConstants.STREAM_INITIAL_POSITION, "TRIM_HORIZON")
    ddbConsumerConfig.setProperty("aws.credentials.provider", "AUTO")
    ddbConsumerConfig.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_MAX, "100")
    ddbConsumerConfig.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_RETRIES, "10")
    ddbConsumerConfig.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_BACKOFF_BASE, "1000")
    ddbConsumerConfig.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_BACKOFF_MAX, "5000")

    val ddbSource = new FlinkKinesisConsumer[IdMappingChange](
      ddbStreamArn,
      new DdbStreamDeserializer(),
      ddbConsumerConfig
    )

    val mappingStream = env
      .addSource(ddbSource)(TypeInformation.of(classOf[IdMappingChange]))
      .uid("ddb-streams-source")

    val broadcastStream = mappingStream.broadcast(MeterEnrichmentFunction.ID_MAP)

    // ── Step 4: Enrichment branch ──

    val enrichedStream = parsedStream
      .connect(broadcastStream)
      .process(new MeterEnrichmentFunction())
      .uid("meter-enrichment")

    val deadLetters = enrichedStream.getSideOutput(SideOutputTags.DEAD_LETTER)

    // ── Step 5: Counter delta ──

    val deltaStream = enrichedStream
      .keyBy(_._1.meterId) // key by logical meter ID
      .process(new CounterDeltaFunction())
      .uid("counter-delta")

    val anomalies = deltaStream.getSideOutput(SideOutputTags.ANOMALY)

    // ── Step 6: Enriched Iceberg sink ──

    val enrichedTableSchema = TableSchema.builder()
      .field("meter_id", DataTypes.STRING().notNull())
      .field("timestamp", DataTypes.TIMESTAMP_WITH_LOCAL_TIME_ZONE(6).notNull())
      .field("value", DataTypes.DOUBLE().notNull())
      .field("unit", DataTypes.STRING().notNull())
      .field("created", DataTypes.TIMESTAMP_WITH_LOCAL_TIME_ZONE(6).notNull())
      .field("partner_id", DataTypes.INT().notNull())
      .field("company_id", DataTypes.INT().notNull())
      .field("property_id", DataTypes.INT())
      .field("building_id", DataTypes.INT())
      .field("area_id", DataTypes.INT())
      .field("group_id", DataTypes.INT())
      .build()

    val enrichedTableId = TableIdentifier.of(Namespace.of("all"), "nested_meter_readings_cdk2")
    val enrichedTableLoader = TableLoader.fromCatalog(catalogLoader, enrichedTableId)

    val enrichedRowStream = deltaStream
      .map { record =>
        Row.of(
          record.meterId,
          parseInstant(record.timestamp),
          java.lang.Double.valueOf(record.value),
          record.unit,
          parseInstant(record.created),
          java.lang.Integer.valueOf(record.partnerId),
          java.lang.Integer.valueOf(record.companyId),
          record.propertyId.map(java.lang.Integer.valueOf).orNull,
          record.buildingId.map(java.lang.Integer.valueOf).orNull,
          record.areaId.map(java.lang.Integer.valueOf).orNull,
          record.groupId.map(java.lang.Integer.valueOf).orNull
        )
      }(Types.ROW_NAMED(
        Array("meter_id", "timestamp", "value", "unit", "created",
              "partner_id", "company_id", "property_id", "building_id", "area_id", "group_id"),
        Types.STRING, Types.INSTANT, Types.DOUBLE, Types.STRING, Types.INSTANT,
        Types.INT, Types.INT, Types.INT, Types.INT, Types.INT, Types.INT
      ))
      .uid("enriched-row-mapper")

    FlinkSink.forRow(enrichedRowStream, enrichedTableSchema)
      .tableLoader(enrichedTableLoader)
      .distributionMode(DistributionMode.NONE)
      .writeParallelism(1)
      .append()

    // ── Step 7: Error sink (all side outputs → single Kinesis stream) ──

    if errorStreamName.nonEmpty then
      val errorMapper = new ObjectMapper()
      errorMapper.registerModule(DefaultScalaModule)

      val allErrors = parseErrors.union(deadLetters).union(anomalies)

      val errorProducerConfig = new Properties()
      errorProducerConfig.setProperty("aws.region", region)
      errorProducerConfig.setProperty("AggregationEnabled", "false")

      val errorSink = new FlinkKinesisProducer[ErrorRecord](
        new org.apache.flink.api.common.serialization.SerializationSchema[ErrorRecord] {
          @transient private lazy val om = {
            val m = new ObjectMapper(); m.registerModule(DefaultScalaModule); m
          }
          override def serialize(element: ErrorRecord): Array[Byte] =
            om.writeValueAsBytes(element)
        },
        errorProducerConfig
      )
      errorSink.setDefaultStream(errorStreamName)
      errorSink.setDefaultPartition("0")

      allErrors.addSink(errorSink).uid("error-sink")

    env.execute("DAQ Meter Enrichment Pipeline")
```

- [ ] **Step 3: Verify it compiles**

Run: `sbt compile`
Expected: BUILD SUCCESS

- [ ] **Step 4: Run existing tests to verify no regressions**

Run: `sbt test`
Expected: All existing processor tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/scala/com/enity/flink/Main.scala
git commit -m "feat: wire up enrichment pipeline with side outputs and dual Iceberg sinks"
```

---

## Task 9: CDK infrastructure changes

**Files:**
- Modify: `lib/flink_transforms-stack.ts`

- [ ] **Step 1: Add DynamoDB and Kinesis imports**

Add to imports in `flink_transforms-stack.ts`:

```typescript
import * as dynamodb from "aws-cdk-lib/aws-dynamodb";
```

- [ ] **Step 2: Add DynamoDB table, error stream, IAM permissions, and env properties**

After the `inputStream` reference (line 30), add:

```typescript
    // Create meter-identity DynamoDB table
    const meterIdentityTable = new dynamodb.Table(this, "MeterIdentity", {
      tableName: "meter-identity",
      partitionKey: { name: "pk", type: dynamodb.AttributeType.STRING },
      sortKey: { name: "sk", type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      stream: dynamodb.StreamViewType.NEW_AND_OLD_IMAGES,
      pointInTimeRecovery: true,
      removalPolicy: RemovalPolicy.RETAIN,
    });

    // Create error Kinesis stream
    const errorStream = new kinesis.Stream(this, "ErrorStream", {
      streamName: `${props.appName}-errors`,
      shardCount: 1,
      retentionPeriod: cdk.Duration.hours(24),
    });
```

After the existing S3 Tables permissions block (after line 103), add:

```typescript
    // DynamoDB permissions for bootstrap scan
    serviceRole.addToPolicy(
      new iam.PolicyStatement({
        effect: iam.Effect.ALLOW,
        actions: ["dynamodb:Query"],
        resources: [meterIdentityTable.tableArn],
      }),
    );

    // DynamoDB Streams permissions
    serviceRole.addToPolicy(
      new iam.PolicyStatement({
        effect: iam.Effect.ALLOW,
        actions: [
          "dynamodb:DescribeStream",
          "dynamodb:GetRecords",
          "dynamodb:GetShardIterator",
          "dynamodb:ListStreams",
        ],
        resources: [`${meterIdentityTable.tableArn}/stream/*`],
      }),
    );

    // Error stream write permissions
    serviceRole.addToPolicy(
      new iam.PolicyStatement({
        effect: iam.Effect.ALLOW,
        actions: [
          "kinesis:PutRecord",
          "kinesis:PutRecords",
          "kinesis:DescribeStream",
          "kinesis:DescribeStreamSummary",
        ],
        resources: [errorStream.streamArn],
      }),
    );
```

Add new environment properties to the `FlinkApplicationProperties` propertyMap:

```typescript
                METER_IDENTITY_TABLE: meterIdentityTable.tableName,
                DDB_STREAM_ARN: meterIdentityTable.tableStreamArn!,
                ERROR_STREAM: errorStream.streamName,
```

- [ ] **Step 3: Verify CDK synth works**

Run: `cd /home/sla/projects/EMS/infra/daq/data_pipeline && npx cdk synth FlinkIcebergStack -c SHA=test -c RUN_NR=1 -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements --quiet`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add lib/flink_transforms-stack.ts
git commit -m "infra: add meter-identity DynamoDB table, error stream, and IAM permissions"
```

---

## Task 10: Run all tests and verify build

**Files:** None (validation only)

- [ ] **Step 1: Run all unit tests**

Run: `sbt test`
Expected: All tests PASS (existing processor tests + new enrichment tests)

- [ ] **Step 2: Build assembly JAR**

Run: `sbt clean assembly`
Expected: JAR built successfully at `target/scala-3.3.4/flink-app-scala-0.1.0.jar`

- [ ] **Step 3: Verify CDK deploy would work (diff only)**

Run: `cd /home/sla/projects/EMS/infra/daq/data_pipeline && npx cdk diff FlinkIcebergStack -c SHA=$(git rev-parse --short HEAD) -c RUN_NR=$(date +%s) -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements`
Expected: Shows new DynamoDB table, error stream, IAM changes, updated Flink app config

- [ ] **Step 4: Commit everything**

```bash
git add -A
git commit -m "feat: complete meter enrichment pipeline — ready for deploy"
```
