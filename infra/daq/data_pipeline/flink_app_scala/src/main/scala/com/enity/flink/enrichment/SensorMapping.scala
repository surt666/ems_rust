package com.enity.flink.enrichment

import com.fasterxml.jackson.annotation.JsonProperty
import java.time.Instant

/** Broadcast state entry: one per daqId.
  * Uses java.lang.Integer for nullable fields to avoid Flink Kryo serialization
  * corrupting Scala Option[Int] across operator boundaries.
  *
  * `logicalId` is the sensor id (int). `hn1`/`hn2` are partner/company; `hn3..hn9`
  * are schema-defined per company. `energyType` is what the sensor measures
  * (e.g., "electricity", "water", "district_heating") — the energitype axis, not
  * what the energy is spent on. */
case class SensorMapping(
  logicalId: Int,
  readingKind: String,
  hn1: Int,
  hn2: Int,
  hn3: java.lang.Integer,
  hn4: java.lang.Integer,
  hn5: java.lang.Integer,
  hn6: java.lang.Integer,
  hn7: java.lang.Integer,
  hn8: java.lang.Integer,
  hn9: java.lang.Integer,
  energyType: String,
  resampleMinutes: java.lang.Integer = null
) extends Serializable

/** Keyed state for counter delta computation */
case class CounterState(
  lastValue: Double,
  lastTimestamp: Instant
) extends Serializable

/** Buffered reading for the event-time reordering buffer in ResampleFunction.
  * Carries the mapping so the operator has access to readingKind + resampleMinutes per reading. */
case class BufferedReadingV2(
  cumulativeValue: Double,
  record: EnrichedRecord,
  mapping: SensorMapping
) extends Serializable

/** DDB Streams change event */
case class IdMappingChange(
  eventType: String,
  daqId: String,
  mapping: Option[SensorMapping]
) extends Serializable

/** Enriched record ready for Iceberg sink.
  * Uses java.lang.Integer / java.lang.Long / java.lang.Double for nullable fields
  * to avoid Flink Kryo serialization corrupting Scala Option across operator boundaries.
  *
  * `resampleTimestamp` (epoch millis) / `resampleValue` / `resampleMethod` are populated by
  * ResampleFunction. For meters with `resampleMinutes=null`, all three are null and the raw
  * reading is what reaches the sink.
  *
  * `resampleMethod` is provenance for tests and debugging only — `logical_data` has no such
  * column (it holds one value per timestamp, however that value was arrived at). */
case class EnrichedRecord(
  logicalId: Int,
  timestamp: String,
  value: Double,
  unit: String,
  ingestedTime: String,
  hn1: Int,
  hn2: Int,
  hn3: java.lang.Integer,
  hn4: java.lang.Integer,
  hn5: java.lang.Integer,
  hn6: java.lang.Integer,
  hn7: java.lang.Integer,
  hn8: java.lang.Integer,
  hn9: java.lang.Integer,
  energyType: String,
  readingKind: String,
  resampleTimestamp: java.lang.Long = null,
  resampleValue: java.lang.Double = null,
  resampleMethod: String = null
) extends Serializable

/** Error/dead-letter record for the error Kinesis stream.
  * Uses @JsonProperty to serialize errorType as "type" and daqId as "daq_id". */
case class ErrorRecord(
  @JsonProperty("type") errorType: String,
  timestamp: String,
  @JsonProperty("daq_id") daqId: String,
  payload: String,
  error: String
) extends Serializable
