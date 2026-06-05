package com.enity.flink.scenarios

import com.enity.flink.SensorRecord
import com.enity.flink.enrichment.MeterMapping
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class ScenarioTestHelperSpec extends AnyFlatSpec with Matchers with MiniClusterTest {

  private def makeSensor(daqId: String, value: Double, ts: String, unit: String = "kWh"): SensorRecord =
    SensorRecord(
      daqId = daqId, `type` = "test", gatewayId = "gw1", meterId = "m1",
      timestamp = ts, ingestedTime = "2026-01-01T00:00:01Z",
      sensorId = "s1", value = value.toString, unit = unit
    )

  private val gaugeMapping = MeterMapping(
    logicalId = 7, meterType = "gauge",
    hn1 = 1, hn2 = 1,
    hn3 = null, hn4 = null, hn5 = null, hn6 = null, hn7 = null, hn8 = null, hn9 = null,
    purpose = "test"
  )

  "ScenarioTestHelper" should "run a gauge record through the pipeline" in {
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(makeSensor("daq1", 42.0, "2026-01-01T10:00:00Z")),
      mappings = Map("daq1" -> gaugeMapping)
    )

    result.enrichedRecords should have size 1
    result.enrichedRecords.head.logicalId shouldBe 7
    result.enrichedRecords.head.value shouldBe 42.0
    result.sideOutputs.values.flatten shouldBe empty
  }
}
