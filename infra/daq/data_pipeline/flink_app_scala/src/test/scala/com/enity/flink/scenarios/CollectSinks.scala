package com.enity.flink.scenarios

import com.enity.flink.enrichment.{EnrichedRecord, ErrorRecord}
import org.apache.flink.streaming.api.functions.sink.SinkFunction

import java.util.Collections
import scala.jdk.CollectionConverters.*

object CollectSinks:
  val enriched: java.util.List[EnrichedRecord] =
    Collections.synchronizedList(new java.util.ArrayList[EnrichedRecord]())
  val parseErrors: java.util.List[ErrorRecord] =
    Collections.synchronizedList(new java.util.ArrayList[ErrorRecord]())
  val deadLetters: java.util.List[ErrorRecord] =
    Collections.synchronizedList(new java.util.ArrayList[ErrorRecord]())
  val anomalies: java.util.List[ErrorRecord] =
    Collections.synchronizedList(new java.util.ArrayList[ErrorRecord]())
  val lateArrivals: java.util.List[ErrorRecord] =
    Collections.synchronizedList(new java.util.ArrayList[ErrorRecord]())

  def clear(): Unit =
    enriched.clear()
    parseErrors.clear()
    deadLetters.clear()
    anomalies.clear()
    lateArrivals.clear()

  def allErrors: Map[String, List[ErrorRecord]] = Map(
    "PARSE_ERROR" -> parseErrors.asScala.toList,
    "DEAD_LETTER" -> deadLetters.asScala.toList,
    "ANOMALY" -> anomalies.asScala.toList,
    "LATE_ARRIVAL" -> lateArrivals.asScala.toList
  )

class EnrichedSink extends SinkFunction[EnrichedRecord] with Serializable:
  override def invoke(value: EnrichedRecord, context: SinkFunction.Context): Unit =
    CollectSinks.enriched.add(value)

class ErrorSink(tag: String) extends SinkFunction[ErrorRecord] with Serializable:
  override def invoke(value: ErrorRecord, context: SinkFunction.Context): Unit =
    tag match
      case "PARSE_ERROR" => CollectSinks.parseErrors.add(value)
      case "DEAD_LETTER" => CollectSinks.deadLetters.add(value)
      case "ANOMALY"     => CollectSinks.anomalies.add(value)
      case "LATE_ARRIVAL" => CollectSinks.lateArrivals.add(value)
