package com.enity.flink.enrichment

import com.enity.flink.SensorRecord
import org.apache.flink.api.common.state.MapStateDescriptor
import org.apache.flink.api.common.typeinfo.{BasicTypeInfo, TypeInformation}
import org.apache.flink.configuration.Configuration
import org.apache.flink.streaming.api.functions.co.BroadcastProcessFunction
import org.apache.flink.util.Collector
import org.slf4j.LoggerFactory
import software.amazon.awssdk.regions.Region
import software.amazon.awssdk.services.dynamodb.DynamoDbClient

import java.time.Instant

/** BroadcastProcessFunction that enriches SensorRecords with meter identity and hierarchy context.
  *
  * Main input: SensorRecord stream
  * Broadcast input: IdMappingChange stream (from DDB Streams via Kinesis)
  * Output: (EnrichedRecord, MeterMapping) tuples — mapping carries meterType + resampleMinutes for ResampleFunction
  *
  * On open(), performs a full DynamoDB scan to bootstrap the local cache.
  * CDC events from the broadcast stream update broadcast state, which takes priority. */
class MeterEnrichmentFunction(region: String, tableName: String)
    extends BroadcastProcessFunction[SensorRecord, IdMappingChange, (EnrichedRecord, MeterMapping)]:

  @transient private lazy val logger = LoggerFactory.getLogger(getClass)
  @transient private var bootstrapCache: Map[String, MeterMapping] = Map.empty

  override def open(parameters: Configuration): Unit =
    super.open(parameters)
    try
      val client = DynamoDbClient.builder()
        .region(Region.of(region))
        .build()
      bootstrapCache = DdbBootstrapLoader.loadAll(client, tableName)
      client.close()
      logger.info(s"Bootstrap cache loaded: ${bootstrapCache.size} mappings")
    catch
      case e: Exception =>
        logger.error(s"Bootstrap scan failed: ${e.getMessage}", e)
        // Continue without bootstrap — CDC stream will populate state

  override def processElement(
      record: SensorRecord,
      ctx: BroadcastProcessFunction[SensorRecord, IdMappingChange, (EnrichedRecord, MeterMapping)]#ReadOnlyContext,
      out: Collector[(EnrichedRecord, MeterMapping)]
  ): Unit =
    val state = ctx.getBroadcastState(MeterEnrichmentFunction.ID_MAP)
    // Broadcast state takes priority, fall back to bootstrap cache
    val mapping = Option(state.get(record.daqId)).orElse(bootstrapCache.get(record.daqId))

    mapping match
      case Some(m) =>
        out.collect((MeterEnrichmentFunction.enrich(record, m), m))
      case None =>
        val errorRecord = ErrorRecord(
          errorType = "dead_letter",
          timestamp = Instant.now().toString,
          daqId = record.daqId,
          payload = s"value=${record.value}, unit=${record.unit}, ts=${record.timestamp}",
          error = "No meter mapping found in broadcast state"
        )
        ctx.output(SideOutputTags.DEAD_LETTER, errorRecord)

  override def processBroadcastElement(
      change: IdMappingChange,
      ctx: BroadcastProcessFunction[SensorRecord, IdMappingChange, (EnrichedRecord, MeterMapping)]#Context,
      out: Collector[(EnrichedRecord, MeterMapping)]
  ): Unit =
    val state = ctx.getBroadcastState(MeterEnrichmentFunction.ID_MAP)
    change.eventType match
      case "INSERT" | "MODIFY" =>
        change.mapping.foreach(m => state.put(change.daqId, m))
        logger.debug(s"Updated mapping for ${change.daqId}")
      case "REMOVE" =>
        state.remove(change.daqId)
        logger.debug(s"Removed mapping for ${change.daqId}")
      case other =>
        logger.warn(s"Unknown DDB event type: $other for ${change.daqId}")

object MeterEnrichmentFunction:

  val ID_MAP: MapStateDescriptor[String, MeterMapping] =
    new MapStateDescriptor[String, MeterMapping](
      "meter-id-map",
      BasicTypeInfo.STRING_TYPE_INFO,
      TypeInformation.of(classOf[MeterMapping])
    )

  /** Pure function for testability. Passes through the original timestamp un-floored.
    * ResampleFunction downstream computes resample_timestamp / resample_value / resample_method from
    * the raw timestamp + per-meter resampleMinutes config. */
  def enrich(record: SensorRecord, m: MeterMapping): EnrichedRecord =
    EnrichedRecord(
      logicalId = m.logicalId,
      timestamp = record.timestamp,
      value = record.value.toDouble,
      unit = record.unit,
      ingestedTime = record.ingestedTime,
      hn1 = m.hn1,
      hn2 = m.hn2,
      hn3 = m.hn3,
      hn4 = m.hn4,
      hn5 = m.hn5,
      hn6 = m.hn6,
      hn7 = m.hn7,
      hn8 = m.hn8,
      hn9 = m.hn9,
      purpose = m.purpose
    )
