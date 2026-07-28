package com.enity.flink.enrichment

import com.enity.flink.SensorRecord
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

/** Integration test: exercises enrich → ResampleFunction.computeBins pipeline end-to-end
  * using the pure functions, verifying delta correctness, time-proportional split,
  * and gauge linear interpolation. Includes out-of-order scenarios. */
class EnrichmentPipelineSpec extends AnyFlatSpec with Matchers {

  private val FifteenMin = java.lang.Integer.valueOf(15)

  private val counterMapping = SensorMapping(
    logicalId = 1001,
    readingKind = "counter",
    hn1 = 1, hn2 = 1,
    hn3 = java.lang.Integer.valueOf(1), hn4 = java.lang.Integer.valueOf(1), hn5 = null,
    hn6 = null, hn7 = null, hn8 = null, hn9 = null,
    energyType = "volume",
    resampleMinutes = FifteenMin
  )

  private val gaugeMapping = counterMapping.copy(
    logicalId = 2002,
    readingKind = "gauge"
  )

  private def makeSensorRecord(value: Double, ts: String): SensorRecord =
    SensorRecord(
      daqId = "daq:flowiq2200_v1:123:0013ea010337c072:volume",
      `type` = "flowiq2200_v1", gatewayId = "gw1", meterId = "0013ea010337c072",
      timestamp = ts, ingestedTime = "2026-03-28T00:00:01Z",
      sensorId = "volume", value = value.toString, unit = "m3"
    )

  private def epochMs(ts: String): Long = java.time.Instant.parse(ts).toEpochMilli

  /** Compute (timestamp, delta, isAnomaly) for a sequence of counter readings using
    * the new ResampleFunction.computeBins pure function. Sorts buffer by event time first,
    * matching the ResampleFunction operator's behavior on out-of-order arrival. */
  private def computeDeltasWithBuffer(readings: Seq[(Double, String)]): Seq[(String, Double, Boolean)] =
    val buffered = readings.map { (value, ts) =>
      val sensor = makeSensorRecord(value, ts)
      val record = MeterEnrichmentFunction.enrich(sensor, counterMapping)
      (epochMs(ts), value, BufferedReadingV2(value, record, counterMapping))
    }

    buffered.sortBy(_._1).sliding(2).collect {
      case Seq((prevTs, _, prev), (curTs, _, cur)) =>
        ResampleFunction.computeBins(prev, cur, prevTs, curTs, counterMapping) match
          case ResampleFunction.Anomaly =>
            (cur.record.timestamp, cur.cumulativeValue - prev.cumulativeValue, true)
          case ResampleFunction.Bins(rows) =>
            // delta is in `value` field (same across all rows for a given (prev, current))
            (cur.record.timestamp, rows.headOption.map(_.value).getOrElse(0.0), false)
    }.toSeq

  "Enrichment pipeline" should "produce correct deltas for a counter meter sequence" in {
    val rawReadings = Seq(
      (1234.567, "2026-03-27T00:00:00Z"),
      (1235.323, "2026-03-27T02:00:00Z"),
      (1236.079, "2026-03-27T07:00:00Z"),
      (1239.479, "2026-03-27T09:00:00Z"),
      (1244.761, "2026-03-27T14:00:00Z")
    )

    val deltas = computeDeltasWithBuffer(rawReadings)

    deltas.size shouldBe 4
    deltas.foreach(_._3 shouldBe false)
    val rawValues = rawReadings.map(_._1)
    for (i <- deltas.indices) do
      deltas(i)._2 shouldBe (rawValues(i + 1) - rawValues(i)) +- 1e-10
  }

  it should "produce correct deltas matching real flowiq data pattern" in {
    val cumulativeVolumes = Seq(5281.123, 5286.412, 5293.206, 5301.887, 5310.566)
    val timestamps = Seq("00:00:00", "02:00:00", "07:00:00", "09:00:00", "14:00:00")
      .map(t => s"2026-03-27T${t}Z")

    val deltas = computeDeltasWithBuffer(cumulativeVolumes.zip(timestamps))

    deltas.size shouldBe 4
    deltas.map(_._2).sum shouldBe (5310.566 - 5281.123) +- 1e-10
  }

