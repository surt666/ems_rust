package com.enity.flink.utils

import com.enity.flink.SensorRecord
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

import java.time.Instant
import java.time.format.DateTimeFormatter

class ProcessUtilsSpec extends AnyFlatSpec with Matchers {

  val isoFormatter: DateTimeFormatter = DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ss.SSSSSS'Z'")
    .withZone(java.time.ZoneOffset.UTC)

  "ProcessUtils.filterValid" should "filter out records with invalid values" in {
    val validRecord = SensorRecord(
      daqId = "daq:test:customer:meter:sensor",
      `type` = "test",
      gatewayId = "gateway1",
      meterId = "meter1",
      timestamp = "2025-01-06T08:00:00.000000Z",
      ingestedTime = isoFormatter.format(Instant.now()),
      sensorId = "sensor1",
      value = "123.45",
      unit = "kWh"
    )

    val invalidValueRecord = validRecord.copy(value = "not-a-number")

    val records = Seq(validRecord, invalidValueRecord)
    val filtered = ProcessUtils.filterValid(records)

    filtered should have size 1
    filtered.head shouldBe validRecord
  }

  it should "filter out records with invalid timestamps" in {
    val validRecord = SensorRecord(
      daqId = "daq:test:customer:meter:sensor",
      `type` = "test",
      gatewayId = "gateway1",
      meterId = "meter1",
      timestamp = "2025-01-06T08:00:00.000000Z",
      ingestedTime = isoFormatter.format(Instant.now()),
      sensorId = "sensor1",
      value = "123.45",
      unit = "kWh"
    )

    val invalidTimestampRecord = validRecord.copy(timestamp = "not-a-timestamp")

    val records = Seq(validRecord, invalidTimestampRecord)
    val filtered = ProcessUtils.filterValid(records)

    filtered should have size 1
    filtered.head shouldBe validRecord
  }

  it should "filter out null records" in {
    val validRecord = SensorRecord(
      daqId = "daq:test:customer:meter:sensor",
      `type` = "test",
      gatewayId = "gateway1",
      meterId = "meter1",
      timestamp = "2025-01-06T08:00:00.000000Z",
      ingestedTime = isoFormatter.format(Instant.now()),
      sensorId = "sensor1",
      value = "123.45",
      unit = "kWh"
    )

    val records = Seq(validRecord, null, validRecord)
    val filtered = ProcessUtils.filterValid(records)

    filtered should have size 2
  }

  it should "accept records with valid data" in {
    val record1 = SensorRecord(
      daqId = "daq:test:customer:meter:sensor1",
      `type` = "test",
      gatewayId = "gateway1",
      meterId = "meter1",
      timestamp = "2025-01-06T08:00:00.000000Z",
      ingestedTime = isoFormatter.format(Instant.now()),
      sensorId = "sensor1",
      value = "123.45",
      unit = "kWh"
    )

    val record2 = SensorRecord(
      daqId = "daq:test:customer:meter:sensor2",
      `type` = "test",
      gatewayId = "gateway1",
      meterId = "meter1",
      timestamp = "2025-01-06T09:00:00.000000Z",
      ingestedTime = isoFormatter.format(Instant.now()),
      sensorId = "sensor2",
      value = "0",
      unit = "kWh"
    )

    val records = Seq(record1, record2)
    val filtered = ProcessUtils.filterValid(records)

    filtered should have size 2
    filtered shouldBe records
  }

  "ProcessUtils.processDataToRecords" should "process valid data and return records" in {
    def mockTransform(data: Map[String, Any], version: String): Seq[SensorRecord] = {
      Seq(
        SensorRecord(
          daqId = "daq:test:customer:meter:sensor",
          `type` = "test",
          gatewayId = "gateway1",
          meterId = "meter1",
          timestamp = "2025-01-06T08:00:00.000000Z",
          ingestedTime = isoFormatter.format(Instant.now()),
          sensorId = "sensor1",
          value = "123.45",
          unit = "kWh"
        )
      )
    }

    val data = Map("test" -> "data")
    val result = ProcessUtils.processDataToRecords("TEST", mockTransform, data, "v1")

    result should have size 1
    result.head.daqId shouldBe "daq:test:customer:meter:sensor"
    result.head.value shouldBe "123.45"
    result.head.unit shouldBe "kWh"
  }

  it should "filter out invalid records from transform results" in {
    def mockTransform(data: Map[String, Any], version: String): Seq[SensorRecord] = {
      Seq(
        SensorRecord(
          daqId = "daq:test:customer:meter:sensor1",
          `type` = "test",
          gatewayId = "gateway1",
          meterId = "meter1",
          timestamp = "2025-01-06T08:00:00.000000Z",
          ingestedTime = isoFormatter.format(Instant.now()),
          sensorId = "sensor1",
          value = "123.45",
          unit = "kWh"
        ),
        SensorRecord(
          daqId = "daq:test:customer:meter:sensor2",
          `type` = "test",
          gatewayId = "gateway1",
          meterId = "meter1",
          timestamp = "invalid-timestamp",
          ingestedTime = isoFormatter.format(Instant.now()),
          sensorId = "sensor2",
          value = "not-a-number",
          unit = "kWh"
        )
      )
    }

    val data = Map("test" -> "data")
    val result = ProcessUtils.processDataToRecords("TEST", mockTransform, data, "v1")

    result should have size 1
    result.head.sensorId shouldBe "sensor1"
  }

  it should "return empty sequence on exception" in {
    def mockTransform(_data: Map[String, Any], _version: String): Seq[SensorRecord] = {
      throw new RuntimeException("Test exception")
    }

    val data = Map("test" -> "data")
    val result = ProcessUtils.processDataToRecords("TEST", mockTransform, data, "v1")

    result shouldBe empty
  }
}
