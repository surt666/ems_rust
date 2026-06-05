package com.enity.flink.processors

import com.enity.flink.SensorRecord
import com.enity.flink.utils.Extensions.*
import org.slf4j.LoggerFactory

import java.nio.{ByteBuffer, ByteOrder}
import java.time.Instant
import java.util.Base64
import scala.annotation.tailrec

object EmuProcessor:
  private val logger = LoggerFactory.getLogger(getClass)

  case class DataType(len: Int, description: String, dataType: String, unit: Option[String] = None)

  private val dataTypes: Map[Int, DataType] = Map(
    0x00 -> DataType(4, "data-logger-index", "Uint32"),
    0x01 -> DataType(4, "timestamp", "Uint32", Some("seconds")),
    0x03 -> DataType(4, "Active Energy Import T1", "Uint32", Some("Wh")),
    0x04 -> DataType(4, "Active Energy Import T2", "Uint32", Some("Wh")),
    0x05 -> DataType(4, "Active Energy Export T1", "Uint32", Some("Wh")),
    0x06 -> DataType(4, "Active Energy Export T2", "Uint32", Some("Wh")),
    0xF0 -> DataType(1, "errorcode", "ErrorCode")
  )

  private def decodeValue(data: Array[Byte], offset: Int, dataType: DataType): Long =
    dataType.dataType match
      case "Uint32" =>
        val buffer = ByteBuffer.wrap(data, offset, 4).order(ByteOrder.LITTLE_ENDIAN)
        buffer.getInt().toLong & 0xFFFFFFFFL
      case "ErrorCode" =>
        data(offset).toLong & 0xFF
      case _ => 0L

  private def decodeEmuPayload(payload: String): Seq[Map[String, Any]] =
    val decoded = Base64.getDecoder.decode(payload)

    if decoded.length < 4 then
      throw new IllegalArgumentException(s"Payload too short: ${decoded.length} bytes")

    logger.debug(s"Successfully decoded base64 payload, length: ${decoded.length} bytes")

    val buffer = ByteBuffer.wrap(decoded, 0, 4).order(ByteOrder.LITTLE_ENDIAN)
    val timestamp = buffer.getInt().toLong & 0xFFFFFFFFL

    @tailrec
    def loop(pos: Int, acc: Vector[Map[String, Any]]): Vector[Map[String, Any]] =
      if pos >= decoded.length - 1 then acc
      else
        val typeByte = decoded(pos) & 0xFF
        dataTypes.get(typeByte) match
          case Some(dt) =>
            val dataPos = pos + 1
            val value = decodeValue(decoded, dataPos, dt)
            val timestampInstant = Instant.ofEpochSecond(timestamp)
            val formattedTimestamp = isoFormatter.format(timestampInstant)
            val item = Map[String, Any](
              "type" -> dt.description,
              "sensorid" -> f"0x$typeByte%x",
              "value" -> value,
              "unit" -> dt.unit.getOrElse(""),
              "timestamp" -> formattedTimestamp
            )
            val nextAcc = if item("type") != "errorcode" then acc :+ item else acc
            loop(dataPos + dt.len, nextAcc)
          case None =>
            loop(pos + 1, acc)

    loop(4, Vector.empty)

  def transform(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"EMU $data")

    val requiredFields = Seq("schematype", "customerid", "WirelessMetadata", "PayloadData")
    val missing = data.missingFields(requiredFields)

    if missing.nonEmpty then
      logger.info("EMU Missing Fields")
      throw new IllegalArgumentException(s"Missing required fields: ${missing.mkString(", ")}")

    data("PayloadData") match
      case payloadStr: String =>
        logger.info("EMU decode")
        val payloads = decodeEmuPayload(payloadStr)
        logger.debug(s"EMU payloads $payloads")

        val schematype = data("schematype").toString
        val customerid = data("customerid").toString
        val loRaWAN = data.nestedMap("WirelessMetadata").nestedMap("LoRaWAN")
        val devEui = loRaWAN("DevEui").toString
        val ingestedTime = isoFormatter.format(Instant.now())

        payloads.map { payload =>
          val sensorId = payload("sensorid").toString
          val daqId = buildDaqId(schematype, customerid, devEui, sensorId)

          SensorRecord(
            daqId = daqId,
            `type` = schematype,
            gatewayId = customerid,
            meterId = devEui,
            timestamp = payload("timestamp").toString,
            ingestedTime = ingestedTime,
            sensorId = sensorId,
            value = payload("value").toString,
            unit = payload("unit").toString
          )
        }
      case _ =>
        logger.info("EMU Payload not string")
        throw new IllegalArgumentException("PayloadData must be a string")
