package com.enity.flink.enrichment

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

import java.time.Instant

class ResampleFunctionSpec extends AnyFlatSpec with Matchers {

  private val FifteenMin = java.lang.Integer.valueOf(15)
  private val OneHour = java.lang.Integer.valueOf(60)

  private def mapping(meterType: String, resampleMinutes: java.lang.Integer = FifteenMin): MeterMapping =
    MeterMapping(
      logicalId = 101,
      meterType = meterType,
      hn1 = 1, hn2 = 2,
      hn3 = null, hn4 = java.lang.Integer.valueOf(8), hn5 = null,
      hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      purpose = "test",
      resampleMinutes = resampleMinutes
    )

  private def enriched(value: Double, ts: String): EnrichedRecord =
    EnrichedRecord(
      logicalId = 101,
      timestamp = ts,
      value = value,
      unit = "kWh",
      ingestedTime = "2026-03-27T10:00:01Z",
      hn1 = 1, hn2 = 2,
      hn3 = null, hn4 = java.lang.Integer.valueOf(8), hn5 = null,
      hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      purpose = "test"
    )

  private def buffered(value: Double, ts: String, m: MeterMapping): BufferedReadingV2 =
    BufferedReadingV2(value, enriched(value, ts), m)

  private def epochMs(ts: String): Long = Instant.parse(ts).toEpochMilli

  // ── enumerateBinsIn ──

  "enumerateBinsIn" should "return empty when binSize is zero" in {
    ResampleFunction.enumerateBinsIn(0L, 1000L, 0L) shouldBe empty
  }

  it should "return empty when currentTs <= prevTs" in {
    ResampleFunction.enumerateBinsIn(1000L, 1000L, 100L) shouldBe empty
    ResampleFunction.enumerateBinsIn(1000L, 500L, 100L) shouldBe empty
  }

  it should "be left-exclusive: prevTs exactly on boundary is not emitted" in {
    val binMs = 15L * 60L * 1000L
    val prev = epochMs("2026-05-01T10:00:00Z")
    val curr = epochMs("2026-05-01T10:14:00Z")
    ResampleFunction.enumerateBinsIn(prev, curr, binMs) shouldBe empty
  }

  it should "be right-inclusive: currentTs exactly on boundary is emitted" in {
    val binMs = 15L * 60L * 1000L
    val prev = epochMs("2026-05-01T10:00:00Z")
    val curr = epochMs("2026-05-01T10:15:00Z")
    ResampleFunction.enumerateBinsIn(prev, curr, binMs) shouldBe Seq(epochMs("2026-05-01T10:15:00Z"))
  }

  it should "enumerate multiple bins across a gap" in {
    val binMs = 15L * 60L * 1000L
    val prev = epochMs("2026-05-01T10:00:00Z")
    val curr = epochMs("2026-05-01T10:45:00Z")
    ResampleFunction.enumerateBinsIn(prev, curr, binMs) shouldBe Seq(
      epochMs("2026-05-01T10:15:00Z"),
      epochMs("2026-05-01T10:30:00Z"),
      epochMs("2026-05-01T10:45:00Z")
    )
  }

  it should "enumerate one bin in a normal-spaced period" in {
    val binMs = 15L * 60L * 1000L
    val prev = epochMs("2026-05-01T10:08:00Z")
    val curr = epochMs("2026-05-01T10:23:00Z")
    ResampleFunction.enumerateBinsIn(prev, curr, binMs) shouldBe Seq(epochMs("2026-05-01T10:15:00Z"))
  }

  // ── Counter time-proportional split ──

  "computeBins (counter)" should "flag negative delta as Anomaly" in {
    val m = mapping("counter")
    val prev = buffered(150.0, "2026-05-01T10:00:00Z", m)
    val curr = buffered(140.0, "2026-05-01T10:15:00Z", m)
    ResampleFunction.computeBins(prev, curr, epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m) shouldBe
      ResampleFunction.Anomaly
  }

