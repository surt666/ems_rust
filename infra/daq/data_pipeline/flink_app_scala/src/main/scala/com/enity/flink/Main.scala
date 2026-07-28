package com.enity.flink

import com.enity.flink.processors.*
import com.enity.flink.utils.{Extensions, ProcessUtils}
import com.enity.flink.enrichment.*
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.scala.DefaultScalaModule
import org.apache.flink.api.common.serialization.SimpleStringSchema
import org.apache.flink.api.common.typeinfo.{TypeInformation, Types}
import org.apache.flink.streaming.api.environment.StreamExecutionEnvironment
import org.apache.flink.streaming.api.functions.ProcessFunction
import org.apache.flink.streaming.connectors.kinesis.FlinkKinesisConsumer
import org.apache.flink.streaming.connectors.kinesis.config.ConsumerConfigConstants
import org.apache.flink.connector.kinesis.sink.{KinesisStreamsSink, PartitionKeyGenerator}
import org.apache.flink.table.api.{DataTypes, TableSchema}
import org.apache.flink.types.Row
import org.apache.flink.util.Collector
import org.apache.iceberg.catalog.{Namespace, TableIdentifier}
import org.apache.iceberg.flink.{CatalogLoader, TableLoader}
import org.apache.iceberg.DistributionMode
import org.apache.iceberg.flink.sink.FlinkSink
import org.slf4j.LoggerFactory

import org.apache.flink.api.common.eventtime.{SerializableTimestampAssigner, WatermarkStrategy}

import java.io.File
import java.time.{Duration, Instant}
import java.util.Properties
import scala.io.Source

