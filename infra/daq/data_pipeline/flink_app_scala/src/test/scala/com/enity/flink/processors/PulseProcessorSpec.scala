package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class PulseProcessorSpec extends AnyFlatSpec with Matchers {

  "PulseProcessor.transform" should "decode and transform Pulse payload correctly" in {
    val inputData = Map(
      "MessageId" -> "8ea27b91-4884-4d0a-a24c-06570ff3151c",
      "WirelessDeviceId" -> "cafc5d31-4011-45a6-b5cc-c2b94bb55973",
      "PayloadData" -> "RgAAAHi/AAAAAA==",
      "WirelessMetadata" -> Map(
        "LoRaWAN" -> Map(
          "ADR" -> true,
          "Bandwidth" -> 125,
          "ClassB" -> false,
          "CodeRate" -> "4/5",
          "DataRate" -> "5",
          "DevAddr" -> "01e3c3ab",
          "DevEui" -> "0018b210000191c7",
          "FCnt" -> 2881,
          "FOptLen" -> 1,
          "FPort" -> 1,
          "Frequency" -> "867500000",
          "Gateways" -> List(
            Map(
              "GatewayEui" -> "7076ff0064053dd2",
              "Rssi" -> -106,
              "Snr" -> -2
            )
          ),
          "MIC" -> "7bada29c",
          "MType" -> "UnconfirmedDataUp",
          "Major" -> "LoRaWANR1",
          "Modulation" -> "LORA",
          "PolarizationInversion" -> false,
          "SpreadingFactor" -> 7,
          "Timestamp" -> "2025-06-12T06:24:33Z"
        )
      ),
      "customerid" -> "123",
      "schematype" -> "pulse_v1"
    )

    val result = PulseProcessor.transform(inputData, "v1")

    // Should return exactly 2 records (counter_a and counter_b)
    result should have size 2

    // Define all expected records with ALL fields (sensorId, daqId, type, gatewayId, meterId, timestamp, value, unit)
    val expectedRecords = Seq(
      ("counter_a", "daq:pulse_v1:123:0018b210000191c7:counter_a", "pulse_v1", "123", "0018b210000191c7", "2025-06-12T06:24:33.000000Z", "30911", "NA"),
      ("counter_b", "daq:pulse_v1:123:0018b210000191c7:counter_b", "pulse_v1", "123", "0018b210000191c7", "2025-06-12T06:24:33.000000Z", "0", "NA")
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
      "schematype" -> "pulse_v1"
      // Missing other required fields
    )

    an[IllegalArgumentException] should be thrownBy {
      PulseProcessor.transform(inputData, "v1")
    }
  }

  it should "throw exception when PayloadData is not a string" in {
    val inputData = Map(
      "schematype" -> "pulse_v1",
      "customerid" -> "123",
      "WirelessMetadata" -> Map("LoRaWAN" -> Map("DevEui" -> "test", "Timestamp" -> "2025-06-12T06:24:33Z")),
      "PayloadData" -> 12345 // Not a string
    )

    an[IllegalArgumentException] should be thrownBy {
      PulseProcessor.transform(inputData, "v1")
    }
  }
}
