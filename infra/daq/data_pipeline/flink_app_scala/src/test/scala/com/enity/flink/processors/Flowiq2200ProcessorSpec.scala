package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class Flowiq2200ProcessorSpec extends AnyFlatSpec with Matchers {

  "Flowiq2200Processor.transform" should "decode and transform first FLOWIQ2200 payload correctly" in {
    val inputData = Map(
      "MessageId" -> "707a5dd1-ebbd-4436-b99b-f0e6b1553efe",
      "WirelessDeviceId" -> "966da4ab-9cc4-4dda-8e7c-4175d01ee799",
      "PayloadData" -> "eERtACAoNEQTAAAAAFL/JQAAQv8c/w9iOwAAUjsAAGFbgFFbgA==",
      "WirelessMetadata" -> Map(
        "LoRaWAN" -> Map(
          "ADR" -> true,
          "Bandwidth" -> 125,
          "ClassB" -> false,
          "CodeRate" -> "4/5",
          "DataRate" -> "5",
          "DevAddr" -> "01b88ee3",
          "DevEui" -> "0013ea010337c072",
          "FCnt" -> 2,
          "FOptLen" -> 1,
          "FPort" -> 4,
          "Frequency" -> "867500000",
          "Gateways" -> List(
            Map(
              "GatewayEui" -> "7076ff006405323d",
              "Rssi" -> -31,
              "Snr" -> 8.5
            )
          ),
          "MIC" -> "50e4ab09",
          "MType" -> "UnconfirmedDataUp",
          "Major" -> "LoRaWANR1",
          "Modulation" -> "LORA",
          "PolarizationInversion" -> false,
          "SpreadingFactor" -> 7,
          "Timestamp" -> "2025-04-08T11:50:44Z"
        )
      ),
      "customerid" -> "12345",
      "schematype" -> "flowiq2200_v1"
    )

    val result = Flowiq2200Processor.transform(inputData, "v1")

    // Should have exactly 14 records
    result.size shouldBe 14

    // Define all expected records with ALL fields
    val expectedRecords = Seq(
      ("Volume", "daq:flowiq2200_v1:12345:0013ea010337c072:volume", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "0.0", "m3"),
      ("Dry", "daq:flowiq2200_v1:12345:0013ea010337c072:dry", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "False", "NA"),
      ("Reverse", "daq:flowiq2200_v1:12345:0013ea010337c072:reverse", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "False", "NA"),
      ("Leak", "daq:flowiq2200_v1:12345:0013ea010337c072:leak", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "False", "NA"),
      ("Burst", "daq:flowiq2200_v1:12345:0013ea010337c072:burst", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "False", "NA"),
      ("Tamper", "daq:flowiq2200_v1:12345:0013ea010337c072:tamper", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "False", "NA"),
      ("Low Battery", "daq:flowiq2200_v1:12345:0013ea010337c072:low_battery", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "False", "NA"),
      ("Low Ambient Temperature", "daq:flowiq2200_v1:12345:0013ea010337c072:low_ambient_temperature", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "False", "NA"),
      ("High Ambient Temperature", "daq:flowiq2200_v1:12345:0013ea010337c072:high_ambient_temperature", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "False", "NA"),
      ("ALD last day", "daq:flowiq2200_v1:12345:0013ea010337c072:ald_last_day", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "None", "Invalid"),
      ("Min Volume flow", "daq:flowiq2200_v1:12345:0013ea010337c072:min_volume_flow", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "0.0", "m3/h"),
      ("Max Volume flow", "daq:flowiq2200_v1:12345:0013ea010337c072:max_volume_flow", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "0.0", "m3/h"),
      ("Min Flow temperature", "daq:flowiq2200_v1:12345:0013ea010337c072:min_flow_temperature", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "None", "Invalid"),
      ("Max Flow temperature", "daq:flowiq2200_v1:12345:0013ea010337c072:max_flow_temperature", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-04-08T00:00:00.000000Z", "None", "Invalid")
    )

    // Validate each record completely
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

  it should "decode and transform second FLOWIQ2200 payload correctly" in {
    val inputData = Map(
      "MessageId" -> "606e3404-df8b-428d-b8f7-6e35cf5656f2",
      "WirelessDeviceId" -> "966da4ab-9cc4-4dda-8e7c-4175d01ee799",
      "PayloadData" -> "eERtACAwNkQTq6MEAFL/JQAAQv8cBQBiOwAAUjsAAGFbgFFbgA==",
      "WirelessMetadata" -> Map(
        "LoRaWAN" -> Map(
          "ADR" -> true,
          "Bandwidth" -> 125,
          "ClassB" -> false,
          "CodeRate" -> "4/5",
          "DataRate" -> "5",
          "DevAddr" -> "01089bd1",
          "DevEui" -> "0013ea010337c072",
          "FCnt" -> 368,
          "FOptLen" -> 0,
          "FPort" -> 4,
          "Frequency" -> "868300000",
          "Gateways" -> List(
            Map(
              "GatewayEui" -> "7076ff0064053dd2",
              "Rssi" -> -99,
              "Snr" -> 4.5
            )
          ),
          "MIC" -> "e56320c5",
          "MType" -> "UnconfirmedDataUp",
          "Major" -> "LoRaWANR1",
          "Modulation" -> "LORA",
          "PolarizationInversion" -> false,
          "SpreadingFactor" -> 7,
          "Timestamp" -> "2025-06-16T13:34:47Z"
        )
      ),
      "customerid" -> "12345",
      "schematype" -> "flowiq2200_v1"
    )

    val result = Flowiq2200Processor.transform(inputData, "v1")

    // Should have exactly 14 records
    result.size shouldBe 14

    // Define all expected records with ALL fields (different timestamp and some values from first payload)
    val expectedRecords = Seq(
      ("Volume", "daq:flowiq2200_v1:12345:0013ea010337c072:volume", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "304.043", "m3"),
      ("Dry", "daq:flowiq2200_v1:12345:0013ea010337c072:dry", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "False", "NA"),
      ("Reverse", "daq:flowiq2200_v1:12345:0013ea010337c072:reverse", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "False", "NA"),
      ("Leak", "daq:flowiq2200_v1:12345:0013ea010337c072:leak", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "False", "NA"),
      ("Burst", "daq:flowiq2200_v1:12345:0013ea010337c072:burst", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "False", "NA"),
      ("Tamper", "daq:flowiq2200_v1:12345:0013ea010337c072:tamper", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "False", "NA"),
      ("Low Battery", "daq:flowiq2200_v1:12345:0013ea010337c072:low_battery", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "False", "NA"),
      ("Low Ambient Temperature", "daq:flowiq2200_v1:12345:0013ea010337c072:low_ambient_temperature", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "False", "NA"),
      ("High Ambient Temperature", "daq:flowiq2200_v1:12345:0013ea010337c072:high_ambient_temperature", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "False", "NA"),
      ("ALD last day", "daq:flowiq2200_v1:12345:0013ea010337c072:ald_last_day", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "5.0", "NA"),
      ("Min Volume flow", "daq:flowiq2200_v1:12345:0013ea010337c072:min_volume_flow", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "0.0", "m3/h"),
      ("Max Volume flow", "daq:flowiq2200_v1:12345:0013ea010337c072:max_volume_flow", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "0.0", "m3/h"),
      ("Min Flow temperature", "daq:flowiq2200_v1:12345:0013ea010337c072:min_flow_temperature", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "None", "Invalid"),
      ("Max Flow temperature", "daq:flowiq2200_v1:12345:0013ea010337c072:max_flow_temperature", "flowiq2200_v1", "12345", "0013ea010337c072", "2025-06-16T00:00:00.000000Z", "None", "Invalid")
    )

    // Validate each record completely
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
      "schematype" -> "flowiq2200_v1"
      // Missing other required fields
    )

    an[IllegalArgumentException] should be thrownBy {
      Flowiq2200Processor.transform(inputData, "v1")
    }
  }

  it should "throw exception when PayloadData is not a string" in {
    val inputData = Map(
      "schematype" -> "flowiq2200_v1",
      "customerid" -> "12345",
      "WirelessMetadata" -> Map("LoRaWAN" -> Map("DevEui" -> "test", "Timestamp" -> "2025-04-08T11:50:44Z")),
      "PayloadData" -> 12345 // Not a string
    )

    an[IllegalArgumentException] should be thrownBy {
      Flowiq2200Processor.transform(inputData, "v1")
    }
  }
}
