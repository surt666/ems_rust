package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class MivoProcessorSpec extends AnyFlatSpec with Matchers {

  "MivoProcessor.transform" should "transform MIVO data correctly" in {
    val inputData = Map(
      "UnitSerial" -> "000080",
      "ProtocolVersion" -> 2,
      "Readings" -> List(
        Map(
          "ID" -> "LUG76842391-4-4",
          "Name" -> "420-008-FJV01",
          "MeterNumber" -> "76842391",
          "MType" -> "Heat",
          "Time" -> "2020-08-19T13:10:01Z",
          "Values" -> List(
            Map("Code" -> "HeatEnergy", "Unit" -> "Wh", "Value" -> 1000),
            Map("Code" -> "HeatVolume", "Unit" -> "m3", "Value" -> 1000),
            Map("Code" -> "HeatPower", "Unit" -> "W", "Value" -> 1000),
            Map("Code" -> "HeatVolumeFlow", "Unit" -> "m3PerHour", "Value" -> 1000),
            Map("Code" -> "HeatForwardTemperature", "Unit" -> "Celcius", "Value" -> 70.5),
            Map("Code" -> "HeatReturnTemperature", "Unit" -> "Celcius", "Value" -> 50)
          )
        )
      ),
      "schematype" -> "mivo_json_v1",
      "customerid" -> "1095"
    )

    val result = MivoProcessor.transform(inputData, "v1")

    // Check that we got exactly 6 records (6 values)
    result should have size 6

    // Define all expected records with ALL fields (sensorId, daqId, type, gatewayId, meterId, timestamp, value, unit)
    val expectedRecords = Seq(
      ("HeatEnergy", "daq:mivo_json_v1:000080:76842391:heatenergy", "mivo_json_v1", "000080", "76842391", "2020-08-19T13:10:01.000000Z", "1000", "Wh"),
      ("HeatVolume", "daq:mivo_json_v1:000080:76842391:heatvolume", "mivo_json_v1", "000080", "76842391", "2020-08-19T13:10:01.000000Z", "1000", "m3"),
      ("HeatPower", "daq:mivo_json_v1:000080:76842391:heatpower", "mivo_json_v1", "000080", "76842391", "2020-08-19T13:10:01.000000Z", "1000", "W"),
      ("HeatVolumeFlow", "daq:mivo_json_v1:000080:76842391:heatvolumeflow", "mivo_json_v1", "000080", "76842391", "2020-08-19T13:10:01.000000Z", "1000", "m3PerHour"),
      ("HeatForwardTemperature", "daq:mivo_json_v1:000080:76842391:heatforwardtemperature", "mivo_json_v1", "000080", "76842391", "2020-08-19T13:10:01.000000Z", "70.5", "Celcius"),
      ("HeatReturnTemperature", "daq:mivo_json_v1:000080:76842391:heatreturntemperature", "mivo_json_v1", "000080", "76842391", "2020-08-19T13:10:01.000000Z", "50", "Celcius")
    )

    // Validate each record completely - checking ALL fields
    expectedRecords.zipWithIndex.foreach { case ((sensorId, daqId, typeVal, gatewayId, meterId, timestamp, value, unit), idx) =>
      val record = result(idx)

      withClue(s"Record $idx ($sensorId) - daqId: ") { record.daqId shouldBe daqId }
      withClue(s"Record $idx ($sensorId) - type: ") { record.`type` shouldBe typeVal }
      withClue(s"Record $idx ($sensorId) - gatewayId: ") { record.gatewayId shouldBe gatewayId }
      withClue(s"Record $idx ($sensorId) - meterId: ") { record.meterId shouldBe meterId }
      withClue(s"Record $idx ($sensorId) - timestamp: ") { record.timestamp shouldBe timestamp }
      withClue(s"Record $idx ($sensorId) - sensorId: ") { record.sensorId shouldBe sensorId }
      withClue(s"Record $idx ($sensorId) - value: ") { record.value shouldBe value }
      withClue(s"Record $idx ($sensorId) - unit: ") { record.unit shouldBe unit }
      withClue(s"Record $idx ($sensorId) - created: ") {
        record.ingestedTime should fullyMatch regex "\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}\\.\\d{6}Z"
      }
    }
  }

  it should "throw exception when required fields are missing" in {
    val inputData = Map(
      "schematype" -> "mivo_json_v1"
      // Missing other required fields
    )

    an[IllegalArgumentException] should be thrownBy {
      MivoProcessor.transform(inputData, "v1")
    }
  }
}
