package com.enity.flink.processors

import com.enity.flink.SensorRecord
import com.enity.flink.utils.Extensions.*
import org.slf4j.LoggerFactory

import java.time.Instant

object BluemeteringProcessor:
  private val logger = LoggerFactory.getLogger(getClass)

  /**
   * Transform Bluemetering data to uniform format
   */
  def transform(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"BLUE $data")

    val requiredFields = Seq(
      "schematype",
      "customerid",
      "dateTime",
      "obisIndex",
      "dataSource",
      "value"
    )
    val missing = data.missingFields(requiredFields)

    if missing.nonEmpty then
      logger.info("Bluemetering Missing Fields")
      throw new IllegalArgumentException(s"Missing required fields: ${missing.mkString(", ")}")

    val schematype = data("schematype").toString
    val customerid = data("customerid").toString
    val meterSerialNumber = data("meterSerialNumber").toString
    val obisIndex = data("obisIndex").toString
    val dataSource = data("dataSource").toString

    val daqId = buildDaqId(schematype, customerid, meterSerialNumber, obisIndex)

    val timestamp = parseTimestamp(data("dateTime").toString)
    val formattedTimestamp = isoFormatter.format(timestamp)

    val ingestedTime = isoFormatter.format(Instant.now())

    Seq(SensorRecord(
      daqId = daqId,
      `type` = schematype,
      gatewayId = dataSource,
      meterId = meterSerialNumber,
      timestamp = formattedTimestamp,
      ingestedTime = ingestedTime,
      sensorId = obisIndex,
      value = data("value").toString,
      unit = data("unit").toString
    ))
