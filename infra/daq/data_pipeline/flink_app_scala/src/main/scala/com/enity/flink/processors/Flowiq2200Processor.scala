package com.enity.flink.processors

import com.enity.flink.SensorRecord
import com.enity.flink.utils.Extensions.*
import org.slf4j.LoggerFactory

import java.time.{Instant, ZoneOffset, ZonedDateTime}
import java.time.format.DateTimeFormatter
import java.util.Base64
import scala.annotation.tailrec
import scala.util.boundary, boundary.break

object Flowiq2200Processor:
  private val logger = LoggerFactory.getLogger(getClass)

  case class VifEntry(
    `type`: String,
    unit: String,
    resolution: Double,
    conversionType: String
  )

  case class VibObject(
    `type`: String,
    unit: String,
    resolution: Double,
    conversionType: String,
    orthoVife: String,
    isProfileData: Boolean
  )

  case class DibObject(
    datafield: Int,
    functionfield: Int,
    storagenumber: Int
  )

  case class MbusRecord(
    dib: DibObject,
    vib: Option[VibObject],
    data: Option[Any],
    profileData: Option[ProfileData]
  )

  case class ProfileData(
    spacingValue: Int,
    spacingUnit: Int,
    incMode: Int,
    profileValues: Seq[Option[Long]]
  )

  case class DecodeResult(
    data: Map[String, Any],
    errors: Seq[String],
    warnings: Seq[String]
  )

  // Primary VIF table according to Table 10 in 13757-3:2018
  private val priVifTable: Map[Int, VifEntry] = Map(
    0x10 -> VifEntry("Volume", "m3", 1e-6, "B"),
    0x11 -> VifEntry("Volume", "m3", 1e-5, "B"),
    0x12 -> VifEntry("Volume", "m3", 1e-4, "B"),
    0x13 -> VifEntry("Volume", "m3", 1e-3, "B"),
    0x14 -> VifEntry("Volume", "m3", 1e-2, "B"),
    0x15 -> VifEntry("Volume", "m3", 1e-1, "B"),
    0x16 -> VifEntry("Volume", "m3", 1e0, "B"),
    0x17 -> VifEntry("Volume", "m3", 1e1, "B"),
    0x38 -> VifEntry("Volume flow", "m3/h", 1e-6, "B"),
    0x39 -> VifEntry("Volume flow", "m3/h", 1e-5, "B"),
    0x3A -> VifEntry("Volume flow", "m3/h", 1e-4, "B"),
    0x3B -> VifEntry("Volume flow", "m3/h", 1e-3, "B"),
    0x3C -> VifEntry("Volume flow", "m3/h", 1e-2, "B"),
    0x3D -> VifEntry("Volume flow", "m3/h", 1e-1, "B"),
    0x3E -> VifEntry("Volume flow", "m3/h", 1e0, "B"),
    0x3F -> VifEntry("Volume flow", "m3/h", 1e1, "B"),
    0x58 -> VifEntry("Flow temperature", "C", 1e-3, "B"),
    0x59 -> VifEntry("Flow temperature", "C", 1e-2, "B"),
    0x5A -> VifEntry("Flow temperature", "C", 1e-1, "B"),
    0x5B -> VifEntry("Flow temperature", "C", 1e0, "B"),
    0x64 -> VifEntry("External temperature", "C", 1e-3, "B"),
    0x65 -> VifEntry("External temperature", "C", 1e-2, "B"),
    0x66 -> VifEntry("External temperature", "C", 1e-1, "B"),
    0x67 -> VifEntry("External temperature", "C", 1e0, "B"),
    0x6C -> VifEntry("Date/time", "NA", 1, "G"),
    0x6D -> VifEntry("Date/time", "NA", 1, "F/J/I/M")
  )

  // Manufacture specific VIFE
  private val manuVifeTable: Map[Int, VifEntry] = Map(
    0x25 -> VifEntry("Infocode", "NA", 1, "D"),
    0x1C -> VifEntry("ALD last day", "NA", 1, "C"),
    0x16 -> VifEntry("Module type/config number", "NA", 1, "C"),
    0x1B -> VifEntry("ALD", "NA", 1, "C")
  )

  // Orthogonal VIFE table according to Table 15 in 13757-3:2018
  private val orthoVifeTable: Map[Int, String] = Map(
    0x13 -> "Inverse Compact Profile",
    0x3C -> "Reverse"
  )

  private val infocodeTable: Map[Int, String] = Map(
    0 -> "Dry",
    1 -> "Reverse",
    2 -> "Leak",
    3 -> "Burst",
    4 -> "Tamper",
    5 -> "Low Battery",
    6 -> "Low Ambient Temperature",
    7 -> "High Ambient Temperature"
  )

  private def parseVib(vibArray: Array[Int]): Option[VibObject] =
    if vibArray(0) == 0xFF then
      manuVifeTable.get(vibArray(1)).map { tempObj =>
        VibObject(
          `type` = tempObj.`type`,
          unit = tempObj.unit,
          resolution = tempObj.resolution,
          conversionType = tempObj.conversionType,
          orthoVife = "NA",
          isProfileData = false
        )
      }
    else
      priVifTable.get(vibArray(0) & 0x7F).flatMap { tempObj =>
        val hasExtension = (vibArray(0) & 0x80) != 0
        val orthoVifeOpt = if hasExtension then
          orthoVifeTable.get(vibArray(1))
        else
          Some("NA")

        orthoVifeOpt.map { orthoVife =>
          val isProfile = orthoVife == "Inverse Compact Profile"
          VibObject(
            `type` = tempObj.`type`,
            unit = tempObj.unit,
            resolution = tempObj.resolution,
            conversionType = tempObj.conversionType,
            orthoVife = orthoVife,
            isProfileData = isProfile
          )
        }
      }

  private def readUintLe(buffer: Array[Byte], offset: Int, byteLength: Int): Long =
    (0 until byteLength).foldLeft(0L) { (acc, i) =>
      acc | ((buffer(offset + i) & 0xFF).toLong << (8 * i))
    }

  private def readIntLe(buffer: Array[Byte], offset: Int, byteLength: Int): Long =
    val value = readUintLe(buffer, offset, byteLength)
    val maxVal = 1L << (8 * byteLength - 1)
    if (value & maxVal) != 0 then value - (1L << (8 * byteLength)) else value

  private def typeA(buffer: Array[Byte], idx: Int, size: Int): Option[Long] =
    boundary:
      val result = (idx until (idx + size)).foldLeft(0L -> 1L) { case ((acc, multiplier), j) =>
        val lsb = buffer(j) & 0xF
        val msb = (buffer(j) >> 4) & 0xF
        if lsb > 9 || msb > 9 then
          break(None)
        (acc + lsb * multiplier + msb * multiplier * 10, multiplier * 100)
      }
      Some(result._1)

  private def typeB(buffer: Array[Byte], idx: Int, size: Int): Option[Long] =
    val invalidValues = Map(
      1 -> -0x80L,
      2 -> -0x8000L,
      3 -> -0x800000L,
      4 -> -0x80000000L,
      6 -> -0x800000000000L
    )
    val result = readIntLe(buffer, idx, size)
    if invalidValues.get(size).contains(result) then None else Some(result)

  private def typeC(buffer: Array[Byte], idx: Int, size: Int): Option[Long] =
    val invalidValues = Map(
      1 -> 0xFFL,
      2 -> 0xFFFFL,
      3 -> 0xFFFFFFL,
      4 -> 0xFFFFFFFFL,
      6 -> 0xFFFFFFFFFFFFL
    )
    val result = readUintLe(buffer, idx, size)
    if invalidValues.get(size).contains(result) then None else Some(result)

  private def typeD(buffer: Array[Byte], idx: Int, size: Int): Long =
    readUintLe(buffer, idx, size)

  private def typeF(buffer: Array[Byte], idx: Int, size: Int): Option[ZonedDateTime] =
    val data = readUintLe(buffer, idx, size)

    // Check invalid flag (bit 7 of byte 1)
    if (data & 0x00000080L) != 0 then None
    else
      // Extract fields according to the correct bit layout
      val minutes = (data & 0x3F).toInt
      val hours = ((data >> 8) & 0x1F).toInt
      val centuryBits = ((data >> 13) & 0x3).toInt
      val days = ((data >> 16) & 0x1F).toInt
      val yearLow = ((data >> 21) & 0x7).toInt
      val months = ((data >> 24) & 0xF).toInt
      val yearHigh = ((data >> 28) & 0xF).toInt

      // Combine year bits
      val yearWithinCentury = (yearHigh << 3) | yearLow

      // Calculate full year based on century
      val centuryBase = Array(1900, 2000, 2100, 2200)(centuryBits)
      val fullYear = centuryBase + yearWithinCentury

      try
        Some(ZonedDateTime.of(fullYear, months, days, hours, minutes, 0, 0, ZoneOffset.UTC))
      catch
        case _: Exception => None

  private def typeG(buffer: Array[Byte], idx: Int, size: Int): Option[ZonedDateTime] =
    val data = readUintLe(buffer, idx, size)
    if data == 0xFFFFL then None
    else
      val days = ((data >> 0) & 0x001F).toInt
      val months = ((data >> 8) & 0x000F).toInt
      val years = 2000 + (((data >> 5) & 0x0007) + (((data >> 12) & 0x000F) << 3)).toInt

      try
        Some(ZonedDateTime.of(years, months, days, 0, 0, 0, 0, ZoneOffset.UTC))
      catch
        case _: Exception => None

  private def typeI(buffer: Array[Byte], idx: Int): Option[ZonedDateTime] =
    if (buffer(idx + 1) & 0x80) != 0 then None
    else
      val seconds = buffer(idx) & 0x3F
      val minutes = buffer(idx + 1) & 0x3F
      val hours = buffer(idx + 2) & 0x1F
      val days = buffer(idx + 3) & 0x1F
      val months = buffer(idx + 4) & 0xF
      val years = 2000 + (((buffer(idx + 3) >> 5) & 0x7) + (((buffer(idx + 4) >> 4) & 0xF) << 3))

      try
        Some(ZonedDateTime.of(years, months, days, hours, minutes, seconds, 0, ZoneOffset.UTC))
      catch
        case _: Exception => None

  private def inverseCompactProfile(buffer: Array[Byte], idx: Int, size: Int): Option[ProfileData] =
    val spacingControl = buffer(idx) & 0xFF
    val spacingValue = buffer(idx + 1) & 0xFF

    val elementSize = spacingControl & 0x0F
    val spacingUnit = (spacingControl >> 4) & 0x03
    val incMode = (spacingControl >> 6) & 0x03

    if elementSize < 1 || elementSize > 4 || incMode == 0 then None
    else
      @tailrec
      def readValues(currentIdx: Int, remaining: Int, acc: Vector[Option[Long]]): Vector[Option[Long]] =
        if remaining <= 0 then acc
        else
          val value = if incMode == 0x3 then // Signed difference - TypeB
            typeB(buffer, currentIdx, elementSize)
          else // Unsigned increments or decrements - TypeC
            typeC(buffer, currentIdx, elementSize)
          readValues(currentIdx + elementSize, remaining - elementSize, acc :+ value)

      val profileValues = readValues(idx + 2, size - 2, Vector.empty)
      Some(ProfileData(spacingValue, spacingUnit, incMode, profileValues))

  private def getNextTimestamp(timestamp: ZonedDateTime, spacingUnit: Int, spacingValue: Int): Option[ZonedDateTime] =
    if spacingValue > 0 && spacingValue < 251 then
      spacingUnit match
        case 0 => Some(timestamp.minusSeconds(spacingValue))
        case 1 => Some(timestamp.minusMinutes(spacingValue))
        case 2 => Some(timestamp.minusHours(spacingValue))
        case 3 => Some(timestamp.minusDays(spacingValue))
        case _ => None
    else if spacingValue == 254 && spacingUnit == 3 then
      Some(timestamp.minusMonths(1))
    else if spacingValue == 254 && spacingUnit == 2 then
      Some(timestamp.minusMonths(3))
    else if spacingValue == 254 && spacingUnit == 1 then
      Some(timestamp.minusMonths(6))
    else
      None

  private def normalize(number: Long, resolution: Double): Double =
    Math.round(number * resolution * 1e10) / 1e10

  private def normalize(number: Double, resolution: Double): Double =
    Math.round(number * resolution * 1e10) / 1e10

  // State threaded through the decode parsing loop
  private case class DecoderState(
    pos: Int,
    records: Vector[MbusRecord],
    errors: Vector[String],
    warnings: Vector[String]
  )

  private def decode(payload: String): DecodeResult =
    val raw = Base64.getDecoder.decode(payload)

    boundary:
      // TPL according to EN 13757-7
      if raw.length < 1 then
        break(DecodeResult(Map.empty, Seq("Invalid uplink payload: Could not retrieve CI field"), Seq.empty))

      val ci = raw(0) & 0xFF

      val (headerPos, configfield) =
        if ci == 0x7A then // Short data header
          if raw.length < 5 then
            break(DecodeResult(Map.empty, Seq("Invalid uplink payload: Could not retrieve TPL layer"), Seq.empty))
          val cf = (raw(3) & 0xFF) | ((raw(4) & 0xFF) << 8)
          (5, cf)
        else if ci == 0x72 then // Long data header
          if raw.length < 13 then
            break(DecodeResult(Map.empty, Seq("Invalid uplink payload: Could not retrieve TPL layer"), Seq.empty))
          val cf = (raw(11) & 0xFF) | ((raw(12) & 0xFF) << 8)
          (13, cf)
        else if ci == 0x78 then // No data header
          (1, 0)
        else // Unsupported header
          break(DecodeResult(Map.empty, Seq("Invalid uplink payload: Invalid CI in TPL layer"), Seq.empty))

      if (configfield & 0x1F00) != 0x0 then // Security mode different from 0
        break(DecodeResult(Map.empty, Seq("Invalid uplink payload: MBus TPL encryption is not supported"), Seq.empty))

      // APL according to EN 13757-3
      val initialState = DecoderState(headerPos, Vector.empty, Vector.empty, Vector.empty)

      @tailrec
      def readDibExtensions(pos: Int, temp: Int, storagenumber: Int, snBitShift: Int): (Int, Int) =
        if (temp & 0x80) == 0 || pos >= raw.length then (pos, storagenumber)
        else
          val newTemp = raw(pos) & 0xFF
          readDibExtensions(pos + 1, newTemp, storagenumber + ((newTemp & 0xF) << snBitShift), snBitShift + 4)

      @tailrec
      def readVibExtensions(pos: Int, temp: Int, acc: Vector[Int]): (Int, Vector[Int]) =
        if (temp & 0x80) == 0 || pos >= raw.length then (pos, acc)
        else
          val newTemp = raw(pos) & 0xFF
          readVibExtensions(pos + 1, newTemp, acc :+ newTemp)

      def parseDatafield(datafield: Int): (Int, Boolean, Boolean) =
        datafield match
          case 0 | 0x8 => (0, false, false)
          case 0x9     => (1, true, false)
          case 0x1     => (1, false, false)
          case 0xA     => (2, true, false)
          case 0x2     => (2, false, false)
          case 0xB     => (3, true, false)
          case 0x3     => (3, false, false)
          case 0xC     => (4, true, false)
          case 0x4 | 0x5 => (4, false, false)
          case 0xE     => (6, true, false)
          case 0x6     => (6, false, false)
          case 0x7     => (8, false, false)
          case 0xD     => (1, false, true)
          case _       => (0, false, false)

      @tailrec
      def parseRecords(state: DecoderState): DecoderState =
        if state.pos >= raw.length then state
        else
          val temp = raw(state.pos) & 0xFF
          val nextPos = state.pos + 1

          if temp == 0x2F then
            parseRecords(state.copy(pos = nextPos)) // Skip filler bytes
          else if temp == 0x0F || temp == 0x1F || temp == 0x7F then
            // Unsupported special DIF functions - terminal error
            state.copy(
              errors = state.errors :+ "Invalid uplink payload: Unsupported special DIF function"
            )
          else
            // DIB
            val datafield = temp & 0xF
            val functionfield = (temp & 0x30) >> 4
            val initialStoragenumber = (temp & 0x40) >> 6

            val (posAfterDibExt, storagenumber) =
              readDibExtensions(nextPos, temp, initialStoragenumber, 1)

            val dib = DibObject(datafield, functionfield, storagenumber)

            // VIB
            val vibFirst = raw(posAfterDibExt) & 0xFF
            val posAfterVibFirst = posAfterDibExt + 1
            val (posAfterVibExt, vibBytes) =
              readVibExtensions(posAfterVibFirst, vibFirst, Vector(vibFirst))

            // Parse VIF code
            val vib = parseVib(vibBytes.toArray)
            if vib.isEmpty then
              state.copy(
                pos = posAfterVibExt,
                errors = state.errors :+ "Invalid uplink payload: Unsupported VIB"
              )
            else
              // Data
              val (sizeByte, bcd, lvar) = parseDatafield(datafield)

              if raw.length < posAfterVibExt + sizeByte || sizeByte > 6 then
                state.copy(
                  pos = posAfterVibExt,
                  errors = state.errors :+ "Invalid uplink payload: Not enough bytes for datafield or datafield is larger than 6 bytes"
                )
              else if !lvar then
                val recordData: Option[Any] =
                  if bcd then typeA(raw, posAfterVibExt, sizeByte)
                  else
                    vib.get.conversionType match
                      case "C" => typeC(raw, posAfterVibExt, sizeByte)
                      case "B" => typeB(raw, posAfterVibExt, sizeByte)
                      case "D" => Some(typeD(raw, posAfterVibExt, sizeByte))
                      case "G" => typeG(raw, posAfterVibExt, sizeByte)
                      case "F/J/I/M" =>
                        if sizeByte == 4 then typeF(raw, posAfterVibExt, sizeByte)
                        else if sizeByte == 6 then typeI(raw, posAfterVibExt)
                        else None
                      case _ => None

                val newRecord = MbusRecord(dib, vib, recordData, None)
                parseRecords(state.copy(
                  pos = posAfterVibExt + sizeByte,
                  records = state.records :+ newRecord
                ))
              else // Lvar
                val nbBytes = raw(posAfterVibExt) & 0xFF
                val dataStart = posAfterVibExt + 1
                if raw.length < dataStart + nbBytes || nbBytes < 3 then
                  state.copy(
                    pos = dataStart,
                    errors = state.errors :+ "Invalid uplink payload: Not enough bytes for LVAR"
                  )
                else if !vib.get.isProfileData then
                  state.copy(
                    pos = dataStart,
                    errors = state.errors :+ "Invalid uplink payload: LVAR that is not Inverse Compact Profile is not supported"
                  )
                else
                  val profileData = inverseCompactProfile(raw, dataStart, nbBytes)
                  if profileData.isEmpty then
                    state.copy(
                      pos = dataStart + nbBytes,
                      errors = state.errors :+ "Invalid uplink payload: Could not parse Inverse Compact Profile"
                    )
                  else
                    val newRecord = MbusRecord(dib, vib, None, profileData)
                    parseRecords(state.copy(
                      pos = dataStart + nbBytes,
                      records = state.records :+ newRecord
                    ))

      val finalState = parseRecords(initialState)

      // On any error during parsing, return early like the original code
      if finalState.errors.nonEmpty then
        break(DecodeResult(Map.empty, finalState.errors, finalState.warnings))

      // Append functionfield and orthogonal VIFE to type
      val processedRecords = finalState.records.map { record =>
        record.vib match
          case Some(vib) =>
            val orthoPrefix =
              if vib.orthoVife != "NA" && vib.orthoVife != "Inverse Compact Profile" then s"${vib.orthoVife} "
              else ""

            val functionFieldText = record.dib.functionfield match
              case 0x0 => ""
              case 0x1 => "Max "
              case 0x2 => "Min "
              case 0x3 => "Error state "
              case _ => ""

            val typeStr = functionFieldText + orthoPrefix + vib.`type`
            record.copy(vib = Some(vib.copy(`type` = typeStr)))
          case None => record
      }

      // Retrieve timestamps
      val (timestamps, timestampWarnings) = processedRecords.foldLeft((Map.empty[Int, ZonedDateTime], Vector.empty[String])) {
        case ((tsMap, warns), record) =>
          record.vib match
            case Some(vib) if vib.`type` == "Date/time" =>
              record.data match
                case Some(dt: ZonedDateTime) => (tsMap + (record.dib.storagenumber -> dt), warns)
                case Some(_) => (tsMap, warns :+ "Invalid value among timestamps")
                case None => (tsMap, warns)
            case _ => (tsMap, warns)
      }

      val allWarnings = finalState.warnings ++ timestampWarnings

      // Generate the output data
      case class OutputAccumulator(
        values: Vector[Map[String, Any]],
        latestVolume: Option[Double],
        errors: Vector[String],
        warnings: Vector[String]
      )

      val output = processedRecords.foldLeft(OutputAccumulator(Vector.empty, None, Vector.empty, allWarnings)) { (acc, currentRecord) =>
        currentRecord.vib match
          case None => acc
          case Some(vib) =>
            if vib.`type` == "Date/time" then
              acc // Skip timestamps as they are mapped directly onto records
            else if vib.`type`.contains("Infocode") && currentRecord.data.isDefined then
              // Special infocode handling
              currentRecord.data match
                case Some(data: Long) =>
                  val infocodeValues = infocodeTable.toVector.sortBy(_._1).map { case (j, typeVal) =>
                    val boolValue = (data & (1L << j)) != 0
                    val value = if boolValue then "True" else "False"

                    val timestamp = timestamps.get(currentRecord.dib.storagenumber).map { ts =>
                      ts.format(DateTimeFormatter.ISO_LOCAL_DATE_TIME)
                    }

                    Map(
                      "Type" -> typeVal,
                      "Value" -> value,
                      "Unit" -> vib.unit,
                      "Timestamp" -> timestamp.orNull
                    )
                  }
                  acc.copy(values = acc.values ++ infocodeValues)
                case _ => acc
            else if !vib.isProfileData then // Regular values
              val typeVal = vib.`type`

              // If invalid ALD
              val dataValue = if typeVal == "ALD last day" && currentRecord.data.contains(4095L) then
                None
              else
                currentRecord.data

              val (value, unit, extraWarnings) = dataValue match
                case Some(d: Long) => (Some(normalize(d, vib.resolution)), vib.unit, Vector.empty[String])
                case Some(d: Double) => (Some(normalize(d, vib.resolution)), vib.unit, Vector.empty[String])
                case _ => (None, "Invalid", Vector("Invalid value among data"))

              val timestamp = timestamps.get(currentRecord.dib.storagenumber).map { ts =>
                ts.format(DateTimeFormatter.ISO_LOCAL_DATE_TIME)
              }

              val newValue = Map(
                "Type" -> typeVal,
                "Value" -> value.orNull,
                "Unit" -> unit,
                "Timestamp" -> timestamp.orNull
              )

              val newLatestVolume = if typeVal == "Volume" && value.isDefined then value else acc.latestVolume

              acc.copy(
                values = acc.values :+ newValue,
                latestVolume = newLatestVolume,
                warnings = acc.warnings ++ extraWarnings
              )
            else // Profile values
              val typeVal = vib.`type`
              val unit = vib.unit

              // Find base value
              val baseRecord = processedRecords.find { item =>
                item.vib.exists { itemVib =>
                  itemVib.`type` == typeVal &&
                  itemVib.unit == unit &&
                  item.dib.storagenumber == currentRecord.dib.storagenumber &&
                  itemVib.resolution == vib.resolution &&
                  itemVib.isProfileData != vib.isProfileData
                }
              }

              (baseRecord.flatMap(_.data), currentRecord.profileData) match
                case (Some(baseData: Long), Some(profile)) =>
                  timestamps.get(currentRecord.dib.storagenumber) match
                    case Some(baseTimestamp) =>
                      val (profileValues, _, _, profileWarnings) =
                        profile.profileValues.foldLeft((Vector.empty[Map[String, Any]], baseData, baseTimestamp, Vector.empty[String])) {
                          case ((vals, tempVal, ts, warns), deltaValue) =>
                            getNextTimestamp(ts, profile.spacingUnit, profile.spacingValue) match
                              case Some(nextTs) =>
                                deltaValue match
                                  case Some(delta) =>
                                    val newTempVal = if profile.incMode == 2 then tempVal + delta else tempVal - delta
                                    val value = normalize(newTempVal, vib.resolution)
                                    val timestampStr = nextTs.format(DateTimeFormatter.ISO_LOCAL_DATE_TIME)
                                    val entry = Map(
                                      "Type" -> typeVal,
                                      "Value" -> value,
                                      "Unit" -> unit,
                                      "Timestamp" -> timestampStr
                                    )
                                    (vals :+ entry, newTempVal, nextTs, warns)
                                  case None =>
                                    (vals, tempVal, nextTs, warns :+ "Invalid value among profile values or timestamps for profile values")
                              case None =>
                                (vals, tempVal, ts, warns :+ "Invalid value among profile values or timestamps for profile values")
                        }
                      acc.copy(
                        values = acc.values ++ profileValues,
                        warnings = acc.warnings ++ profileWarnings
                      )
                    case None =>
                      acc.copy(errors = acc.errors :+ "Invalid uplink payload: Could not find base time for profile data")
                case _ =>
                  acc.copy(errors = acc.errors :+ "Invalid uplink payload: Could not find base value for profile data")
      }

      val dataMap = output.latestVolume match
        case Some(vol) => Map("values" -> output.values.toSeq, "latestVolume" -> vol)
        case None => Map("values" -> output.values.toSeq)

      DecodeResult(dataMap, output.errors, output.warnings)

  private def decodeFlowiq2200Payload(payload: String): Map[String, Any] =
    val result = decode(payload)
    result.data

  def transform(data: Map[String, Any], version: String): Seq[SensorRecord] =
    logger.debug(s"FLOWIQ2200 $data")

    val required = Seq("schematype", "customerid", "WirelessMetadata", "PayloadData")
    val missing = data.missingFields(required)

    if missing.nonEmpty then
      logger.info("FLOWIQ2200 Missing Fields")
      throw new IllegalArgumentException(s"Missing required fields: ${missing.mkString(", ")}")

    if !data.isString("PayloadData") then
      logger.info("FLOWIQ2200 Payload not string")
      throw new IllegalArgumentException("PayloadData must be a string")

    logger.info("FLOWIQ2200 decode")
    val payloads = decodeFlowiq2200Payload(data("PayloadData").toString)
    logger.debug(s"FLOWIQ2200 payloads $payloads")

    val schematype = data("schematype").toString
    val customerid = data("customerid").toString
    val wirelessMetadata = data.nestedMap("WirelessMetadata")
    val loRaWAN = wirelessMetadata.nestedMap("LoRaWAN")
    val devEui = loRaWAN("DevEui").toString

    val ingestedTime = isoFormatter.format(Instant.now())

    val values = payloads.get("values") match
      case Some(s: Seq[?]) => s.collect { case m: Map[?, ?] => m.asInstanceOf[Map[String, Any]] }
      case _ => Seq.empty

    values.map { payload =>
      val typeVal = payload("Type").toString
      val sensorId = typeVal
      val daqId = buildDaqId(schematype, customerid, devEui, sensorId)

      val timestampStr = payload.get("Timestamp") match
        case Some(ts: String) =>
          val normalizedTs = if ts.endsWith("Z") || ts.matches(".*[+-]\\d{2}:\\d{2}$") then
            ts
          else
            ts + "Z"
          isoFormatter.format(parseTimestamp(normalizedTs))
        case _ =>
          ingestedTime

      val value = payload.get("Value") match
        case Some(null) => "None"  // Python str(None) returns "None"
        case Some(v) => v.toString
        case None => ""

      val unit = payload.get("Unit") match
        case Some(u) if u != null => u.toString
        case _ => "NA"

      SensorRecord(
        daqId = daqId,
        `type` = schematype,
        gatewayId = customerid,
        meterId = devEui,
        timestamp = timestampStr,
        ingestedTime = ingestedTime,
        sensorId = sensorId,
        value = value,
        unit = unit
      )
    }
