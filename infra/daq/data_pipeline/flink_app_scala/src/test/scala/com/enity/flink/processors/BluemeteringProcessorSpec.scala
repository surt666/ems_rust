package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class BluemeteringProcessorSpec extends AnyFlatSpec with Matchers {

  "BluemeteringProcessor.transform" should "transform Bluemetering JSON data correctly" in {
    val inputData = Map(
      "dateTime" -> "2024-06-24T16:15:00+02:00",
      "type" -> "REAL_VALUE",
      "error" -> "NONE",
      "dataSource" -> "BRICK4U",
      "obisIndex" -> "1-1:1.8.0",
      "value" -> 0.0,
      "unit" -> "KILO_WATT_HOUR",
      "tafId" -> null,
      "transformerRatioU" -> null,
      "transformerRatioI" -> null,
      "meteringLocationId" -> "DE0000000000000000SUB010000014456",
      "marketLocationId" -> null,
      "meterSerialNumber" -> "00020905",
      "schematype" -> "bluemetering_json_v1",
      "customerid" -> "1095"
    )

    val result = BluemeteringProcessor.transform(inputData, "v1")

    // Check that we got exactly 1 record
    result should have size 1

    // Validate ALL fields on the record
    val record = result.head

    withClue("daqId: ") {
      record.daqId shouldBe "daq:bluemetering_json_v1:1095:00020905:1_1:1_8_0"
    }
    withClue("type: ") {
      record.`type` shouldBe "bluemetering_json_v1"
    }
    withClue("gatewayId: ") {
      record.gatewayId shouldBe "BRICK4U"
    }
    withClue("meterId: ") {
      record.meterId shouldBe "00020905"
    }
    withClue("timestamp: ") {
      record.timestamp shouldBe "2024-06-24T14:15:00.000000Z"
    }
    withClue("sensorId: ") {
      record.sensorId shouldBe "1-1:1.8.0"
    }
    withClue("value: ") {
      record.value shouldBe "0.0"
    }
    withClue("unit: ") {
      record.unit shouldBe "KILO_WATT_HOUR"
    }
    withClue("created: ") {
      record.ingestedTime should fullyMatch regex "\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}\\.\\d{6}Z"
    }
  }

  it should "throw exception when required fields are missing" in {
    val inputData = Map(
      "schematype" -> "bluemetering_json_v1"
      // Missing other required fields
    )

    an[IllegalArgumentException] should be thrownBy {
      BluemeteringProcessor.transform(inputData, "v1")
    }
  }
}
