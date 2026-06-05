package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class EdielProcessorSpec extends AnyFlatSpec with Matchers {

  "EdielProcessor.transformJsonV1" should "transform EDIEL JSON v1 data correctly" in {
    val inputData = Map(
      "schematype" -> "ediel_json_v1",
      "customerid" -> "91100",
      "data" -> List(
        Map(
          "meterid" -> "735999888000004015",
          "timestamp" -> "2025-06-30 23:00:00",
          "sensorid" -> "TOTAL",
          "value" -> "38741",
          "unit" -> "KWH"
        )
      )
    )

    val result = EdielProcessor.transformJsonV1(inputData, "v1")

    // Check that we got exactly 1 record
    result should have size 1

    // Validate ALL fields on the record
    val record = result.head

    withClue("daqId: ") {
      record.daqId shouldBe "daq:ediel_json_v1:91100:735999888000004015:total"
    }
    withClue("type: ") {
      record.`type` shouldBe "ediel_json_v1"
    }
    withClue("gatewayId: ") {
      record.gatewayId shouldBe "91100"
    }
    withClue("meterId: ") {
      record.meterId shouldBe "735999888000004015"
    }
    withClue("timestamp: ") {
      record.timestamp shouldBe "2025-06-30T23:00:00.000000Z"
    }
    withClue("sensorId: ") {
      record.sensorId shouldBe "TOTAL"
    }
    withClue("value: ") {
      record.value shouldBe "38741"
    }
    withClue("unit: ") {
      record.unit shouldBe "KWH"
    }
    withClue("created: ") {
      record.ingestedTime should fullyMatch regex "\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}\\.\\d{6}Z"
    }
  }

  it should "return empty sequence when data field is not a list" in {
    val inputData = Map(
      "schematype" -> "ediel_json_v1",
      "customerid" -> "91100",
      "data" -> "not-a-list"
    )

    val result = EdielProcessor.transformJsonV1(inputData, "v1")

    result shouldBe empty
  }

  it should "return empty when schematype or data fields are missing" in {
    val inputData1 = Map(
      "customerid" -> "91100"
      // Missing schematype
    )
    EdielProcessor.transformJsonV1(inputData1, "v1") shouldBe empty

    val inputData2 = Map(
      "schematype" -> "ediel_json_v1",
      "customerid" -> "91100"
      // Missing data field - returns empty, not exception
    )
    EdielProcessor.transformJsonV1(inputData2, "v1") shouldBe empty
  }
}
