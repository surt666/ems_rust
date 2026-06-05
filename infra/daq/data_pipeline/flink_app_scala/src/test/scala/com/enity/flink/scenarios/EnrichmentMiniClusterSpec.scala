package com.enity.flink.scenarios

import com.enity.flink.SensorRecord
import com.enity.flink.enrichment.MeterMapping
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class EnrichmentMiniClusterSpec extends AnyFlatSpec with Matchers with MiniClusterTest {

  private def makeSensor(
      daqId: String, value: Double, ts: String,
      unit: String = "kWh", sensorType: String = "test"
  ): SensorRecord =
    SensorRecord(
      daqId = daqId, `type` = sensorType, gatewayId = "gw1", meterId = "m1",
      timestamp = ts, ingestedTime = "2026-01-01T00:00:01Z",
      sensorId = "energy", value = value.toString, unit = unit
    )

  private def counterMapping(daqId: String, logicalId: Int): (String, MeterMapping) =
    daqId -> MeterMapping(
      logicalId = logicalId, meterType = "counter",
      hn1 = 1, hn2 = 1,
      hn3 = java.lang.Integer.valueOf(10),
      hn4 = java.lang.Integer.valueOf(20),
      hn5 = null, hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      purpose = "energy"
    )

  private def gaugeMapping(daqId: String, logicalId: Int): (String, MeterMapping) =
    daqId -> MeterMapping(
      logicalId = logicalId, meterType = "gauge",
      hn1 = 1, hn2 = 1,
      hn3 = null, hn4 = null, hn5 = null, hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      purpose = "test"
    )

  "Counter out-of-order" should "produce corrected deltas after watermark" in {
    val daqId = "daq:test:ooo"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(
        makeSensor(daqId, 100.0, "2026-01-01T10:00:00Z"),
        makeSensor(daqId, 130.0, "2026-01-01T10:30:00Z"),
        makeSensor(daqId, 115.0, "2026-01-01T10:15:00Z")
      ),
      mappings = Map(counterMapping(daqId, 1001)),
      config = ScenarioConfig(maxOutOfOrdernessMs = 5000L, bufferRetentionMs = 60000L)
    )

    result.enrichedRecords should not be empty
    result.sideOutputs("ANOMALY") shouldBe empty
    result.enrichedRecords.foreach(r => r.value should be >= 0.0)
    val deltaValues = result.enrichedRecords.map(_.value).toSet
    deltaValues should contain(30.0)
    deltaValues should contain(15.0)
  }

  "Mapping update" should "use updated mapping for later records" in {
    val daqId = "daq:test:cdc"
    val mapping1 = MeterMapping(
      logicalId = 1, meterType = "gauge",
      hn1 = 1, hn2 = 1,
      hn3 = java.lang.Integer.valueOf(10),
      hn4 = null, hn5 = null, hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      purpose = "test"
    )

    val result1 = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(makeSensor(daqId, 42.0, "2026-01-01T10:00:00Z")),
      mappings = Map(daqId -> mapping1)
    )

    result1.enrichedRecords should have size 1
    result1.enrichedRecords.head.logicalId shouldBe 1
    result1.enrichedRecords.head.hn3 shouldBe java.lang.Integer.valueOf(10)

    CollectSinks.clear()

    val mapping2 = mapping1.copy(
      logicalId = 2,
      hn3 = java.lang.Integer.valueOf(99)
    )

    val result2 = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(makeSensor(daqId, 43.0, "2026-01-01T11:00:00Z")),
      mappings = Map(daqId -> mapping2)
    )

    result2.enrichedRecords should have size 1
    result2.enrichedRecords.head.logicalId shouldBe 2
    result2.enrichedRecords.head.hn3 shouldBe java.lang.Integer.valueOf(99)
  }

  "Multiple meters" should "route records to correct logical meters" in {
    val daqEnergy = "daq:test:multi:energy"
    val daqTemp = "daq:test:multi:temperature"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(
        makeSensor(daqEnergy, 100.0, "2026-01-01T10:00:00Z", unit = "kWh"),
        makeSensor(daqTemp, 21.5, "2026-01-01T10:00:00Z", unit = "C"),
        makeSensor(daqEnergy, 150.0, "2026-01-01T10:15:00Z", unit = "kWh"),
        makeSensor(daqTemp, 22.0, "2026-01-01T10:15:00Z", unit = "C")
      ),
      mappings = Map(
        counterMapping(daqEnergy, 100),
        gaugeMapping(daqTemp, 200)
      )
    )

    val tempRecords = result.enrichedRecords.filter(_.logicalId == 200)
    val energyRecords = result.enrichedRecords.filter(_.logicalId == 100)

    tempRecords should have size 2
    tempRecords.map(_.value).toSet shouldBe Set(21.5, 22.0)

    energyRecords should have size 1
    energyRecords.head.value shouldBe 50.0
  }

  "Parse error routing" should "route malformed JSON to PARSE_ERROR side output" in {
    val result = ScenarioTestHelper.buildAndRunFromJson(
      jsonStrings = List(
        """{"schematype":"unknown_type_xyz","data":"garbage"}""",
        """not valid json at all {{{"""
      ),
      mappings = Map.empty
    )

    result.enrichedRecords shouldBe empty
    result.sideOutputs("PARSE_ERROR") should have size 2
    result.sideOutputs("PARSE_ERROR").foreach { err =>
      err.errorType shouldBe "parse_error"
    }
  }

  "Mixed gauge + counter" should "pass gauges through and compute counter deltas" in {
    val gaugeDaq = "daq:test:mixed:gauge"
    val counterDaq = "daq:test:mixed:counter"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(
        makeSensor(gaugeDaq, 50.0, "2026-01-01T10:00:00Z"),
        makeSensor(counterDaq, 1000.0, "2026-01-01T10:00:00Z"),
        makeSensor(gaugeDaq, 55.0, "2026-01-01T10:15:00Z"),
        makeSensor(counterDaq, 1025.0, "2026-01-01T10:15:00Z"),
        makeSensor(gaugeDaq, 53.0, "2026-01-01T10:30:00Z"),
        makeSensor(counterDaq, 1060.0, "2026-01-01T10:30:00Z")
      ),
      mappings = Map(
        gaugeMapping(gaugeDaq, 300),
        counterMapping(counterDaq, 400)
      )
    )

    val gauges = result.enrichedRecords.filter(_.logicalId == 300)
    val counters = result.enrichedRecords.filter(_.logicalId == 400)

    gauges should have size 3
    gauges.map(_.value).toSet shouldBe Set(50.0, 55.0, 53.0)

    counters should have size 2
    counters.map(_.value).toSet shouldBe Set(25.0, 35.0)

    result.sideOutputs.values.flatten shouldBe empty
  }
}
