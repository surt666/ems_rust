package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class Mc603ProcessorSpec extends AnyFlatSpec with Matchers {

  "Mc603Processor.transform" should "decode and transform MC603 payload correctly" in {
    val inputData = Map(
      "MessageId" -> "5e6c3088-156e-443e-b76d-668d12ac5b20",
      "WirelessDeviceId" -> "530948f9-9b11-42c6-8463-43d0fd69e30d",
      "PayloadData" -> "FQQAAAAAAAQRAAAAAAIqAAACOwAAAlntCAJd8QgMeGMjU4UE/RcAAQAA",
      "WirelessMetadata" -> Map(
        "LoRaWAN" -> Map(
          "ADR" -> true,
          "Bandwidth" -> 125,
          "ClassB" -> false,
          "CodeRate" -> "4/5",
          "DataRate" -> "5",
          "DevAddr" -> "01006db3",
          "DevEui" -> "102ceffffe010ef5",
          "FCnt" -> 1166,
          "FOptLen" -> 0,
          "FPort" -> 1,
          "Frequency" -> "867100000",
          "Gateways" -> List(
            Map("GatewayEui" -> "7076ff006405323d", "Rssi" -> -100, "Snr" -> 4.25)
          ),
          "MIC" -> "5078be03",
          "MType" -> "UnconfirmedDataUp",
          "Major" -> "LoRaWANR1",
          "Modulation" -> "LORA",
          "PolarizationInversion" -> false,
          "SpreadingFactor" -> 7,
          "Timestamp" -> "2024-11-24T03:15:28Z"
        )
      ),
      "customerid" -> "12345",
      "schematype" -> "mc603_v1"
    )

    val result = Mc603Processor.transform(inputData, "v1")

    // Should return exactly 6 records (heat_energy, volume, power, flow, forward_temp, return_temp)
    result should have size 6

    // Define all expected records with ALL fields (sensorId, daqId, type, gatewayId, meterId, timestamp, value, unit)
    val expectedRecords = Seq(
      ("heat_energy", "daq:mc603_v1:12345:102ceffffe010ef5:heat_energy", "mc603_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:28.000000Z", "0.0", "Wh"),
      ("volume", "daq:mc603_v1:12345:102ceffffe010ef5:volume", "mc603_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:28.000000Z", "0.0", "m^3"),
      ("power", "daq:mc603_v1:12345:102ceffffe010ef5:power", "mc603_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:28.000000Z", "0", "W"),
      ("flow", "daq:mc603_v1:12345:102ceffffe010ef5:flow", "mc603_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:28.000000Z", "0.0", "m^3/h"),
      ("forward_temperature", "daq:mc603_v1:12345:102ceffffe010ef5:forward_temperature", "mc603_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:28.000000Z", "22.85", "C"),
      ("return_temperature", "daq:mc603_v1:12345:102ceffffe010ef5:return_temperature", "mc603_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:28.000000Z", "22.89", "C")
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
  }

  it should "throw exception when required fields are missing" in {
    val inputData = Map(
      "schematype" -> "mc603_v1"
      // Missing other required fields
    )

    an[IllegalArgumentException] should be thrownBy {
      Mc603Processor.transform(inputData, "v1")
    }
  }

  it should "throw exception when PayloadData is not a string" in {
    val inputData = Map(
      "schematype" -> "mc603_v1",
      "customerid" -> "12345",
      "WirelessMetadata" -> Map("LoRaWAN" -> Map("DevEui" -> "test", "Timestamp" -> "2024-11-24T03:15:28Z")),
      "PayloadData" -> 12345 // Not a string
    )

    an[IllegalArgumentException] should be thrownBy {
      Mc603Processor.transform(inputData, "v1")
    }
  }
}
