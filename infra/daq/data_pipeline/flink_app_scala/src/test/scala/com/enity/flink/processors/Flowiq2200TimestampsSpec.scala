package com.enity.flink.processors

import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class Flowiq2200TimestampsSpec extends AnyFlatSpec with Matchers {

  "Flowiq2200Processor.transform" should "handle inverse compact profile with correct timestamps" in {
    val inputData = Map(
      "MessageId" -> "ff6778fe-4013-49a6-b33a-5365f1b5fb98",
      "WirelessDeviceId" -> "966da4ab-9cc4-4dda-8e7c-4175d01ee799",
      "PayloadData" -> "eERtACQ0OkQTAd0UAE2TEyJiAX0BegF9AXkBfgF5AX0BegF9AXkBfAF6AXwBeQF9AXoB",
      "WirelessMetadata" -> Map(
        "LoRaWAN" -> Map(
          "ADR" -> true,
          "Bandwidth" -> 125,
          "ClassB" -> false,
          "CodeRate" -> "4/5",
          "DataRate" -> "5",
          "DevAddr" -> "01a44424",
          "DevEui" -> "0013ea010337c072",
          "FCnt" -> 438,
          "FOptLen" -> 0,
          "FPort" -> 4,
          "Frequency" -> "868100000",
          "Gateways" -> List(
            Map(
              "GatewayEui" -> "7076ff0064053dd2",
              "Rssi" -> -33,
              "Snr" -> 9.75
            )
          ),
          "MIC" -> "d6a0de02",
          "MType" -> "UnconfirmedDataUp",
          "Major" -> "LoRaWANR1",
          "Modulation" -> "LORA",
          "PolarizationInversion" -> false,
          "SpreadingFactor" -> 7,
          "Timestamp" -> "2025-10-20T03:12:48Z"
        )
      ),
      "customerid" -> "123",
      "schematype" -> "flowiq2200_v1"
    )

    val result = Flowiq2200Processor.transform(inputData, "v1")

    // Should have exactly 17 volume records from inverse compact profile
    result.size shouldBe 17

    // Define all expected records with ALL fields (sensorId, daqId, type, gatewayId, meterId, timestamp, value, unit)
    val expectedRecords = Seq(
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-20T04:00:00.000000Z", "1367.297", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-20T03:00:00.000000Z", "1366.916", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-20T02:00:00.000000Z", "1366.538", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-20T01:00:00.000000Z", "1366.157", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-20T00:00:00.000000Z", "1365.78", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T23:00:00.000000Z", "1365.398", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T22:00:00.000000Z", "1365.021", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T21:00:00.000000Z", "1364.64", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T20:00:00.000000Z", "1364.262", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T19:00:00.000000Z", "1363.881", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T18:00:00.000000Z", "1363.504", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T17:00:00.000000Z", "1363.124", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T16:00:00.000000Z", "1362.746", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T15:00:00.000000Z", "1362.366", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T14:00:00.000000Z", "1361.989", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T13:00:00.000000Z", "1361.608", "m3"),
      ("Volume", "daq:flowiq2200_v1:123:0013ea010337c072:volume", "flowiq2200_v1", "123", "0013ea010337c072", "2025-10-19T12:00:00.000000Z", "1361.23", "m3")
    )

    // Validate each record completely - checking ALL fields
    expectedRecords.zipWithIndex.foreach { case ((sensorId, daqId, typeVal, gatewayId, meterId, timestamp, value, unit), idx) =>
      val record = result(idx)

      withClue(s"Record $idx - daqId: ") { record.daqId shouldBe daqId }
      withClue(s"Record $idx - type: ") { record.`type` shouldBe typeVal }
      withClue(s"Record $idx - gatewayId: ") { record.gatewayId shouldBe gatewayId }
      withClue(s"Record $idx - meterId: ") { record.meterId shouldBe meterId }
      withClue(s"Record $idx - timestamp: ") { record.timestamp shouldBe timestamp }
      withClue(s"Record $idx - sensorId: ") { record.sensorId shouldBe sensorId }
      withClue(s"Record $idx - value: ") { record.value shouldBe value }
      withClue(s"Record $idx - unit: ") { record.unit shouldBe unit }
      withClue(s"Record $idx - created: ") {
        record.ingestedTime should fullyMatch regex "\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}\\.\\d{6}Z"
      }
    }
  }
}
