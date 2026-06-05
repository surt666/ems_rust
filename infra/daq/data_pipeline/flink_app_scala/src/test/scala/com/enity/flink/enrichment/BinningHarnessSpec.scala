package com.enity.flink.enrichment

import org.apache.flink.api.common.typeinfo.Types
import org.apache.flink.api.java.functions.KeySelector
import org.apache.flink.streaming.api.operators.KeyedProcessOperator
import org.apache.flink.streaming.util.KeyedOneInputStreamOperatorTestHarness
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers
import org.scalatest.BeforeAndAfterEach

import scala.jdk.CollectionConverters.*

class BinningHarnessSpec extends AnyFlatSpec with Matchers with BeforeAndAfterEach {

  private var harness: KeyedOneInputStreamOperatorTestHarness[
    java.lang.Integer, (EnrichedRecord, MeterMapping), EnrichedRecord
  ] = _

  private val FifteenMin = java.lang.Integer.valueOf(15)

  private def mapping(meterType: String, binning: java.lang.Integer = FifteenMin, logicalId: Int = 1): MeterMapping =
    MeterMapping(
      logicalId = logicalId,
      meterType = meterType,
      hn1 = 1, hn2 = 1,
      hn3 = null, hn4 = null, hn5 = null,
      hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      purpose = "test",
      binning = binning
    )

  private def makeRecord(value: Double, ts: String, logicalId: Int = 1): EnrichedRecord =
    EnrichedRecord(
      logicalId = logicalId,
      timestamp = ts,
      value = value,
      unit = "kWh",
      ingestedTime = "2026-01-01T10:00:01Z",
      hn1 = 1, hn2 = 1,
      hn3 = null, hn4 = null, hn5 = null,
      hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      purpose = "test"
    )

  private def tsMillis(ts: String): Long = java.time.Instant.parse(ts).toEpochMilli

  override def beforeEach(): Unit =
    val operator = new KeyedProcessOperator(new BinningFunction(6 * 3600 * 1000L))
    harness = new KeyedOneInputStreamOperatorTestHarness(
      operator,
      new KeySelector[(EnrichedRecord, MeterMapping), java.lang.Integer] {
        override def getKey(value: (EnrichedRecord, MeterMapping)): java.lang.Integer =
          java.lang.Integer.valueOf(value._1.logicalId)
      },
      Types.INT
    )
    harness.open()

  override def afterEach(): Unit =
    harness.close()

  "BinningFunction harness" should "buffer the first counter reading and emit nothing" in {
    val record = makeRecord(100.0, "2026-01-01T10:00:00Z")
    harness.processElement((record, mapping("counter")), tsMillis("2026-01-01T10:00:00Z"))
    harness.extractOutputValues().asScala shouldBe empty
  }

  it should "buffer the first gauge reading and emit nothing" in {
    val record = makeRecord(42.0, "2026-01-01T10:00:00Z")
    harness.processElement((record, mapping("gauge")), tsMillis("2026-01-01T10:00:00Z"))
    harness.extractOutputValues().asScala shouldBe empty
  }

  it should "emit one bin row when two consecutive counter readings span exactly one bin boundary" in {
    val m = mapping("counter")
    val r1 = makeRecord(100.0, "2026-01-01T10:00:00Z")
    val r2 = makeRecord(150.0, "2026-01-01T10:15:00Z")

    harness.processElement((r1, m), tsMillis("2026-01-01T10:00:00Z"))
    harness.processElement((r2, m), tsMillis("2026-01-01T10:15:00Z"))

    val output = harness.extractOutputValues().asScala
    output should have size 1
    output.head.value shouldBe 50.0
    output.head.binValue.doubleValue() shouldBe 50.0 +- 1e-9
    output.head.binMethod shouldBe "time_proportional"
    output.head.binTimestamp.longValue() shouldBe tsMillis("2026-01-01T10:15:00Z")
  }

  it should "fan out across multiple bins when there is a gap" in {
    val m = mapping("counter")
    val r1 = makeRecord(100.0, "2026-01-01T10:00:00Z")
    val r2 = makeRecord(160.0, "2026-01-01T11:00:00Z")

    harness.processElement((r1, m), tsMillis("2026-01-01T10:00:00Z"))
    harness.processElement((r2, m), tsMillis("2026-01-01T11:00:00Z"))

    val output = harness.extractOutputValues().asScala
    output should have size 4
    output.foreach(_.value shouldBe 60.0)
    output.map(_.binValue.doubleValue()).sum shouldBe 60.0 +- 1e-9
    output.foreach(_.binValue.doubleValue() shouldBe 15.0 +- 1e-9)
  }

