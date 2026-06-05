package com.enity.flink.scenarios

import com.enity.flink.SensorRecord
import com.enity.flink.enrichment.*
import org.apache.flink.streaming.api.functions.ProcessFunction
import org.apache.flink.util.Collector

import java.time.Instant

/** Replaces MeterEnrichmentFunction for tests.
  * Pre-loaded mappings instead of DDB bootstrap + broadcast state. */
class TestEnrichmentMapper(mappings: java.util.Map[String, MeterMapping])
    extends ProcessFunction[SensorRecord, (EnrichedRecord, MeterMapping)]:

  override def processElement(
      record: SensorRecord,
      ctx: ProcessFunction[SensorRecord, (EnrichedRecord, MeterMapping)]#Context,
      out: Collector[(EnrichedRecord, MeterMapping)]
  ): Unit =
    val mapping = mappings.get(record.daqId)
    if mapping != null then
      val enriched = MeterEnrichmentFunction.enrich(record, mapping)
      out.collect((enriched, mapping))
    else
      ctx.output(SideOutputTags.DEAD_LETTER, ErrorRecord(
        errorType = "dead_letter",
        timestamp = Instant.now().toString,
        daqId = record.daqId,
        payload = s"value=${record.value}, unit=${record.unit}, ts=${record.timestamp}",
        error = s"No mapping for daqId: ${record.daqId}"
      ))