object Main:
  private val logger = LoggerFactory.getLogger(getClass)
  private val mapper = new ObjectMapper()
  mapper.registerModule(DefaultScalaModule)

  private val APPLICATION_PROPERTIES_FILE_PATH = "/etc/flink/application_properties.json"

  def processJsonToRecords(value: String): Seq[SensorRecord] =
    logger.debug(s"Processing record: $value")
    try
      val cleanedValue = if value.contains("\\\\") then
        value.replace("\\\"", "\"")
      else
        value

      val data = mapper.readValue(cleanedValue, classOf[Map[String, Any]])
      val dataType = data.getOrElse("schematype", "unknown").toString

      dataType match
        case "emu_profes_v1" =>
          ProcessUtils.processDataToRecords("EMU", EmuProcessor.transform, data, "v1")
        case "gwb143_json_v1" =>
          ProcessUtils.processDataToRecords("GWB143", Gwb143Processor.transform, data, "json_v1")
        case "std_jsonl_v1" =>
          ProcessUtils.processDataToRecords("STD", StdProcessor.transformStdJsonl, data, "jsonl_v1")
        case "std_json_v1" =>
          ProcessUtils.processDataToRecords("STD", StdProcessor.transformStdJson, data, "jsonl_v1")
        case "bluemetering_json_v1" =>
          ProcessUtils.processDataToRecords("BLUE", BluemeteringProcessor.transform, data, "json_v1")
        case "mivo_json_v1" =>
          ProcessUtils.processDataToRecords("MIVO", MivoProcessor.transform, data, "json_v1")
        case "mc603_v1" =>
          ProcessUtils.processDataToRecords("MC603", Mc603Processor.transform, data, "v1")
        case "flowiq2200_v1" =>
          ProcessUtils.processDataToRecords("FLOWIQ", Flowiq2200Processor.transform, data, "v1")
        case "pulse_v1" | "adeunis_pu_v1" =>
          ProcessUtils.processDataToRecords("PULSE", PulseProcessor.transform, data, "v1")
        case "ediel_json_v1" =>
          ProcessUtils.processDataToRecords("EDIEL", EdielProcessor.transformJsonV1, data, "json_v1")
        case _ =>
          logger.warn(s"Unknown schematype: $dataType")
          Seq.empty
    catch
      case e: Exception =>
        logger.error(s"Error processing record: ${e.getMessage}", e)
        Seq.empty

  def getApplicationProperties(): Seq[Map[String, Any]] =
    val file = new File(APPLICATION_PROPERTIES_FILE_PATH)
    if file.exists() then
      try
        val source = Source.fromFile(file)
        val contents = source.mkString
        source.close()
        val raw = mapper.readValue(contents, classOf[Seq[Any]])
        raw.collect { case m: Map[?, ?] => m.asInstanceOf[Map[String, Any]] }
      catch
        case e: Exception =>
          logger.error(s"Error reading application properties: ${e.getMessage}")
          Seq.empty
    else
      logger.warn(s"A file at '$APPLICATION_PROPERTIES_FILE_PATH' was not found")
      Seq.empty

  def propertyMap(props: Seq[Map[String, Any]], propertyGroupId: String): Map[String, String] =
    props.find(_.get("PropertyGroupId").contains(propertyGroupId)) match
      case Some(group) =>
        group.get("PropertyMap") match
          case Some(propMap: Map[?, ?]) =>
            propMap.collect { case (k: String, v: String) => k -> v }
          case _ => Map.empty
      case None => Map.empty

  private def toStreamArn(nameOrArn: String, region: String, accountId: String): String =
    if nameOrArn.startsWith("arn:") then nameOrArn
    else s"arn:aws:kinesis:$region:$accountId:stream/$nameOrArn"

  def createFlinkJob(): Unit =
    val env = StreamExecutionEnvironment.getExecutionEnvironment

    val props = getApplicationProperties()
    val properties = propertyMap(props, "FlinkApplicationProperties")
    logger.info(s"FLINKPROPS: $properties")

    val region = properties.getOrElse("AWS_REGION", "eu-central-1")
    val accountId = properties.getOrElse("ACCOUNT_ID", "")
    val tableBucketName = properties.getOrElse("TABLE_BUCKET_NAME", "measurements")
    val inputStream = properties.getOrElse("INPUT_STREAM", "")

    // Configure Kinesis source (using FlinkKinesisConsumer — KinesisStreamsSource
    // does not support partial recovery, causing job failures on autoscaling/failover)
    val sourceProps = new Properties()
    sourceProps.setProperty("aws.region", region)
    sourceProps.setProperty(ConsumerConfigConstants.STREAM_INITIAL_POSITION, "TRIM_HORIZON")
    sourceProps.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_MAX, "500")
    sourceProps.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_INTERVAL_MILLIS, "1000")
    sourceProps.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_RETRIES, "10")
    sourceProps.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_BACKOFF_BASE, "500")
    sourceProps.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_BACKOFF_MAX, "5000")
    sourceProps.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_BACKOFF_EXPONENTIAL_CONSTANT, "1.5")

    val kinesisSource = new FlinkKinesisConsumer[String](
      inputStream, new SimpleStringSchema(), sourceProps
    )

    // Configure Iceberg catalog for S3 Tables
    val catalogProperties = new java.util.HashMap[String, String]()
    catalogProperties.put("type", "iceberg")
    catalogProperties.put("catalog-impl", "software.amazon.s3tables.iceberg.S3TablesCatalog")
    catalogProperties.put("warehouse", s"arn:aws:s3tables:$region:$accountId:bucket/$tableBucketName")

    val catalogLoader = CatalogLoader.custom(
      "s3tablescatalog",
      catalogProperties,
      new org.apache.hadoop.conf.Configuration(),
      "software.amazon.s3tables.iceberg.S3TablesCatalog"
    )

    val tableIdentifier = TableIdentifier.of(Namespace.of("all"), "raw_data")
    val tableLoader = TableLoader.fromCatalog(catalogLoader, tableIdentifier)

    // Define table schema matching the Iceberg table
    // Note: TableSchema is deprecated by Flink but still required by Iceberg 1.7.x FlinkSink.forRow()
    val tableSchema = TableSchema.builder()
      .field("daq_id", DataTypes.STRING().notNull())
      .field("timestamp", DataTypes.TIMESTAMP_WITH_LOCAL_TIME_ZONE(6).notNull())
      .field("value", DataTypes.DOUBLE().notNull())
      .field("unit", DataTypes.STRING().notNull())
      .field("ingested_time", DataTypes.TIMESTAMP_WITH_LOCAL_TIME_ZONE(6).notNull())
      .build()

    // Read new environment properties
    val sensorIdentityTable = properties.getOrElse("SENSOR_IDENTITY_TABLE", "sensor-identity")
    val ddbChangeStream = properties.getOrElse("DDB_CHANGE_STREAM", "")
    val errorStreamName = properties.getOrElse("ERROR_STREAM", "")

    // ── Step 1: Parse JSON with side output for errors ──

    val parsedStream = env
      .addSource(kinesisSource)
      .uid("kinesis-source")
      .process(new ProcessFunction[String, SensorRecord] {
        override def processElement(
            value: String,
            ctx: ProcessFunction[String, SensorRecord]#Context,
            out: Collector[SensorRecord]
        ): Unit =
          try
            val records = processJsonToRecords(value)
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
      .uid("json-parser")
      .returns(TypeInformation.of(classOf[SensorRecord]))

    val parseErrors = parsedStream.getSideOutput(SideOutputTags.PARSE_ERROR)

    // ── Step 2: Raw Iceberg sink (unchanged logic) ──

    val rawStream = parsedStream
      .map { (record: SensorRecord) =>
        Row.of(
          record.daqId,
          Extensions.parseTimestamp(record.timestamp),
          java.lang.Double.valueOf(record.value.toDouble),
          record.unit,
          Instant.now()
        )
      }
      .returns(Types.ROW_NAMED(
        Array("daq_id", "timestamp", "value", "unit", "ingested_time"),
        Types.STRING, Types.INSTANT, Types.DOUBLE, Types.STRING, Types.INSTANT
      ))
      .uid("raw-row-mapper")

    FlinkSink.forRow(rawStream, tableSchema)
      .tableLoader(tableLoader)
      .distributionMode(DistributionMode.NONE)
      .writeParallelism(1)
      .append()

    // ── Step 3: Enrichment branch (only if DDB stream is configured) ──

    if ddbChangeStream.nonEmpty then
      logger.info(s"Enrichment enabled: DDB_CHANGE_STREAM=$ddbChangeStream, ERROR_STREAM=$errorStreamName")

      val ddbSourceProps = new Properties()
      ddbSourceProps.setProperty("aws.region", region)
      ddbSourceProps.setProperty(ConsumerConfigConstants.STREAM_INITIAL_POSITION, "TRIM_HORIZON")
      ddbSourceProps.setProperty(ConsumerConfigConstants.SHARD_GETRECORDS_MAX, "100")

      val ddbKinesisSource = new FlinkKinesisConsumer[String](
        ddbChangeStream, new SimpleStringSchema(), ddbSourceProps
      )

      val ddbDeserializer = new DdbStreamDeserializer()
      val mappingStream = env
        .addSource(ddbKinesisSource)
        .uid("ddb-streams-source")
        .map { (json: String) => ddbDeserializer.deserialize(json.getBytes("UTF-8")) }
        .returns(TypeInformation.of(classOf[IdMappingChange]))

      val broadcastStream = mappingStream.broadcast(MeterEnrichmentFunction.ID_MAP)

      // ── Step 4: Enrichment with event-time watermarks ──

      val maxOutOfOrdernessMs = properties.getOrElse("MAX_OUT_OF_ORDERNESS_MS", "3600000").toLong
      val bufferRetentionMs = properties.getOrElse("BUFFER_RETENTION_MS", "21600000").toLong

      val watermarkStrategy = WatermarkStrategy
        .forBoundedOutOfOrderness[SensorRecord](Duration.ofMillis(maxOutOfOrdernessMs))
        .withTimestampAssigner(new SerializableTimestampAssigner[SensorRecord] {
          override def extractTimestamp(record: SensorRecord, previousTs: Long): Long =
            Extensions.parseTimestamp(record.timestamp).toEpochMilli
        })
        .withIdleness(Duration.ofHours(24))

      val watermarkedStream = parsedStream
        .assignTimestampsAndWatermarks(watermarkStrategy)
        .uid("watermark-assigner")

      val enrichedStream = watermarkedStream
        .connect(broadcastStream)
        .process(new MeterEnrichmentFunction(region, sensorIdentityTable))
        .uid("meter-enrichment")

      val deadLetters = enrichedStream.getSideOutput(SideOutputTags.DEAD_LETTER)

      // ── Step 5: Resampling (delta + interpolation per the 2026-05-01 resampling spec) ──

      val resampledStream = enrichedStream
        .keyBy((t: (EnrichedRecord, SensorMapping)) => java.lang.Integer.valueOf(t._1.logicalId))
        .process(new ResampleFunction(bufferRetentionMs))
        .uid("resample")

      val anomalies = resampledStream.getSideOutput(SideOutputTags.ANOMALY)
      val lateArrivals = resampledStream.getSideOutput(SideOutputTags.LATE_ARRIVAL)

      // ── Step 6: Enriched Iceberg sink ──

      // Note: TableSchema is deprecated by Flink but still required by Iceberg 1.7.x FlinkSink.forRow()
      val enrichedTableSchema = TableSchema.builder()
        .field("logical_id", DataTypes.INT().notNull())
        .field("timestamp", DataTypes.TIMESTAMP_WITH_LOCAL_TIME_ZONE(6).notNull())
        .field("value", DataTypes.DOUBLE().notNull())
        .field("unit", DataTypes.STRING().notNull())
        .field("ingested_time", DataTypes.TIMESTAMP_WITH_LOCAL_TIME_ZONE(6).notNull())
        .field("hn1", DataTypes.INT().notNull())
        .field("hn2", DataTypes.INT().notNull())
        .field("hn3", DataTypes.INT())
        .field("hn4", DataTypes.INT())
        .field("hn5", DataTypes.INT())
        .field("hn6", DataTypes.INT())
        .field("hn7", DataTypes.INT())
        .field("hn8", DataTypes.INT())
        .field("hn9", DataTypes.INT())
        .field("energy_type", DataTypes.STRING())
        .field("resample_value", DataTypes.DOUBLE())
        .field("resample_method", DataTypes.STRING())
        .field("resample_timestamp", DataTypes.TIMESTAMP(6))
        .build()

      val enrichedTableId = TableIdentifier.of(Namespace.of("all"), "logical_data")
      val enrichedTableLoader = TableLoader.fromCatalog(catalogLoader, enrichedTableId)

      val enrichedRowStream = resampledStream
        .map { (record: EnrichedRecord) =>
          val (normalizedUnit, factor) = Extensions.unitFactor(record.unit)
          val normalizedValue = record.value * factor
          val normalizedResampleValue: java.lang.Double =
            if record.resampleValue == null then null
            else java.lang.Double.valueOf(record.resampleValue.doubleValue() * factor)
          val resampleTs: java.time.LocalDateTime =
            if record.resampleTimestamp == null then null
            else java.time.LocalDateTime.ofInstant(
              Instant.ofEpochMilli(record.resampleTimestamp.longValue()),
              java.time.ZoneOffset.UTC
            )
          Row.of(
            java.lang.Integer.valueOf(record.logicalId),
            Extensions.parseTimestamp(record.timestamp),
            java.lang.Double.valueOf(normalizedValue),
            normalizedUnit,
            Instant.now(),
            java.lang.Integer.valueOf(record.hn1),
            java.lang.Integer.valueOf(record.hn2),
            record.hn3, record.hn4, record.hn5,
            record.hn6, record.hn7, record.hn8, record.hn9,
            record.energyType,
            normalizedResampleValue,
            record.resampleMethod,
            resampleTs
          )
        }
        .returns(Types.ROW_NAMED(
          LogicalDataSchema.columns,
          Types.INT, Types.INSTANT, Types.DOUBLE, Types.STRING, Types.INSTANT,
          Types.INT, Types.INT, Types.INT, Types.INT, Types.INT,
          Types.INT, Types.INT, Types.INT, Types.INT,
          Types.STRING, Types.DOUBLE, Types.STRING, Types.LOCAL_DATE_TIME
        ))
        .uid("enriched-row-mapper")

      FlinkSink.forRow(enrichedRowStream, enrichedTableSchema)
        .tableLoader(enrichedTableLoader)
        .distributionMode(DistributionMode.NONE)
        .writeParallelism(1)
        .append()

      // ── Step 7: Error sink ──

      if errorStreamName.nonEmpty then
        val allErrors = parseErrors.union(deadLetters).union(anomalies).union(lateArrivals)

        val errorClientProps = new Properties()
        errorClientProps.setProperty("aws.region", region)

        val errorSink = KinesisStreamsSink.builder[ErrorRecord]()
          .setKinesisClientProperties(errorClientProps)
          .setSerializationSchema(
            new org.apache.flink.api.common.serialization.SerializationSchema[ErrorRecord] {
              @transient private lazy val om = {
                val m = new ObjectMapper(); m.registerModule(DefaultScalaModule); m
              }
              override def serialize(element: ErrorRecord): Array[Byte] =
                om.writeValueAsBytes(element)
            }
          )
          .setStreamArn(toStreamArn(errorStreamName, region, accountId))
          .setPartitionKeyGenerator(new PartitionKeyGenerator[ErrorRecord] {
            override def apply(t: ErrorRecord): String = "0"
          })
          .build()

        allErrors.sinkTo(errorSink).uid("error-sink")

    else
      logger.info("Enrichment disabled: DDB_STREAM_ARN not configured")

    env.execute("DAQ Meter Enrichment Pipeline")

  def main(args: Array[String]): Unit =
    try
      createFlinkJob()
    catch
      case e: Exception =>
        logger.error(s"Error: ${e.getMessage}", e)
        throw e