  it should "emit single bin in normal-spaced period" in {
    val m = mapping("counter")
    val prev = buffered(100.0, "2026-05-01T10:00:00Z", m)
    val curr = buffered(115.0, "2026-05-01T10:15:00Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) =>
        rows.size shouldBe 1
        rows.head.value shouldBe 15.0
        rows.head.binValue.doubleValue() shouldBe 15.0 +- 1e-9
        rows.head.binMethod shouldBe "time_proportional"
        rows.head.binTimestamp.longValue() shouldBe epochMs("2026-05-01T10:15:00Z")
      case _ => fail("expected Bins")
  }

  it should "split delta time-proportionally across multiple bins (energy conservation)" in {
    val m = mapping("counter")
    val prev = buffered(100.0, "2026-05-01T10:00:00Z", m)
    val curr = buffered(120.0, "2026-05-01T10:30:00Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) =>
        rows.size shouldBe 2
        rows.foreach(_.value shouldBe 20.0)
        rows.foreach(_.binMethod shouldBe "time_proportional")
        rows.map(_.binValue.doubleValue()).sum shouldBe 20.0 +- 1e-9
        rows.head.binValue.doubleValue() shouldBe 10.0 +- 1e-9
        rows(1).binValue.doubleValue() shouldBe 10.0 +- 1e-9
      case _ => fail("expected Bins")
  }

  it should "handle gap with partial first bin overlap" in {
    val m = mapping("counter")
    val prev = buffered(100.0, "2026-05-01T10:08:00Z", m)
    val curr = buffered(122.0, "2026-05-01T10:30:00Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) =>
        rows.size shouldBe 2
        rows.head.binValue.doubleValue() shouldBe 7.0 +- 1e-9
        rows(1).binValue.doubleValue() shouldBe 15.0 +- 1e-9
        rows.map(_.binValue.doubleValue()).sum shouldBe 22.0 +- 1e-9
      case _ => fail("expected Bins")
  }

  it should "split delta across both bins when period straddles a boundary mid-period" in {
    // Period 09:54:29 → 10:09:33 (15min04sec) crosses bin boundary 10:00 mid-period.
    // Both bin windows overlap: [09:45, 10:00] gets [09:54:29, 10:00] = 5min31sec;
    // [10:00, 10:15] gets [10:00, 10:09:33] = 9min33sec. Energy conserved across both.
    val m = mapping("counter")
    val prev = buffered(0.0, "2026-05-01T09:54:29Z", m)
    val curr = buffered(100.0, "2026-05-01T10:09:33Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) =>
        rows.size shouldBe 2
        val byBin = rows.map(r => r.binTimestamp.longValue() -> r.binValue.doubleValue()).toMap
        byBin(epochMs("2026-05-01T10:00:00Z")) shouldBe (100.0 * 331.0 / 904.0) +- 1e-9
        byBin(epochMs("2026-05-01T10:15:00Z")) shouldBe (100.0 * 573.0 / 904.0) +- 1e-9
        rows.map(_.binValue.doubleValue()).sum shouldBe 100.0 +- 1e-9
        rows.foreach(_.binMethod shouldBe "time_proportional")
      case _ => fail("expected Bins")
  }

  // ── Gauge linear interpolation ──

  "computeBins (gauge)" should "linearly interpolate at the bin boundary" in {
    val m = mapping("gauge")
    val prev = buffered(10.0, "2026-05-01T10:00:00Z", m)
    val curr = buffered(20.0, "2026-05-01T10:30:00Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) =>
        rows.size shouldBe 2
        rows.head.binValue.doubleValue() shouldBe 15.0 +- 1e-9
        rows(1).binValue.doubleValue() shouldBe 20.0 +- 1e-9
        rows.foreach(_.binMethod shouldBe "linear_interpolation")
      case _ => fail("expected Bins")
  }

  it should "preserve original value field for gauges" in {
    val m = mapping("gauge")
    val prev = buffered(10.0, "2026-05-01T10:00:00Z", m)
    val curr = buffered(20.0, "2026-05-01T10:15:00Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) => rows.head.value shouldBe 20.0
      case _ => fail("expected Bins")
  }

  it should "interpolate gap bins between widely spaced gauge readings" in {
    val m = mapping("gauge")
    val prev = buffered(0.0, "2026-05-01T10:00:00Z", m)
    val curr = buffered(60.0, "2026-05-01T11:00:00Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) =>
        rows.size shouldBe 4
        rows.map(_.binValue.doubleValue()) shouldBe Seq(15.0, 30.0, 45.0, 60.0)
      case _ => fail("expected Bins")
  }

  // ── Edge cases ──

  "computeBins" should "produce no rows when period straddles no bin boundary" in {
    val m = mapping("gauge")
    val prev = buffered(10.0, "2026-05-01T10:01:00Z", m)
    val curr = buffered(20.0, "2026-05-01T10:14:00Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) => rows shouldBe empty
      case _ => fail("expected Bins")
  }

  it should "support hourly bins" in {
    val m = mapping("counter", OneHour)
    val prev = buffered(100.0, "2026-05-01T10:00:00Z", m)
    val curr = buffered(124.0, "2026-05-01T11:00:00Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) =>
        rows.size shouldBe 1
        rows.head.binValue.doubleValue() shouldBe 24.0 +- 1e-9
      case _ => fail("expected Bins")
  }

  it should "pass through unknown meter types unchanged" in {
    val m = mapping("unknown_type")
    val prev = buffered(10.0, "2026-05-01T10:00:00Z", m)
    val curr = buffered(20.0, "2026-05-01T10:15:00Z", m)
    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(prev.record.timestamp), epochMs(curr.record.timestamp), m)
    result match
      case ResampleFunction.Bins(rows) =>
        rows.size shouldBe 1
        rows.head.value shouldBe 20.0
        rows.head.binTimestamp shouldBe null
        rows.head.binValue shouldBe null
        rows.head.binMethod shouldBe null
      case _ => fail("expected Bins")
  }

  // ── BufferedReadingV2 ──

  "BufferedReadingV2" should "store cumulative value, record, and mapping" in {
    val m = mapping("counter")
    val r = enriched(1234.5, "2026-05-01T10:00:00Z")
    val b = BufferedReadingV2(1234.5, r, m)
    b.cumulativeValue shouldBe 1234.5
    b.record.logicalId shouldBe 101
    b.mapping.meterType shouldBe "counter"
  }
}
