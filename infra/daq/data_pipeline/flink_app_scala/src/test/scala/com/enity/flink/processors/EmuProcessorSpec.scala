package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class EmuProcessorSpec extends AnyFlatSpec with Matchers {

  "EmuProcessor.transform" should "decode and transform EMU payload correctly" in {
    val inputData = Map(
      "MessageId" -> "5e6c3088-156e-443e-b76d-668d12ac5b20",
      "WirelessDeviceId" -> "530948f9-9b11-42c6-8463-43d0fd69e30d",
      "PayloadData" -> "NJpCZwNtCwAABAAAAAAFAAAAAAYAAAAA8ACu",
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
      "schematype" -> "emu_v1"
    )

    val result = EmuProcessor.transform(inputData, "v1")

    // Should return exactly 4 records (0x3, 0x4, 0x5, 0x6)
    result should have size 4

    // Define all expected records with ALL fields (sensorId, daqId, type, gatewayId, meterId, timestamp, value, unit)
    val expectedRecords = Seq(
      ("0x3", "daq:emu_v1:12345:102ceffffe010ef5:0x3", "emu_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:00.000000Z", "2925", "Wh"),
      ("0x4", "daq:emu_v1:12345:102ceffffe010ef5:0x4", "emu_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:00.000000Z", "0", "Wh"),
      ("0x5", "daq:emu_v1:12345:102ceffffe010ef5:0x5", "emu_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:00.000000Z", "0", "Wh"),
      ("0x6", "daq:emu_v1:12345:102ceffffe010ef5:0x6", "emu_v1", "12345", "102ceffffe010ef5", "2024-11-24T03:15:00.000000Z", "0", "Wh")
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

  it should "handle another EMU payload correctly" in {
    val inputData = Map(
      "MessageId" -> "475e8000-e850-473a-bcf2-053f6655ae86",
      "WirelessDeviceId" -> "530948f9-9b11-42c6-8463-43d0fd69e30d",
      "PayloadData" -> "gI17ZwMXIgAABAAAAAAFAAAAAAYAAAAA8AC1",
      "WirelessMetadata" -> Map(
        "LoRaWAN" -> Map(
          "ADR" -> true,
          "Bandwidth" -> 125,
          "ClassB" -> false,
          "CodeRate" -> "4/5",
          "DataRate" -> "5",
          "DevAddr" -> "00bec848",
          "DevEui" -> "102ceffffe010ef5",
          "FCnt" -> 1994,
          "FOptLen" -> 0,
          "FPort" -> 1,
          "Frequency" -> "867700000",
          "Gateways" -> List(
            Map("GatewayEui" -> "7076ff006405323d", "Rssi" -> -106, "Snr" -> -3.25)
          ),
          "MIC" -> "e5420b58",
          "MType" -> "UnconfirmedDataUp",
          "Major" -> "LoRaWANR1",
          "Modulation" -> "LORA",
          "PolarizationInversion" -> false,
          "SpreadingFactor" -> 7,
          "Timestamp" -> "2025-01-06T08:05:20Z"
        )
      ),
      "customerid" -> "123",
      "schematype" -> "emu_v1"
    )

    val result = EmuProcessor.transform(inputData, "v1")

    // Should return exactly 4 records
    result should have size 4

    // Define all expected records with ALL fields (sensorId, daqId, type, gatewayId, meterId, timestamp, value, unit)
    val expectedRecords = Seq(
      ("0x3", "daq:emu_v1:123:102ceffffe010ef5:0x3", "emu_v1", "123", "102ceffffe010ef5", "2025-01-06T08:00:00.000000Z", "8727", "Wh"),
      ("0x4", "daq:emu_v1:123:102ceffffe010ef5:0x4", "emu_v1", "123", "102ceffffe010ef5", "2025-01-06T08:00:00.000000Z", "0", "Wh"),
      ("0x5", "daq:emu_v1:123:102ceffffe010ef5:0x5", "emu_v1", "123", "102ceffffe010ef5", "2025-01-06T08:00:00.000000Z", "0", "Wh"),
      ("0x6", "daq:emu_v1:123:102ceffffe010ef5:0x6", "emu_v1", "123", "102ceffffe010ef5", "2025-01-06T08:00:00.000000Z", "0", "Wh")
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
      "schematype" -> "emu_v1"
      // Missing other required fields
    )

    an[IllegalArgumentException] should be thrownBy {
      EmuProcessor.transform(inputData, "v1")
    }
  }

  it should "throw exception when PayloadData is not a string" in {
    val inputData = Map(
      "schematype" -> "emu_v1",
      "customerid" -> "12345",
      "WirelessMetadata" -> Map("LoRaWAN" -> Map("DevEui" -> "test")),
      "PayloadData" -> 12345 // Not a string
    )

    an[IllegalArgumentException] should be thrownBy {
      EmuProcessor.transform(inputData, "v1")
    }
  }
}
