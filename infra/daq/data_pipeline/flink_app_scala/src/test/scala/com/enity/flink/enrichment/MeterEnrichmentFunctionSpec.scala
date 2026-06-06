package com.enity.flink.enrichment

import com.enity.flink.SensorRecord
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class MeterEnrichmentFunctionSpec extends AnyFlatSpec with Matchers {

  private val testMapping = MeterMapping(
    logicalId = 101,
    meterType = "gauge",
    hn1 = 1, hn2 = 2,
    hn3 = null, hn4 = java.lang.Integer.valueOf(8), hn5 = java.lang.Integer.valueOf(3),
    hn6 = null, hn7 = null, hn8 = null, hn9 = null,
    purpose = "supply temp"
  )

  private val testRecord = SensorRecord(
    daqId = "daq:std_json_v1:cust:meter1:temp",
    `type` = "std_json_v1", gatewayId = "gw1", meterId = "meter1",
    timestamp = "2026-03-27T10:00:00Z", ingestedTime = "2026-03-27T10:00:01Z",
    sensorId = "temp", value = "21.5", unit = "C"
  )

  "MeterEnrichmentFunction.enrich" should "produce EnrichedRecord" in {
    val enriched = MeterEnrichmentFunction.enrich(testRecord, testMapping)
    enriched.logicalId shouldBe 101
    enriched.value shouldBe 21.5
    enriched.unit shouldBe "C"
    enriched.hn1 shouldBe 1
    enriched.hn2 shouldBe 2
    enriched.hn4 shouldBe java.lang.Integer.valueOf(8)
    enriched.hn5 shouldBe java.lang.Integer.valueOf(3)
    enriched.hn3 shouldBe null
    enriched.purpose shouldBe "supply temp"
  }

  it should "correctly parse string value to double" in {
    val record = testRecord.copy(value = "123.456")
    val enriched = MeterEnrichmentFunction.enrich(record, testMapping)
    enriched.value shouldBe 123.456
  }

  it should "not normalize units (normalization happens after delta computation)" in {
    val record = testRecord.copy(value = "5.0", unit = "kWh")
    val enriched = MeterEnrichmentFunction.enrich(record, testMapping)
    enriched.unit shouldBe "kWh"
    enriched.value shouldBe 5.0
  }

  it should "preserve all hierarchy levels from mapping" in {
    val fullMapping = testMapping.copy(
      hn3 = java.lang.Integer.valueOf(4),
      hn6 = java.lang.Integer.valueOf(7),
      hn9 = java.lang.Integer.valueOf(99)
    )
    val e = MeterEnrichmentFunction.enrich(testRecord, fullMapping)
    e.hn3 shouldBe java.lang.Integer.valueOf(4)
    e.hn6 shouldBe java.lang.Integer.valueOf(7)
    e.hn9 shouldBe java.lang.Integer.valueOf(99)
    e.hn4 shouldBe java.lang.Integer.valueOf(8)
  }

  it should "preserve raw timestamp regardless of resampleMinutes value" in {
    val mappings = Seq(
      testMapping.copy(resampleMinutes = null),
      testMapping.copy(resampleMinutes = java.lang.Integer.valueOf(15)),
      testMapping.copy(resampleMinutes = java.lang.Integer.valueOf(60))
    )
    for (m <- mappings) do
      val record = testRecord.copy(timestamp = "2026-03-27T10:07:23Z")
      val enriched = MeterEnrichmentFunction.enrich(record, m)
      enriched.timestamp shouldBe "2026-03-27T10:07:23Z"
  }

  it should "leave bin_* fields null at the enrichment stage" in {
    val mapping = testMapping.copy(resampleMinutes = java.lang.Integer.valueOf(15))
    val enriched = MeterEnrichmentFunction.enrich(testRecord, mapping)
    enriched.binTimestamp shouldBe null
    enriched.binValue shouldBe null
    enriched.binMethod shouldBe null
  }
}
