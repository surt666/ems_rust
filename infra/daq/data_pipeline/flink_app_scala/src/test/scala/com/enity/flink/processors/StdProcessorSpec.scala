package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class StdProcessorSpec extends AnyFlatSpec with Matchers {

  "StdProcessor.transformStdJson" should "transform STD JSON data correctly" in {
    val inputData = Map(
      "data" -> List(
        Map(
          "created" -> "2025-06-23T09:32:49.371052Z",
          "daq_id" -> "daq:std_json_v1:iotfabrikken:33333:temperature",
          "gatewayid" -> "None",
          "meterid" -> "33333",
          "sensorid" -> "temperature",
          "timestamp" -> "2025-05-19T07:03:03.000000Z",
          "unit" -> "C",
          "value" -> 21.79
        ),
        Map(
          "created" -> "2025-06-23T09:32:49.371052Z",
          "daq_id" -> "daq:std_json_v1:iotfabrikken:33333:humidity",
          "gatewayid" -> "None",
          "meterid" -> "33333",
          "sensorid" -> "humidity",
          "timestamp" -> "2025-05-19T07:03:03.000000Z",
          "unit" -> "RH%",
          "value" -> 44.39
        ),
        Map(
          "created" -> "2025-06-23T09:32:49.371052Z",
          "daq_id" -> "daq:std_json_v1:iotfabrikken:33333:voc",
          "gatewayid" -> "None",
          "meterid" -> "33333",
          "sensorid" -> "voc",
          "timestamp" -> "2025-05-19T07:03:03.000000Z",
          "unit" -> "ppb",
          "value" -> 130
        ),
        Map(
          "created" -> "2025-06-23T09:32:49.371052Z",
          "daq_id" -> "daq:std_json_v1:iotfabrikken:33333:co2",
          "gatewayid" -> "None",
          "meterid" -> "33333",
          "sensorid" -> "co2",
          "timestamp" -> "2025-05-19T07:03:03.000000Z",
          "unit" -> "ppm",
          "value" -> 505
        ),
        Map(
          "created" -> "2025-06-23T09:32:49.371052Z",
          "daq_id" -> "daq:std_json_v1:iotfabrikken:33333:occupancy",
          "gatewayid" -> "None",
          "meterid" -> "33333",
          "sensorid" -> "occupancy",
          "timestamp" -> "2025-05-19T07:03:03.000000Z",
          "unit" -> "",
          "value" -> 0
        )
      ),
      "schematype" -> "std_json_v1",
      "customerid" -> "iotfabrikken"
    )

    val result = StdProcessor.transformStdJson(inputData, "v1")

    // Should return exactly 5 records
    result should have size 5

    // Define all expected records with ALL fields (sensorId, daqId, type, gatewayId, meterId, timestamp, value, unit)
    val expectedRecords = Seq(
      ("temperature", "daq:std_json_v1:iotfabrikken:33333:temperature", "std_json_v1", "iotfabrikken", "33333", "2025-05-19T07:03:03.000000Z", "21.79", "C"),
      ("humidity", "daq:std_json_v1:iotfabrikken:33333:humidity", "std_json_v1", "iotfabrikken", "33333", "2025-05-19T07:03:03.000000Z", "44.39", "RH%"),
      ("voc", "daq:std_json_v1:iotfabrikken:33333:voc", "std_json_v1", "iotfabrikken", "33333", "2025-05-19T07:03:03.000000Z", "130", "ppb"),
      ("co2", "daq:std_json_v1:iotfabrikken:33333:co2", "std_json_v1", "iotfabrikken", "33333", "2025-05-19T07:03:03.000000Z", "505", "ppm"),
      ("occupancy", "daq:std_json_v1:iotfabrikken:33333:occupancy", "std_json_v1", "iotfabrikken", "33333", "2025-05-19T07:03:03.000000Z", "0", "")
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

  it should "handle gatewayid field with fallback logic" in {
    val inputData = Map(
      "data" -> List(
        Map(
          "gatewayid" -> "actual-gateway",
          "meterid" -> "meter1",
          "sensorid" -> "temp",
          "timestamp" -> "2025-05-19T07:03:03.000000Z",
          "unit" -> "C",
          "value" -> 25.5
        ),
        Map(
          "gatewayid" -> "None",
          "meterid" -> "meter2",
          "sensorid" -> "temp",
          "timestamp" -> "2025-05-19T07:03:03.000000Z",
          "unit" -> "C",
          "value" -> 26.5
        )
      ),
      "schematype" -> "std_json_v1",
      "customerid" -> "testcustomer"
    )

    val result = StdProcessor.transformStdJson(inputData, "v1")

    // Should return exactly 2 records
    result should have size 2

    // Define all expected records with ALL fields (sensorId, daqId, type, gatewayId, meterId, timestamp, value, unit)
    // Note: daqId always uses customerid, while gatewayId uses the actual value or fallback
    val expectedRecords = Seq(
      ("temp", "daq:std_json_v1:testcustomer:meter1:temp", "std_json_v1", "actual-gateway", "meter1", "2025-05-19T07:03:03.000000Z", "25.5", "C"),
      ("temp", "daq:std_json_v1:testcustomer:meter2:temp", "std_json_v1", "testcustomer", "meter2", "2025-05-19T07:03:03.000000Z", "26.5", "C")
    )

    // Validate each record completely - checking ALL fields
    expectedRecords.zipWithIndex.foreach { case ((sensorId, daqId, typeVal, gatewayId, meterId, timestamp, value, unit), idx) =>
      val record = result(idx)

      withClue(s"Record $idx (meter=$meterId) - daqId: ") { record.daqId shouldBe daqId }
      withClue(s"Record $idx (meter=$meterId) - type: ") { record.`type` shouldBe typeVal }
      withClue(s"Record $idx (meter=$meterId) - gatewayId: ") { record.gatewayId shouldBe gatewayId }
      withClue(s"Record $idx (meter=$meterId) - meterId: ") { record.meterId shouldBe meterId }
      withClue(s"Record $idx (meter=$meterId) - timestamp: ") { record.timestamp shouldBe timestamp }
      withClue(s"Record $idx (meter=$meterId) - sensorId: ") { record.sensorId shouldBe sensorId }
      withClue(s"Record $idx (meter=$meterId) - value: ") { record.value shouldBe value }
      withClue(s"Record $idx (meter=$meterId) - unit: ") { record.unit shouldBe unit }
      withClue(s"Record $idx (meter=$meterId) - created: ") {
        record.ingestedTime should fullyMatch regex "\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}\\.\\d{6}Z"
      }
    }
  }

  it should "throw exception when required fields are missing" in {
    val inputData = Map(
      "schematype" -> "std_json_v1"
      // Missing customerid and data
    )

    an[IllegalArgumentException] should be thrownBy {
      StdProcessor.transformStdJson(inputData, "v1")
    }
  }
}