  it should "emit anomaly side output for negative counter delta" in {
    val m = mapping("counter")
    val r1 = makeRecord(100.0, "2026-01-01T10:00:00Z")
    val r2 = makeRecord(50.0, "2026-01-01T10:15:00Z")

    harness.processElement((r1, m), tsMillis("2026-01-01T10:00:00Z"))
    harness.processElement((r2, m), tsMillis("2026-01-01T10:15:00Z"))

    harness.extractOutputValues().asScala shouldBe empty
    val anomalies = harness.getSideOutput(SideOutputTags.ANOMALY).asScala
    anomalies should have size 1
    anomalies.head.getValue.errorType shouldBe "anomaly"
    anomalies.head.getValue.error should include("Negative counter delta")
  }

  it should "linearly interpolate gauge between two readings" in {
    val m = mapping("gauge")
    val r1 = makeRecord(10.0, "2026-01-01T10:00:00Z")
    val r2 = makeRecord(30.0, "2026-01-01T10:30:00Z")

    harness.processElement((r1, m), tsMillis("2026-01-01T10:00:00Z"))
    harness.processElement((r2, m), tsMillis("2026-01-01T10:30:00Z"))

    val output = harness.extractOutputValues().asScala
    output should have size 2
    output.foreach(_.binMethod shouldBe "linear_interpolation")
    output.head.binValue.doubleValue() shouldBe 20.0 +- 1e-9
    output(1).binValue.doubleValue() shouldBe 30.0 +- 1e-9
    output.head.value shouldBe 30.0
  }

  it should "track state independently per meter (logicalId)" in {
    val m1 = mapping("counter", logicalId = 1)
    val m2 = mapping("counter", logicalId = 2)
    val m1r1 = makeRecord(100.0, "2026-01-01T10:00:00Z", 1)
    val m2r1 = makeRecord(500.0, "2026-01-01T10:00:00Z", 2)
    val m1r2 = makeRecord(120.0, "2026-01-01T10:15:00Z", 1)
    val m2r2 = makeRecord(530.0, "2026-01-01T10:15:00Z", 2)

    harness.processElement((m1r1, m1), tsMillis("2026-01-01T10:00:00Z"))
    harness.processElement((m2r1, m2), tsMillis("2026-01-01T10:00:00Z"))
    harness.processElement((m1r2, m1), tsMillis("2026-01-01T10:15:00Z"))
    harness.processElement((m2r2, m2), tsMillis("2026-01-01T10:15:00Z"))

    val output = harness.extractOutputValues().asScala
    output should have size 2
    output.find(_.logicalId == 1).get.value shouldBe 20.0
    output.find(_.logicalId == 2).get.value shouldBe 30.0
  }

  it should "emit late arrival side output when predecessor is purged" in {
    harness.close()

    val shortRetention = 1000L
    val operator = new KeyedProcessOperator(new BinningFunction(shortRetention))
    harness = new KeyedOneInputStreamOperatorTestHarness(
      operator,
      new KeySelector[(EnrichedRecord, MeterMapping), java.lang.Integer] {
        override def getKey(value: (EnrichedRecord, MeterMapping)): java.lang.Integer =
          java.lang.Integer.valueOf(value._1.logicalId)
      },
      Types.INT
    )
    harness.open()

    val m = mapping("counter")
    val r1 = makeRecord(100.0, "2026-01-01T10:00:00Z")
    val r2 = makeRecord(120.0, "2026-01-01T10:00:02Z")
    harness.processElement((r1, m), tsMillis("2026-01-01T10:00:00Z"))
    harness.processElement((r2, m), tsMillis("2026-01-01T10:00:02Z"))

    harness.processWatermark(tsMillis("2026-01-01T10:01:00Z"))

    val late = makeRecord(200.0, "2026-01-01T10:00:01Z")
    harness.processElement((late, m), tsMillis("2026-01-01T10:00:01Z"))

    val lateArrivals = harness.getSideOutput(SideOutputTags.LATE_ARRIVAL).asScala
    lateArrivals should have size 1
    lateArrivals.head.getValue.errorType shouldBe "late_arrival"
    lateArrivals.head.getValue.error should include("No predecessor in buffer")
  }

  it should "emit raw row with null bin fields when binning is null" in {
    val m = mapping("counter", binning = null)
    val r1 = makeRecord(100.0, "2026-01-01T10:00:00Z")
    val r2 = makeRecord(150.0, "2026-01-01T10:15:00Z")

    harness.processElement((r1, m), tsMillis("2026-01-01T10:00:00Z"))
    harness.processElement((r2, m), tsMillis("2026-01-01T10:15:00Z"))

    val output = harness.extractOutputValues().asScala
    output should have size 1
    output.head.binTimestamp shouldBe null
    output.head.binValue shouldBe null
    output.head.binMethod shouldBe null
  }
}
