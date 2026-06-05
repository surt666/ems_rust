package com.enity.flink.utils

import com.enity.flink.SensorRecord
import org.slf4j.LoggerFactory

import scala.util.Try

object ProcessUtils:
  private val logger = LoggerFactory.getLogger(getClass)

  def filterValid(records: Seq[SensorRecord]): Seq[SensorRecord] =
    records.filter(isValidRecord)

  private def isValidRecord(record: SensorRecord): Boolean =
    Option(record).exists { r =>
      val valueValid = Try(r.value.toDouble).isSuccess
      val daqIdValid = Option(r.daqId).exists(_.nonEmpty)
      val timestampValid = Try(Extensions.parseTimestamp(r.timestamp)).isSuccess
      valueValid && daqIdValid && timestampValid
    }

  def processDataToRecords(
    typeShort: String,
    transformFunc: (Map[String, Any], String) => Seq[SensorRecord],
    data: Map[String, Any],
    version: String
  ): Seq[SensorRecord] =
    logger.debug(s"Processing $typeShort")
    try
      val transformedRecords = filterValid(transformFunc(data, version))
      logger.debug(s"Processed $typeShort: ${transformedRecords.size} records")
      transformedRecords
    catch
      case e: Exception =>
        logger.error(s"Error processing $typeShort", e)
        Seq.empty
