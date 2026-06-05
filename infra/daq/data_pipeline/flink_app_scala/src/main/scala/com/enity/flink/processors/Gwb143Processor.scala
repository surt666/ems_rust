package com.enity.flink.processors

import com.enity.flink.SensorRecord
import com.enity.flink.utils.Extensions.*
import org.slf4j.LoggerFactory

import java.time.Instant
import scala.util.{Try, Success, Failure}

object Gwb143Processor:
  private val logger = LoggerFactory.getLogger(getClass)

  private def createDefaultRecord(): Map[String, Any] =
    Map(
      "trbSerial" -> "unknown",
      "id" -> "unknown",
      "dataid" -> "unknown",
      "unit" -> "unknown",
      "value" -> "0.0",
      "timestamp" -> "1970-01-01 00:00:00"
    )

  private def flattenGwb143Data(data: Map[String, Any]): Seq[Map[String, Any]] =
    try
      if !data.isSeq("data") then
        logger.error(s"GWB143 data is not list: $data")
        Seq(createDefaultRecord())
      else
        val dataList = data.seqOfMaps("data")
        val flattened = dataList.flatMap { dataItem =>
          val dataid = dataItem.getOrElse("id", "Unknown").toString
          val unit = dataItem.getOrElse("Unit", "Unknown").toString
          val value = dataItem.getOrElse("Value", "0").toString

          val timestampStr = dataItem.get("Timestamp").map(_.toString).getOrElse("")
          val timestamp = Try {
            isoFormatter.format(parseTimestamp(timestampStr))
          }.getOrElse("1970-01-01T00:00:00.000000Z")

          val transformed = Map(
            "dataid" -> dataid,
            "unit" -> unit,
            "value" -> value,
            "timestamp" -> timestamp
          )

          if !unit.startsWith("dimensionless") && !unit.startsWith("Reserved") then
            Try(value.toDouble) match
              case Success(_) => Some(transformed)
              case Failure(_) =>
                logger.debug(s"Skipping record with non-numeric value: $value (unit: $unit)")
                None
          else
            None
        }

        if flattened.isEmpty then Seq(createDefaultRecord()) else flattened
    catch
      case e: Exception =>
        logger.error(s"Error flattening GWB143 data: ${e.getMessage}")
        Seq(createDefaultRecord())

  def transform(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"GWB143 $data")

    val requiredFields = Seq("trbSerial", "data", "customerid", "schematype")
    val missing = data.missingFields(requiredFields)

    if missing.nonEmpty then
      logger.info("GWB143 Missing Fields")
      throw new IllegalArgumentException(s"Missing required fields: ${missing.mkString(", ")}")

    val schematype = data("schematype").toString
    val ingestedTime = isoFormatter.format(Instant.now())

    if !data.isSeq("data") then
      logger.info("GWB143 data field is not a list")
      val trbSerial = data.getOrElse("trbSerial", "unknown").toString
      val id = data.getOrElse("Id", "unknown").toString
      Seq(SensorRecord(
        daqId = buildDaqId(schematype, trbSerial, id, "unknown"),
        `type` = schematype,
        gatewayId = trbSerial,
        meterId = id,
        timestamp = "1970-01-01T00:00:00.000000Z",
        ingestedTime = ingestedTime,
        sensorId = "unknown",
        value = "0.0",
        unit = "unknown"
      ))
    else
      logger.info("GWB143 flatten")
      val flattenedData = flattenGwb143Data(data)
      logger.debug(s"GWB143 flattened data $flattenedData")

      val trbSerial = data("trbSerial").toString
      val id = data("Id").toString

      flattenedData.map { payload =>
        val dataid = payload("dataid").toString
        val daqId = buildDaqId(schematype, trbSerial, id, dataid)

        SensorRecord(
          daqId = daqId,
          `type` = schematype,
          gatewayId = trbSerial,
          meterId = id,
          timestamp = payload("timestamp").toString,
          ingestedTime = ingestedTime,
          sensorId = dataid,
          value = payload("value").toString,
          unit = payload("unit").toString
        )
      }
