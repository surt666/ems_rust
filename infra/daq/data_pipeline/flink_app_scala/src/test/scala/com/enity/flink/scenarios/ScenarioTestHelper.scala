package com.enity.flink.scenarios

import com.enity.flink.SensorRecord
import com.enity.flink.enrichment.*
import com.enity.flink.utils.Extensions
import org.apache.flink.api.common.eventtime.{SerializableTimestampAssigner, WatermarkStrategy}
import org.apache.flink.api.common.typeinfo.TypeInformation
import org.apache.flink.streaming.api.environment.StreamExecutionEnvironment
import org.apache.flink.streaming.api.functions.ProcessFunction
import org.apache.flink.util.Collector

import java.time.{Duration, Instant}
import scala.jdk.CollectionConverters.*

case class ScenarioConfig(
  maxOutOfOrdernessMs: Long = 5000L,   // 5s for fast tests (vs 1h production)
  bufferRetentionMs: Long = 30000L     // 30s for fast tests (vs 6h production)
)

case class ScenarioResult(
  enrichedRecords: List[EnrichedRecord],
  sideOutputs: Map[String, List[ErrorRecord]]
)

object ScenarioTestHelper:

  /** Run full pipeline from raw JSON strings (tests parsing + enrichment + delta). */
  def buildAndRunFromJson(
      jsonStrings: List[String],
      mappings: Map[String, MeterMapping],
      config: ScenarioConfig = ScenarioConfig()
  ): ScenarioResult =
    val env = StreamExecutionEnvironment.getExecutionEnvironment
    env.setParallelism(1)

    // Parse JSON → SensorRecord (reusing Main.processJsonToRecords)
    val parsedStream = env
      .fromCollection(jsonStrings.asJava, TypeInformation.of(classOf[String]))
      .process(new ProcessFunction[String, SensorRecord] {
        override def processElement(
            value: String,
            ctx: ProcessFunction[String, SensorRecord]#Context,
            out: Collector[SensorRecord]
        ): Unit =
          try
            val records = com.enity.flink.Main.processJsonToRecords(value)
            if records.isEmpty then
              ctx.output(SideOutputTags.PARSE_ERROR, ErrorRecord(
                errorType = "parse_error",
                timestamp = Instant.now().toString,
                daqId = "",
                payload = value.take(1000),
                error = "No valid records produced"
              ))
            else
              records.foreach(out.collect)
          catch
            case e: Exception =>
              ctx.output(SideOutputTags.PARSE_ERROR, ErrorRecord(
                errorType = "parse_error",
                timestamp = Instant.now().toString,
                daqId = "",
                payload = value.take(1000),
                error = e.getMessage
              ))
      })
      .returns(TypeInformation.of(classOf[SensorRecord]))

    parsedStream.getSideOutput(SideOutputTags.PARSE_ERROR)
      .addSink(new ErrorSink("PARSE_ERROR"))

    wireEnrichmentPipeline(parsedStream, mappings, config)

    env.execute("scenario-test")
    collectResult()

  /** Run pipeline from pre-parsed SensorRecords (tests enrichment + delta only). */
  def buildAndRunFromRecords(
      sensorRecords: List[SensorRecord],
      mappings: Map[String, MeterMapping],
      config: ScenarioConfig = ScenarioConfig()
  ): ScenarioResult =
    val env = StreamExecutionEnvironment.getExecutionEnvironment
    env.setParallelism(1)

    val sensorStream = env.fromCollection(
      sensorRecords.asJava, TypeInformation.of(classOf[SensorRecord])
    )

    wireEnrichmentPipeline(sensorStream, mappings, config)

    env.execute("scenario-test")
    collectResult()

  private def wireEnrichmentPipeline(
      sensorStream: org.apache.flink.streaming.api.datastream.DataStream[SensorRecord],
      mappings: Map[String, MeterMapping],
      config: ScenarioConfig
  ): Unit =
    val javaMap = new java.util.HashMap[String, MeterMapping]()
    mappings.foreach { case (k, v) => javaMap.put(k, v) }

    val watermarkStrategy = WatermarkStrategy
      .forBoundedOutOfOrderness[SensorRecord](Duration.ofMillis(config.maxOutOfOrdernessMs))
      .withTimestampAssigner(new SerializableTimestampAssigner[SensorRecord] {
        override def extractTimestamp(record: SensorRecord, previousTs: Long): Long =
          Extensions.parseTimestamp(record.timestamp).toEpochMilli
      })

    val watermarked = sensorStream.assignTimestampsAndWatermarks(watermarkStrategy)

    // Note: Do NOT add .returns() here — Flink infers type from ProcessFunction generics.
    // Adding explicit TypeInformation for Scala tuples causes TypeExtractor failures.
    val enrichedStream = watermarked
      .process(new TestEnrichmentMapper(javaMap))

    enrichedStream.getSideOutput(SideOutputTags.DEAD_LETTER)
      .addSink(new ErrorSink("DEAD_LETTER"))

    val binnedStream = enrichedStream
      .keyBy((t: (EnrichedRecord, MeterMapping)) => java.lang.Integer.valueOf(t._1.logicalId))
      .process(new ResampleFunction(config.bufferRetentionMs))

    binnedStream.addSink(new EnrichedSink())

    binnedStream.getSideOutput(SideOutputTags.ANOMALY)
      .addSink(new ErrorSink("ANOMALY"))
    binnedStream.getSideOutput(SideOutputTags.LATE_ARRIVAL)
      .addSink(new ErrorSink("LATE_ARRIVAL"))

  private def collectResult(): ScenarioResult =
    ScenarioResult(
      enrichedRecords = CollectSinks.enriched.asScala.toList,
      sideOutputs = CollectSinks.allErrors
    )