  it should "interpolate gauge values linearly" in {
    val r1 = MeterEnrichmentFunction.enrich(makeSensorRecord(10.0, "2026-03-27T10:00:00Z"), gaugeMapping)
    val r2 = MeterEnrichmentFunction.enrich(makeSensorRecord(30.0, "2026-03-27T10:30:00Z"), gaugeMapping)
    val prev = BufferedReadingV2(10.0, r1, gaugeMapping)
    val curr = BufferedReadingV2(30.0, r2, gaugeMapping)

    val result = ResampleFunction.computeBins(prev, curr,
      epochMs(r1.timestamp), epochMs(r2.timestamp), gaugeMapping)
    result match
      case ResampleFunction.Bins(rows) =>
        rows.size shouldBe 2
        rows.head.resampleValue.doubleValue() shouldBe 20.0 +- 1e-9   // 10:15, midpoint
        rows(1).resampleValue.doubleValue() shouldBe 30.0 +- 1e-9    // 10:30, equals current
        rows.head.logicalId shouldBe 2002
      case _ => fail("expected Bins")
  }

  it should "flag negative delta as anomaly in counter sequence" in {
    val readings = Seq(
      (1000.0, "2026-03-27T10:00:00Z"),
      (1050.0, "2026-03-27T11:00:00Z"),
      (500.0, "2026-03-27T12:00:00Z")
    )

    val deltas = computeDeltasWithBuffer(readings)
    deltas.size shouldBe 2
    deltas(0)._3 shouldBe false
    deltas(1)._3 shouldBe true
  }

  it should "preserve hierarchy context through the pipeline" in {
    val mapping = SensorMapping(
      logicalId = 9001,
      readingKind = "gauge",
      hn1 = 10, hn2 = 20,
      hn3 = java.lang.Integer.valueOf(30),
      hn4 = java.lang.Integer.valueOf(40),
      hn5 = java.lang.Integer.valueOf(50),
      hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      energyType = "test",
      resampleMinutes = FifteenMin
    )
    val enriched = MeterEnrichmentFunction.enrich(
      makeSensorRecord(99.9, "2026-03-27T10:00:00Z"), mapping
    )

    enriched.hn1 shouldBe 10
    enriched.hn2 shouldBe 20
    enriched.hn3 shouldBe java.lang.Integer.valueOf(30)
    enriched.hn4 shouldBe java.lang.Integer.valueOf(40)
    enriched.hn5 shouldBe java.lang.Integer.valueOf(50)
    enriched.hn6 shouldBe null
  }

  // ── Out-of-order message handling tests ──

  "Out-of-order enrichment pipeline" should "produce correct deltas when records arrive out of order" in {
    val readings = Seq(
      (1234.567, "2026-03-27T00:00:00Z"),
      (1236.079, "2026-03-27T07:00:00Z"),
      (1235.323, "2026-03-27T02:00:00Z"),
      (1244.761, "2026-03-27T14:00:00Z"),
      (1239.479, "2026-03-27T09:00:00Z")
    )

    val deltas = computeDeltasWithBuffer(readings)

    deltas.size shouldBe 4
    deltas.foreach(_._3 shouldBe false)
    deltas(0)._1 shouldBe "2026-03-27T02:00:00Z"
    deltas(0)._2 shouldBe (1235.323 - 1234.567) +- 1e-10
    deltas.map(_._2).sum shouldBe (1244.761 - 1234.567) +- 1e-10
  }

  it should "produce same deltas regardless of arrival order" in {
    val rawReadings = Seq(
      (100.0, "2026-03-27T00:00:00Z"),
      (115.0, "2026-03-27T02:00:00Z"),
      (130.0, "2026-03-27T07:00:00Z"),
      (145.0, "2026-03-27T09:00:00Z")
    )

    val inOrder = computeDeltasWithBuffer(rawReadings)
    val reversed = computeDeltasWithBuffer(rawReadings.reverse)
    val shuffled = computeDeltasWithBuffer(Seq(
      rawReadings(2), rawReadings(0), rawReadings(3), rawReadings(1)
    ))

    inOrder.map(_._2) shouldBe reversed.map(_._2)
    inOrder.map(_._2) shouldBe shuffled.map(_._2)
    inOrder.map(_._1) shouldBe reversed.map(_._1)
  }

  it should "detect anomaly correctly even with out-of-order arrival" in {
    val readings = Seq(
      (1239.479, "2026-03-27T09:00:00Z"),
      (500.0, "2026-03-27T10:00:00Z"),
      (1234.567, "2026-03-27T00:00:00Z"),
    )

    val deltas = computeDeltasWithBuffer(readings)

    deltas.size shouldBe 2
    deltas(0)._2 shouldBe (1239.479 - 1234.567) +- 1e-10
    deltas(0)._3 shouldBe false
    deltas(1)._3 shouldBe true
  }
}
