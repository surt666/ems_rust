package com.enity.flink.processors

import com.enity.flink.SensorRecord
import com.enity.flink.utils.Extensions.*
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.scala.DefaultScalaModule
import org.slf4j.LoggerFactory

import java.time.Instant

object StdProcessor:
  private val logger = LoggerFactory.getLogger(getClass)
  private val mapper = new ObjectMapper()
  mapper.registerModule(DefaultScalaModule)

  /**
   * Transform STD JSONL data to uniform format
   */
  def transformStdJsonl(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"STD JSONL $data")

    val requiredFields = Seq("schematype", "customerid", "data")
    val missing = data.missingFields(requiredFields)

    if missing.nonEmpty then
      logger.info("STD JSONL Missing Fields")
      throw new IllegalArgumentException(s"Missing required fields: ${missing.mkString(", ")}")

    val schematype = data("schematype").toString
    val customerid = data("customerid").toString

    // Handle data field - it can be either a List or a String
    val jsonData: Seq[Map[String, Any]] = data("data") match
      case dataList: Seq[?] =>
        dataList.asInstanceOf[Seq[Map[String, Any]]]
      case dataString: String =>
        val cleanedData = dataString.replace("\\n", "\n")
        cleanedData.split("\n").filter(_.trim.nonEmpty).map { line =>
          mapper.readValue(line, classOf[Map[String, Any]])
        }.toSeq
      case _ =>
        throw new IllegalArgumentException("Data field must be a List or String")

    jsonData.map { payload =>
      val meterId = payload("meterid").toString
      val sensorId = payload("sensorid").toString
      val daqId = buildDaqId(schematype, customerid, meterId, sensorId)

      val timestamp = parseTimestamp(payload("timestamp").toString)
      val formattedTimestamp = isoFormatter.format(timestamp)

      val ingestedTime = isoFormatter.format(Instant.now())

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

  /**
   * Transform STD JSON data to uniform format
   */
  def transformStdJson(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"STD JSON $data")

    val requiredFields = Seq("schematype", "customerid", "data")
    val missing = data.missingFields(requiredFields)

    if missing.nonEmpty then
      logger.info("STD JSON Missing Fields")
      throw new IllegalArgumentException(s"Missing required fields: ${missing.mkString(", ")}")

    val schematype = data("schematype").toString
    val customerid = data("customerid").toString
    val dataList = data.seqOfMaps("data")

    dataList.map { payload =>
      val meterId = payload("meterid").toString
      val sensorId = payload("sensorid").toString

      // Determine gatewayId with fallback logic
      val gatewayId = payload.get("gatewayid") match
        case Some(gid) =>
          val gidStr = gid.toString
          if gidStr != "NONE" && gidStr != "None" then gidStr else customerid
        case None => customerid

      val daqId = buildDaqId(schematype, customerid, meterId, sensorId)

      val timestamp = parseTimestamp(payload("timestamp").toString)
      val formattedTimestamp = isoFormatter.format(timestamp)

      val ingestedTime = isoFormatter.format(Instant.now())

      SensorRecord(
        daqId = daqId,
        `type` = schematype,
        gatewayId = gatewayId,
        meterId = meterId,
        timestamp = formattedTimestamp,
        ingestedTime = ingestedTime,
        sensorId = sensorId,
        value = payload("value").toString,
        unit = payload("unit").toString
      )
    }
