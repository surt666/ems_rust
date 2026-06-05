package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class Gwb143ProcessorSpec extends AnyFlatSpec with Matchers {

  "Gwb143Processor.transform" should "transform GWB143 data correctly (simplified test)" in {
    val inputData = Map(
      "trbSerial" -> "6003111057",
      "Id" -> "1853052",
      "Manufacturer" -> "ABB",
      "Status" -> "20",
      "data" -> List(
        Map(
          "Function" -> "Instantaneous value",
          "Timestamp" -> "2025-06-23T06:20:16Z",
          "Value" -> "10",
          "id" -> "0",
          "frame" -> "0",
          "Unit" -> "Energy (10 Wh)",
          "StorageNumber" -> "0"
        ),
        Map(
          "id" -> "1",
          "StorageNumber" -> "0",
          "Function" -> "Instantaneous value",
          "Timestamp" -> "2025-06-23T06:20:16Z",
          "Unit" -> "Energy (10 Wh)",
          "Value" -> "20",
          "frame" -> "0",
          "Device" -> "0",
          "Tariff" -> "1"
        ),
        Map(
          "id" -> "10",
          "Function" -> "Instantaneous value",
          "Timestamp" -> "2025-06-23T06:20:16Z",
          "Value" -> "1",
          "frame" -> "0",
          "Unit" -> "Manufacturer specific",
          "StorageNumber" -> "0"
        ),
        Map(
          "id" -> "19",
          "Function" -> "Instantaneous value",
          "Timestamp" -> "2025-06-23T06:20:16Z",
          "Value" -> "B1.30.0",
          "frame" -> "0",
          "Unit" -> "Firmware version",
          "StorageNumber" -> "0"
        ),
        Map(
          "id" -> "23",
          "Function" -> "Instantaneous value",
          "Timestamp" -> "2025-06-23T06:20:17Z",
          "Value" -> "0",
          "frame" -> "1",
          "Unit" -> "Power (1e-2 W)",
          "StorageNumber" -> "0"
        ),
        Map(
          "id" -> "67",
          "StorageNumber" -> "0",
          "Function" -> "Instantaneous value",
          "Timestamp" -> "2025-06-23T06:20:20Z",
          "Unit" -> "Digital output (binary)",
          "Value" -> "0",
          "frame" -> "3",
          "Device" -> "1",
          "Tariff" -> "0"
        ),
        Map(
          "id" -> "100",
          "Function" -> "Instantaneous value",
          "Timestamp" -> "2025-06-23T06:20:21Z",
          "Value" -> "5",
          "frame" -> "4",
          "Unit" -> "dimensionless",  // Should be filtered out
          "StorageNumber" -> "0"
        ),
        Map(
          "id" -> "101",
          "Function" -> "Instantaneous value",
          "Timestamp" -> "2025-06-23T06:20:21Z",
          "Value" -> "invalid",  // Should be filtered out
          "frame" -> "4",
          "Unit" -> "Energy (10 Wh)",
          "StorageNumber" -> "0"
        )
      ),
      "schematype" -> "gwb143_json_v1",
      "customerid" -> "1095"
    )

    val result = Gwb143Processor.transform(inputData, "v1")

    // Should filter out dimensionless units, invalid values, and non-numeric values
    // From 8 input records, 3 should be filtered out (id=19 non-numeric, id=100 dimensionless, id=101 invalid)
    // Valid records: id=0, 1, 10, 23, 67
    result should have size 5

    // Define all expected records with ALL fields (sensorId, daqId, type, gatewayId, meterId, timestamp, value, unit)
    val expectedRecords = Seq(
      ("0", "daq:gwb143_json_v1:6003111057:1853052:0", "gwb143_json_v1", "6003111057", "1853052", "2025-06-23T06:20:16.000000Z", "10", "Energy (10 Wh)"),
      ("1", "daq:gwb143_json_v1:6003111057:1853052:1", "gwb143_json_v1", "6003111057", "1853052", "2025-06-23T06:20:16.000000Z", "20", "Energy (10 Wh)"),
      ("10", "daq:gwb143_json_v1:6003111057:1853052:10", "gwb143_json_v1", "6003111057", "1853052", "2025-06-23T06:20:16.000000Z", "1", "Manufacturer specific"),
      ("23", "daq:gwb143_json_v1:6003111057:1853052:23", "gwb143_json_v1", "6003111057", "1853052", "2025-06-23T06:20:17.000000Z", "0", "Power (1e-2 W)"),
      ("67", "daq:gwb143_json_v1:6003111057:1853052:67", "gwb143_json_v1", "6003111057", "1853052", "2025-06-23T06:20:20.000000Z", "0", "Digital output (binary)")
    )

    // Validate each record completely - checking ALL fields
    expectedRecords.foreach { case (sensorId, daqId, typeVal, gatewayId, meterId, timestamp, value, unit) =>
      val record = result.find(_.sensorId == sensorId).getOrElse(fail(s"Record with sensorId '$sensorId' not found"))

      withClue(s"Record $sensorId - daqId: ") { record.daqId shouldBe daqId }
      withClue(s"Record $sensorId - type: ") { record.`type` shouldBe typeVal }
      withClue(s"Record $sensorId - gatewayId: ") { record.gatewayId shouldBe gatewayId }
      withClue(s"Record $sensorId - meterId: ") { record.meterId shouldBe meterId }
      withClue(s"Record $sensorId - timestamp: ") { record.timestamp shouldBe timestamp }
      withClue(s"Record $sensorId - sensorId: ") { record.sensorId shouldBe sensorId }
      withClue(s"Record $sensorId - value: ") { record.value shouldBe value }
      withClue(s"Record $sensorId - unit: ") { record.unit shouldBe unit }
      withClue(s"Record $sensorId - created: ") {
        record.ingestedTime should fullyMatch regex "\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}\\.\\d{6}Z"
      }
    }

    // Verify that filtered records are NOT present
    result.exists(_.sensorId == "19") shouldBe false  // Non-numeric value
    result.exists(_.sensorId == "100") shouldBe false  // Dimensionless unit
    result.exists(_.sensorId == "101") shouldBe false  // Invalid value
  }

  it should "throw exception when required fields are missing" in {
    val inputData = Map(
      "schematype" -> "gwb143_json_v1"
      // Missing other required fields
    )

    an[IllegalArgumentException] should be thrownBy {
      Gwb143Processor.transform(inputData, "v1")
    }
  }

  it should "return empty sequence when data field is not a list" in {
    val inputData = Map(
      "trbSerial" -> "6003111057",
      "data" -> "not-a-list",
      "customerid" -> "1095",
      "schematype" -> "gwb143_json_v1",
      "Id" -> "1853052"
    )

    // Should return default record when data is not a list
    val result = Gwb143Processor.transform(inputData, "v1")
    result should not be empty
  }
}
