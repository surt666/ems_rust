package com.enity.flink.processors

import com.enity.flink.SensorRecord
import com.enity.flink.utils.Extensions.*
import org.slf4j.LoggerFactory

import java.nio.{ByteBuffer, ByteOrder}
import java.time.Instant
import java.util.Base64

object Mc603Processor:
  private val logger = LoggerFactory.getLogger(getClass)

  case class ParseResult(offset: Int, result: Map[String, Any])

  private case class ParserState(offset: Int, records: Vector[Map[String, Any]])

  private def parseHeatEnergy(payloadBytes: Array[Byte], offset: Int = 1): ParseResult =
    if offset + 2 > payloadBytes.length || (payloadBytes(offset) & 0xFF) != 0x04 then
      ParseResult(offset, Map.empty)
    else
      val formatType = payloadBytes(offset + 1) & 0xFF
      if formatType == 0xFB then
        if offset + 7 > payloadBytes.length then
          val newOffset = if offset + 3 <= payloadBytes.length then offset + 3 else payloadBytes.length
          ParseResult(newOffset, Map.empty)
        else
          val extendedType = payloadBytes(offset + 2) & 0xFF
          val energyValue = ByteBuffer.wrap(payloadBytes, offset + 3, 4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .getInt()
            .toLong & 0xFFFFFFFFL

          val fbFormats = Map(
            0x0D -> ("MCal", 1),
            0x0E -> ("MCal", 10),
            0x0F -> ("MCal", 100)
          )

          val result = fbFormats.get(extendedType) match
            case Some((unit, multiplier)) => Map[String, Any](
              "value" -> (energyValue * multiplier),
              "unit" -> unit,
              "sensorid" -> "heat_energy"
            )
            case None => Map.empty[String, Any]

          ParseResult(offset + 7, result)
      else if offset + 6 > payloadBytes.length then
        ParseResult(offset + 2, Map.empty)
      else
        val energyValue = ByteBuffer.wrap(payloadBytes, offset + 2, 4)
          .order(ByteOrder.LITTLE_ENDIAN)
          .getInt()
          .toLong & 0xFFFFFFFFL

        val formats = Map(
          0x00 -> ("Wh", 1.0 / 1000),
          0x01 -> ("Wh", 1.0 / 100),
          0x02 -> ("Wh", 1.0 / 10),
          0x03 -> ("Wh", 1.0),
          0x04 -> ("Wh", 10.0),
          0x05 -> ("Wh", 100.0),
          0x06 -> ("kWh", 1.0),
          0x07 -> ("kWh", 10.0),
          0x0E -> ("MJ", 1.0),
          0x0F -> ("MJ", 10.0)
        )

        val result = formats.get(formatType) match
          case Some((unit, factor)) => Map[String, Any](
            "value" -> (energyValue * factor),
            "unit" -> unit,
            "sensorid" -> "heat_energy"
          )
          case None => Map.empty[String, Any]

        ParseResult(offset + 6, result)

  private def parseVolume(payloadBytes: Array[Byte], offset: Int): ParseResult =
    if offset + 2 > payloadBytes.length || (payloadBytes(offset) & 0xFF) != 0x04 then
      ParseResult(offset, Map.empty)
    else
      val formatType = payloadBytes(offset + 1) & 0xFF
      if formatType >= 0x11 && formatType <= 0x17 then
        if offset + 6 > payloadBytes.length then
          ParseResult(offset + 2, Map.empty)
        else
          val volumeValue = ByteBuffer.wrap(payloadBytes, offset + 2, 4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .getInt()
            .toLong & 0xFFFFFFFFL

          val volumeFormats = Map(
            0x11 -> 0.00001,
            0x12 -> 0.0001,
            0x13 -> 0.001,
            0x14 -> 0.01,
            0x15 -> 0.1,
            0x16 -> 1.0,
            0x17 -> 10.0
          )

          val result = volumeFormats.get(formatType) match
            case Some(multiplier) => Map[String, Any](
              "value" -> (volumeValue * multiplier),
              "unit" -> "m^3",
              "sensorid" -> "volume"
            )
            case None => Map.empty[String, Any]

          ParseResult(offset + 6, result)
      else
        ParseResult(offset, Map.empty)

  private def parsePower(payloadBytes: Array[Byte], offset: Int): ParseResult =
    if offset + 2 > payloadBytes.length || (payloadBytes(offset) & 0xFF) != 0x02 then
      ParseResult(offset, Map.empty)
    else
      val formatType = payloadBytes(offset + 1) & 0xFF
      if formatType >= 0x2A && formatType <= 0x2F then
        if offset + 4 > payloadBytes.length then
          ParseResult(offset + 2, Map.empty)
        else
          val powerValue = ByteBuffer.wrap(payloadBytes, offset + 2, 2)
            .order(ByteOrder.LITTLE_ENDIAN)
            .getShort()
            .toInt & 0xFFFF

          val powerFormats = Map(
            0x2A -> ("W", 1),
            0x2B -> ("W", 1),
            0x2C -> ("W", 10),
            0x2D -> ("W", 100),
            0x2E -> ("kW", 1),
            0x2F -> ("kW", 10)
          )

          val result = powerFormats.get(formatType) match
            case Some((unit, multiplier)) => Map[String, Any](
              "value" -> (powerValue * multiplier),
              "unit" -> unit,
              "sensorid" -> "power"
            )
            case None => Map.empty[String, Any]

          ParseResult(offset + 4, result)
      else
        ParseResult(offset, Map.empty)

  private def parseFlow(payloadBytes: Array[Byte], offset: Int): ParseResult =
    if offset + 2 > payloadBytes.length || (payloadBytes(offset) & 0xFF) != 0x02 then
      ParseResult(offset, Map.empty)
    else
      val formatType = payloadBytes(offset + 1) & 0xFF
      if formatType >= 0x3B && formatType <= 0x3F then
        if offset + 4 > payloadBytes.length then
          ParseResult(offset + 2, Map.empty)
        else
          val flowValue = ByteBuffer.wrap(payloadBytes, offset + 2, 2)
            .order(ByteOrder.LITTLE_ENDIAN)
            .getShort()
            .toInt & 0xFFFF

          val flowFormats = Map(
            0x3B -> 0.001,
            0x3C -> 0.01,
            0x3D -> 0.1,
            0x3E -> 1.0,
            0x3F -> 10.0
          )

          val result = flowFormats.get(formatType) match
            case Some(multiplier) => Map[String, Any](
              "value" -> (flowValue * multiplier),
              "unit" -> "m^3/h",
              "sensorid" -> "flow"
            )
            case None => Map.empty[String, Any]

          ParseResult(offset + 4, result)
      else
        ParseResult(offset, Map.empty)

  private def parseForwardTemperature(payloadBytes: Array[Byte], offset: Int): ParseResult =
    if offset + 2 > payloadBytes.length || (payloadBytes(offset) & 0xFF) != 0x02 then
      ParseResult(offset, Map.empty)
    else
      val formatType = payloadBytes(offset + 1) & 0xFF
      if formatType >= 0x58 && formatType <= 0x5B then
        if offset + 4 > payloadBytes.length then
          ParseResult(offset + 2, Map.empty)
        else
          val tempValue = ByteBuffer.wrap(payloadBytes, offset + 2, 2)
            .order(ByteOrder.LITTLE_ENDIAN)
            .getShort()
            .toInt & 0xFFFF

          val tempFormats = Map(
            0x58 -> 0.001,
            0x59 -> 0.01,
            0x5A -> 0.1,
            0x5B -> 1.0
          )

          val result = tempFormats.get(formatType) match
            case Some(multiplier) => Map[String, Any](
              "value" -> (tempValue * multiplier),
              "unit" -> "C",
              "sensorid" -> "forward_temperature"
            )
            case None => Map.empty[String, Any]

          ParseResult(offset + 4, result)
      else
        ParseResult(offset, Map.empty)

  private def parseReturnTemperature(payloadBytes: Array[Byte], offset: Int): ParseResult =
    if offset + 2 > payloadBytes.length || (payloadBytes(offset) & 0xFF) != 0x02 then
      ParseResult(offset, Map.empty)
    else
      val formatType = payloadBytes(offset + 1) & 0xFF
      if formatType >= 0x5C && formatType <= 0x5F then
        if offset + 4 > payloadBytes.length then
          ParseResult(offset + 2, Map.empty)
        else
          val tempValue = ByteBuffer.wrap(payloadBytes, offset + 2, 2)
            .order(ByteOrder.LITTLE_ENDIAN)
            .getShort()
            .toInt & 0xFFFF

          val tempFormats = Map(
            0x5C -> 0.001,
            0x5D -> 0.01,
            0x5E -> 0.1,
            0x5F -> 1.0
          )

          val result = tempFormats.get(formatType) match
            case Some(multiplier) => Map[String, Any](
              "value" -> (tempValue * multiplier),
              "unit" -> "C",
              "sensorid" -> "return_temperature"
            )
            case None => Map.empty[String, Any]

          ParseResult(offset + 4, result)
      else
        ParseResult(offset, Map.empty)

  private def decodeMc603Payload(payload: String): Seq[Map[String, Any]] =
    val payloadBytes = Base64.getDecoder.decode(payload)

    if (payloadBytes(0) & 0xFF) != 0x15 then
      throw new IllegalArgumentException("Not a valid Kamstrup standard message format (0x15)")

    val parsers: Seq[(Array[Byte], Int) => ParseResult] = Seq(
      parseHeatEnergy, parseVolume, parsePower, parseFlow, parseForwardTemperature, parseReturnTemperature
    )

    val finalState = parsers.foldLeft(ParserState(1, Vector.empty)) { (state, parser) =>
      val result = parser(payloadBytes, state.offset)
      val newRecords = if result.result.nonEmpty then state.records :+ result.result else state.records
      ParserState(result.offset, newRecords)
    }

    finalState.records

  def transform(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"MC603 $data")

    val requiredFields = Seq("schematype", "customerid", "WirelessMetadata", "PayloadData")
    val missingFields = data.missingFields(requiredFields)

    if missingFields.nonEmpty then
      logger.info("MC603 Missing Fields")
      throw new IllegalArgumentException(s"Missing required fields: ${missingFields.mkString(", ")}")

    if !data.isString("PayloadData") then
      logger.info("MC603 Payload not string")
      throw new IllegalArgumentException("PayloadData must be a string")

    logger.info("MC603 decode")
    val payloads = decodeMc603Payload(data("PayloadData").toString)
    logger.debug(s"MC603 payloads $payloads")

    val schematype = data("schematype").toString
    val customerid = data("customerid").toString
    val wirelessMetadata = data.nestedMap("WirelessMetadata")
    val loRaWAN = wirelessMetadata.nestedMap("LoRaWAN")
    val devEui = loRaWAN("DevEui").toString
    val ts = loRaWAN("Timestamp").toString

    val timestamp = parseTimestamp(ts)
    val formattedTimestamp = isoFormatter.format(timestamp)

    val ingestedTime = isoFormatter.format(Instant.now())

    payloads.map { payload =>
      val sensorId = payload("sensorid").toString
      val daqId = buildDaqId(schematype, customerid, devEui, sensorId)

      SensorRecord(
        daqId = daqId,
        `type` = schematype,
        gatewayId = customerid,
        meterId = devEui,
        timestamp = formattedTimestamp,
        ingestedTime = ingestedTime,
        sensorId = sensorId,
        value = payload("value").toString,
        unit = payload("unit").toString
      )
    }
