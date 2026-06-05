package com.enity.flink.processors

import com.enity.flink.SensorRecord
import com.enity.flink.utils.Extensions.*
import org.slf4j.LoggerFactory

import java.time.Instant
import scala.util.{Try, Failure}

object MivoProcessor:
  private val logger = LoggerFactory.getLogger(getClass)

  def transform(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"MIVO $data")

    val requiredFields = Seq("schematype", "customerid", "Readings")
    val missing = data.missingFields(requiredFields)

    if missing.nonEmpty then
      logger.info("MIVO Missing Fields")
      throw new IllegalArgumentException(s"Missing required fields: ${missing.mkString(", ")}")

    val schematype = data("schematype").toString
    val unitSerial = data("UnitSerial").toString
    val readings = data.seqOfMaps("Readings")
    val ingestedTime = isoFormatter.format(Instant.now())

    readings.flatMap { reading =>
      Try {
        val meterNumber = reading("MeterNumber").toString

        val timestamp = parseTimestamp(reading("Time").toString)
        val formattedTimestamp = isoFormatter.format(timestamp)

        val values = reading.seqOfMaps("Values")

        values.map { value =>
          val code = value("Code").toString
          val daqId = buildDaqId(schematype, unitSerial, meterNumber, code)

          SensorRecord(
            daqId = daqId,
            `type` = schematype,
            gatewayId = unitSerial,
            meterId = meterNumber,
            timestamp = formattedTimestamp,
            ingestedTime = ingestedTime,
            sensorId = code,
            value = value("Value").toString,
            unit = value("Unit").toString
          )
        }
      } match
        case scala.util.Success(records) => records
        case Failure(e) =>
          logger.error(s"MIVO Skipping record due to missing key: ${e.getMessage}")
          Seq.empty
    }
