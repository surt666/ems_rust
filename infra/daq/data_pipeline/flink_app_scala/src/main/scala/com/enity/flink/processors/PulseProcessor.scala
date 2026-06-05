package com.enity.flink.processors

import com.enity.flink.SensorRecord
import com.enity.flink.utils.Extensions.*
import org.slf4j.LoggerFactory

import java.nio.{ByteBuffer, ByteOrder}
import java.time.{Instant, ZoneOffset, ZonedDateTime}
import java.time.format.DateTimeFormatter
import java.util.Base64

object PulseProcessor:
  private val logger = LoggerFactory.getLogger(getClass)

  private def decodePulsePayload(payload: String): Map[String, Any] =
    try
      val decoded = Base64.getDecoder.decode(payload)

      if decoded.length < 10 then
        Map("error" -> "Payload too short for counter frame")
      else
        val frameCode = decoded(0) & 0xFF

        if frameCode != 0x46 then
          Map("error" -> f"Not a counter frame. Frame code: 0x$frameCode%02x")
        else
          val status = decoded(1) & 0xFF
          val counterA = ByteBuffer.wrap(decoded, 2, 4).order(ByteOrder.BIG_ENDIAN).getInt().toLong & 0xFFFFFFFFL
          val counterB = ByteBuffer.wrap(decoded, 6, 4).order(ByteOrder.BIG_ENDIAN).getInt().toLong & 0xFFFFFFFFL

          val base = Map[String, Any](
            "frame_code" -> "0x46",
            "status" -> status,
            "counter_a" -> counterA,
            "counter_b" -> counterB
          )

          if decoded.length >= 14 then
            val timestampRaw = ByteBuffer.wrap(decoded, 10, 4).order(ByteOrder.BIG_ENDIAN).getInt().toLong & 0xFFFFFFFFL
            val epoch2013 = ZonedDateTime.of(2013, 1, 1, 0, 0, 0, 0, ZoneOffset.UTC)
            val timestamp = epoch2013.plusSeconds(timestampRaw)
            base ++ Map(
              "timestamp" -> timestamp.format(DateTimeFormatter.ISO_INSTANT),
              "timestamp_raw" -> timestampRaw
            )
          else
            base
    catch
      case e: Exception =>
        Map("error" -> e.getMessage, "raw_payload" -> payload)

  def transform(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"PULSE $data")

    val requiredFields = Seq("schematype", "customerid", "WirelessMetadata", "PayloadData")
    val missing = data.missingFields(requiredFields)

    if missing.nonEmpty then
      logger.info("PULSE Missing Fields")
      throw new IllegalArgumentException(s"Missing required fields: ${missing.mkString(", ")}")

    data("PayloadData") match
      case payloadStr: String =>
        logger.info("PULSE decode")
        val payload = decodePulsePayload(payloadStr)
        logger.debug(s"PULSE payloads $payload")

        val schematype = data("schematype").toString
        val customerid = data("customerid").toString
        val loRaWAN = data.nestedMap("WirelessMetadata").nestedMap("LoRaWAN")
        val devEui = loRaWAN("DevEui").toString
        val ts = loRaWAN("Timestamp").toString

        val timestamp = parseTimestamp(ts)
        val formattedTimestamp = isoFormatter.format(timestamp)

        val ingestedTime = isoFormatter.format(Instant.now())

        Seq(
          SensorRecord(
            daqId = buildDaqId(schematype, customerid, devEui, "counter_a"),
            `type` = schematype,
            gatewayId = customerid,
            meterId = devEui,
            timestamp = formattedTimestamp,
            ingestedTime = ingestedTime,
            sensorId = "counter_a",
            value = payload("counter_a").toString,
            unit = "NA"
          ),
          SensorRecord(
            daqId = buildDaqId(schematype, customerid, devEui, "counter_b"),
            `type` = schematype,
            gatewayId = customerid,
            meterId = devEui,
            timestamp = formattedTimestamp,
            ingestedTime = ingestedTime,
            sensorId = "counter_b",
            value = payload("counter_b").toString,
            unit = "NA"
          )
        )
      case _ =>
        logger.info("PULSE Payload not string")
        throw new IllegalArgumentException("PayloadData must be a string")
