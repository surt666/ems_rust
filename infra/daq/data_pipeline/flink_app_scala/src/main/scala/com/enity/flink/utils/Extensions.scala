package com.enity.flink.utils

import java.time.{Instant, OffsetDateTime, ZoneOffset}
import java.time.format.DateTimeFormatter
import scala.util.Try

object Extensions:

  val isoFormatter: DateTimeFormatter =
    DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ss.SSSSSS'Z'").withZone(ZoneOffset.UTC)

  def parseTimestamp(ts: String): Instant =
    Try(Instant.parse(ts)).getOrElse(OffsetDateTime.parse(ts).toInstant)

  extension (data: Map[String, Any])
    /** Safely extract a nested Map */
    def nestedMap(key: String): Map[String, Any] = data(key) match
      case m: Map[?, ?] => m.asInstanceOf[Map[String, Any]]
      case other => throw IllegalArgumentException(s"Expected Map for key '$key', got ${other.getClass}")

    /** Safely extract a Seq of Maps */
    def seqOfMaps(key: String): Seq[Map[String, Any]] = data(key) match
      case s: Seq[?] => s.map { case m: Map[?, ?] => m.asInstanceOf[Map[String, Any]] }
      case other => throw IllegalArgumentException(s"Expected Seq for key '$key', got ${other.getClass}")

    /** Check if a value is a Seq */
    def isSeq(key: String): Boolean = data.get(key).exists(_.isInstanceOf[Seq[?]])

    /** Check if a value is a String */
    def isString(key: String): Boolean = data.get(key).exists(_.isInstanceOf[String])

    /** Validate required fields, returning missing field names */
    def missingFields(required: Seq[String]): Seq[String] = required.filterNot(data.contains)

  /** Build a standardized daq ID from parts */
  def buildDaqId(parts: String*): String =
    s"daq:${parts.mkString(":")}"
      .toLowerCase
      .replace("-", "_")
      .replace(".", "_")
      .replace(" ", "_")

  /** Normalize a unit string and scale the value to a canonical base unit.
    * Returns (normalizedUnit, scaledValue). Unknown units pass through unchanged. */
  def normalizeUnit(unit: String, value: Double): (String, Double) =
    val (normalized, factor) = unitFactor(unit)
    (normalized, value * factor)

  /** Look up canonical unit + scaling factor for a raw unit string. Used when the same
    * factor needs to scale multiple values from one record (e.g., `value` and `resample_value`). */
  def unitFactor(unit: String): (String, Double) =
    UnitConversions.getOrElse(unit, (unit, 1.0))

  private val UnitConversions: Map[String, (String, Double)] = Map(
    "Energy (10 Wh)"                    -> ("Wh", 10),
    "Energy (kWh)"                      -> ("Wh", 1000),
    "Energy (100 Wh)"                   -> ("Wh", 100),
    "Power (1e-1 W)"                    -> ("W", 1e-1),
    "Power (1e-2 W)"                    -> ("W", 1e-2),
    "Power (100 W)"                     -> ("W", 100),
    "W"                                 -> ("W", 1),
    "VA"                                -> ("W", 1),
    "m A"                               -> ("A", 1e-3),
    "A"                                 -> ("A", 1),
    "1e-1  V"                           -> ("V", 1e-1),
    "V"                                 -> ("V", 1),
    "Hz"                                -> ("Hz", 1),
    "Return temperature (1e-2 deg C)"   -> ("C", 1e-2),
    "Flow temperature (1e-2 deg C)"     -> ("C", 1e-2),
    "Flow temperature (deg C)"          -> ("C", 1),
    "Volume (1e-2 m^3)"                 -> ("m^3", 1e-2),
    "Volume flow (m m^3/h)"             -> ("m^3/h", 1e-3),
    "Volume (m m^3)"                    -> ("m^3", 1e-3),
    "WATT"                              -> ("W", 1),
    "KILO_WATT"                         -> ("W", 1000),
    "WATT_HOUR"                         -> ("Wh", 1),
    "KILO_WATT_HOUR"                    -> ("Wh", 1000),
    "CUBIC_METRE"                       -> ("m^3", 1),
    "CUBIC_METRE_HOUR"                  -> ("m^3/h", 1),
    "m3PerHour"                         -> ("m^3/h", 1),
    "m^3/h"                             -> ("m^3/h", 1),
    "DEGREE_CELSIUS"                    -> ("C", 1),
    "british-thermal-unit"              -> ("Wh", 0.293),
    "C"                                 -> ("C", 1),
    "Celcius"                           -> ("C", 1),
    "Kelvin"                            -> ("K", 1),
    "fluid-ounce"                       -> ("m3", 0.000030),
    "fluid-ounce-imperial"              -> ("m3", 0.000028),
    "foot"                              -> ("m", 0.305),
    "gallon"                            -> ("m3", 0.003785),
    "gallon-imperial"                   -> ("m3", 0.004546),
    "Gcal"                              -> ("Wh", 1163000.0),
    "GJ"                                -> ("Wh", 278000.0),
    "Kg"                                -> ("g", 1000),
    "kg-Fgas"                           -> ("Wh", 13900.0),
    "Kg-træpiller"                      -> ("Wh", 4865),
    "km"                                -> ("m", 1000),
    "Kr."                               -> ("Kr", 1),
    "kWh"                               -> ("Wh", 1000),
    "KWH"                               -> ("Wh", 1000),
    "Liter"                             -> ("m3", 0.001),
    "Liter-gasolie"                     -> ("Wh", 9890),
    "m3"                                -> ("m^3", 1),
    "m^3"                               -> ("m^3", 1),
    "m3-10Gr"                           -> ("Wh", 11627.910),
    "m3-25Gr"                           -> ("Wh", 29069.770),
    "m3-30Gr"                           -> ("Wh", 34883.720),
    "m3-35Gr"                           -> ("Wh", 40705.002),
    "m3-40Gr"                           -> ("Wh", 46509.998),
    "m3-5Gr"                            -> ("Wh", 5813.950),
    "m3-Bgas"                           -> ("Wh", 4380.0),
    "m3-Fgas"                           -> ("Wh", 34194.0),
    "m3-fjv"                            -> ("Wh", 34883.720),
    "m3-kond."                          -> ("Wh", 700000),
    "m3-Ngas"                           -> ("Wh", 11000.0),
    "mile"                              -> ("m", 1609.340),
    "MJ"                                -> ("Wh", 278),
    "mm-british-thermal-unit"           -> ("Wh", 293071.070),
    "MWh"                               -> ("Wh", 1000000),
    "Nautical miles"                    -> ("m", 1.852),
    "Nm3"                               -> ("Nm3", 1),
    "Bar"                               -> ("bar", 1),
    "ounce"                             -> ("g", 28),
    "Pct"                               -> ("%", 1),
    "Percent"                           -> ("%", 1),
    "pejling"                           -> ("m3", 0.001),
    "pound"                             -> ("g", 454),
    "påfyldt"                           -> ("m3", 0.001),
    "Styk"                              -> ("Units", 1),
    "timer"                             -> ("s", 3600),
    "Ton"                               -> ("g", 1000000),
    "Ton-træpiller"                     -> ("Wh", 4865000),
    "Wh"                                -> ("Wh", 1),
    "ppm"                               -> ("ppm", 1),
    "ppb"                               -> ("ppb", 1),
    "RH%"                               -> ("RH%", 1),
    ""                                  -> ("EMPTY", 1),
  )
