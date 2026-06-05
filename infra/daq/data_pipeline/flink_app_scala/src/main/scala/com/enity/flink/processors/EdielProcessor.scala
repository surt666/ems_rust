package com.enity.flink.processors

import com.enity.flink.SensorRecord
import com.enity.flink.utils.Extensions.*
import org.slf4j.LoggerFactory

import java.time.Instant

object EdielProcessor:
  private val logger = LoggerFactory.getLogger(getClass)

  def transformJsonV1(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"EDIEL JSON $data")

    // Check if this is the standard format with schematype and data array
    if data.contains("schematype") && data.contains("data") then
      val requiredFields = Seq("schematype", "customerid", "data")
      val missing = data.missingFields(requiredFields)

      if missing.nonEmpty then
        logger.info("EDIEL JSON Missing Fields (standard format)")
        throw new IllegalArgumentException(s"Missing required fields: ${missing.mkString(", ")}")

      data("data") match
        case _: Seq[?] =>
          val schematype = data("schematype").toString
          val customerid = data("customerid").toString
          val dataList = data.seqOfMaps("data")

          val ingestedTime = isoFormatter.format(Instant.now())

          dataList.map { payload =>
            val meterId = payload("meterid").toString
            val sensorId = payload("sensorid").toString
            val daqId = buildDaqId(schematype, customerid, meterId, sensorId)

            // Handle timestamp with space separator (e.g., "2025-06-30 23:00:00")
            val timestampRaw = payload("timestamp").toString
            val timestampStr =
              if timestampRaw.contains("T") then
                // Already has 'T', just ensure Z is added if not present
                if timestampRaw.endsWith("Z") then timestampRaw else timestampRaw + "Z"
              else
                // Has space instead of 'T', replace and add Z
                timestampRaw.replace(" ", "T") + "Z"
            val timestamp = Instant.parse(timestampStr)
            val formattedTimestamp = isoFormatter.format(timestamp)

            SensorRecord(
              daqId = daqId,
              `type` = schematype,
              gatewayId = customerid,
              meterId = meterId,
              timestamp = formattedTimestamp,
              ingestedTime = ingestedTime,
              sensorId = sensorId,
              value = payload("value").toString,
              unit = payload("unit").toString
            )
          }
        case _ =>
          logger.info("EDIEL JSON data field is not a list")
          Seq.empty
    else
      Seq.empty
