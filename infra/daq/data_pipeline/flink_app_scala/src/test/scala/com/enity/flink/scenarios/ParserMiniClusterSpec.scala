package com.enity.flink.scenarios

import com.enity.flink.SensorRecord
import com.enity.flink.enrichment.MeterMapping
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class ParserMiniClusterSpec extends AnyFlatSpec with Matchers with MiniClusterTest {

  private def makeSensor(
      daqId: String, value: Double, ts: String,
      unit: String = "kWh", sensorType: String = "test",
      sensorId: String = "energy"
  ): SensorRecord =
    SensorRecord(
      daqId = daqId, `type` = sensorType, gatewayId = "gw1", meterId = "m1",
      timestamp = ts, ingestedTime = "2026-01-01T00:00:01Z",
      sensorId = sensorId, value = value.toString, unit = unit
    )

  private def gaugeMapping(daqId: String, logicalId: Int, hn1: Int = 1, hn2: Int = 1): (String, MeterMapping) =
    daqId -> MeterMapping(
      logicalId = logicalId, meterType = "gauge",
      hn1 = hn1, hn2 = hn2,
      hn3 = java.lang.Integer.valueOf(10),
      hn4 = java.lang.Integer.valueOf(20),
      hn5 = null, hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      purpose = "test"
    )

  private def counterMapping(daqId: String, logicalId: Int): (String, MeterMapping) =
    daqId -> MeterMapping(
      logicalId = logicalId, meterType = "counter",
      hn1 = 1, hn2 = 1,
      hn3 = java.lang.Integer.valueOf(10),
      hn4 = java.lang.Integer.valueOf(20),
      hn5 = null, hn6 = null, hn7 = null, hn8 = null, hn9 = null,
      purpose = "test"
    )

  "EMU gauge" should "flow through pipeline with correct enrichment" in {
    val daqId = "daq:emu_v1:customer1:deveui1:energy"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(makeSensor(daqId, 42.5, "2026-01-01T10:00:00Z", unit = "kWh", sensorType = "emu_profes_v1")),
      mappings = Map(gaugeMapping(daqId, 1))
    )

    result.enrichedRecords should have size 1
    val r = result.enrichedRecords.head
    r.logicalId shouldBe 1
    r.value shouldBe 42.5
    r.unit shouldBe "kWh"
    r.hn1 shouldBe 1
    r.hn2 shouldBe 1
    r.hn3 shouldBe java.lang.Integer.valueOf(10)
    r.hn4 shouldBe java.lang.Integer.valueOf(20)
    result.sideOutputs.values.flatten shouldBe empty
  }

  "EMU counter" should "compute delta between two readings" in {
    val daqId = "daq:emu_v1:customer1:deveui1:energy"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(
        makeSensor(daqId, 1000.0, "2026-01-01T10:00:00Z", sensorType = "emu_profes_v1"),
        makeSensor(daqId, 1050.0, "2026-01-01T10:15:00Z", sensorType = "emu_profes_v1")
      ),
      mappings = Map(counterMapping(daqId, 2))
    )

    result.enrichedRecords should have size 1
    result.enrichedRecords.head.value shouldBe 50.0
    result.enrichedRecords.head.logicalId shouldBe 2
    result.sideOutputs.values.flatten shouldBe empty
  }

  "FLOWIQ water meter" should "handle multiple sensor records per device" in {
    val volumeId = "daq:flowiq2200_v1:cust1:serial1:volume"
    val tempId = "daq:flowiq2200_v1:cust1:serial1:temperature"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(
        makeSensor(volumeId, 5281.123, "2026-01-01T10:00:00Z", unit = "m3", sensorType = "flowiq2200_v1", sensorId = "volume"),
        makeSensor(tempId, 18.5, "2026-01-01T10:00:00Z", unit = "C", sensorType = "flowiq2200_v1", sensorId = "temperature")
      ),
      mappings = Map(
        counterMapping(volumeId, 3),
        gaugeMapping(tempId, 4)
      )
    )

    val gaugeRecords = result.enrichedRecords.filter(_.logicalId == 4)
    gaugeRecords should have size 1
    gaugeRecords.head.value shouldBe 18.5
    gaugeRecords.head.unit shouldBe "C"
  }

  "Bluemetering" should "handle multi-register payload" in {
    val reg1 = "daq:bluemetering_v1:cust1:device1:register1"
    val reg2 = "daq:bluemetering_v1:cust1:device1:register2"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(
        makeSensor(reg1, 100.0, "2026-01-01T10:00:00Z", unit = "kWh", sensorType = "bluemetering_json_v1", sensorId = "register1"),
        makeSensor(reg2, 200.0, "2026-01-01T10:00:00Z", unit = "kWh", sensorType = "bluemetering_json_v1", sensorId = "register2")
      ),
      mappings = Map(gaugeMapping(reg1, 5), gaugeMapping(reg2, 6))
    )

    result.enrichedRecords should have size 2
    result.enrichedRecords.map(_.logicalId).toSet shouldBe Set(5, 6)
  }

  "MIVO" should "handle one SensorRecord per sub-meter" in {
    val sub1 = "daq:mivo_json_v1:cust1:gateway1:meter1"
    val sub2 = "daq:mivo_json_v1:cust1:gateway1:meter2"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(
        makeSensor(sub1, 300.0, "2026-01-01T10:00:00Z", sensorType = "mivo_json_v1", sensorId = "meter1"),
        makeSensor(sub2, 450.0, "2026-01-01T10:00:00Z", sensorType = "mivo_json_v1", sensorId = "meter2")
      ),
      mappings = Map(gaugeMapping(sub1, 7), gaugeMapping(sub2, 8))
    )

    result.enrichedRecords should have size 2
    val values = result.enrichedRecords.map(r => (r.logicalId, r.value)).toMap
    values(7) shouldBe 300.0
    values(8) shouldBe 450.0
  }

  "MC603 heat meter" should "produce energy and volume records" in {
    val energyId = "daq:mc603_v1:cust1:serial1:energy"
    val volumeId = "daq:mc603_v1:cust1:serial1:volume"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(
        makeSensor(energyId, 12345.0, "2026-01-01T10:00:00Z", unit = "kWh", sensorType = "mc603_v1", sensorId = "energy"),
        makeSensor(volumeId, 567.8, "2026-01-01T10:00:00Z", unit = "m3", sensorType = "mc603_v1", sensorId = "volume")
      ),
      mappings = Map(gaugeMapping(energyId, 9), gaugeMapping(volumeId, 10))
    )

    result.enrichedRecords should have size 2
    val byId = result.enrichedRecords.map(r => r.logicalId -> r).toMap
    byId(9).value shouldBe 12345.0
    byId(9).unit shouldBe "kWh"
    byId(10).value shouldBe 567.8
    byId(10).unit shouldBe "m3"
  }

  "StdProcessor" should "handle standard JSON format" in {
    val daqId = "daq:std_json_v1:cust1:meter1:energy"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(makeSensor(daqId, 88.8, "2026-01-01T10:00:00Z", sensorType = "std_json_v1")),
      mappings = Map(gaugeMapping(daqId, 11))
    )

    result.enrichedRecords should have size 1
    result.enrichedRecords.head.value shouldBe 88.8
  }

  it should "handle standard JSONL format" in {
    val daqId = "daq:std_jsonl_v1:cust1:meter1:energy"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(makeSensor(daqId, 77.7, "2026-01-01T10:00:00Z", sensorType = "std_jsonl_v1")),
      mappings = Map(gaugeMapping(daqId, 12))
    )

    result.enrichedRecords should have size 1
    result.enrichedRecords.head.value shouldBe 77.7
  }

  "Pulse counter" should "produce counter record with cumulative value" in {
    val daqId = "daq:pulse_v1:cust1:deveui1:count"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(
        makeSensor(daqId, 1000.0, "2026-01-01T10:00:00Z", unit = "Wh", sensorType = "pulse_v1", sensorId = "count"),
        makeSensor(daqId, 1025.0, "2026-01-01T10:15:00Z", unit = "Wh", sensorType = "pulse_v1", sensorId = "count")
      ),
      mappings = Map(counterMapping(daqId, 13))
    )

    result.enrichedRecords should have size 1
    result.enrichedRecords.head.value shouldBe 25.0
  }

  "EDIEL energy data" should "produce gauge records" in {
    val daqId = "daq:ediel_json_v1:cust1:meteringpoint1:energy"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(makeSensor(daqId, 155.3, "2026-01-01T10:00:00Z", unit = "kWh", sensorType = "ediel_json_v1")),
      mappings = Map(gaugeMapping(daqId, 14))
    )

    result.enrichedRecords should have size 1
    result.enrichedRecords.head.value shouldBe 155.3
    result.enrichedRecords.head.logicalId shouldBe 14
  }

  "GWB143 gateway" should "produce sensor records" in {
    val daqId = "daq:gwb143_json_v1:cust1:gateway1:energy"
    val result = ScenarioTestHelper.buildAndRunFromRecords(
      sensorRecords = List(makeSensor(daqId, 999.0, "2026-01-01T10:00:00Z", sensorType = "gwb143_json_v1")),
      mappings = Map(gaugeMapping(daqId, 15))
    )

    result.enrichedRecords should have size 1
    result.enrichedRecords.head.value shouldBe 999.0
  }
}
