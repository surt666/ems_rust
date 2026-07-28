package com.enity.flink.enrichment

import org.slf4j.LoggerFactory
import software.amazon.awssdk.services.dynamodb.DynamoDbClient
import software.amazon.awssdk.services.dynamodb.model.{AttributeValue, ScanRequest}

import java.util.concurrent.{ConcurrentHashMap, Executors, TimeUnit}
import scala.annotation.tailrec
import scala.jdk.CollectionConverters.*

object DdbBootstrapLoader:
  private val logger = LoggerFactory.getLogger(getClass)
  private val NUM_PARTITIONS = 20_000
  private val SCAN_SEGMENTS = 20

  def zeroPad(n: Int): String = f"$n%05d"

  def partitionKey(daqId: String): String =
    zeroPad(Math.abs(daqId.hashCode) % NUM_PARTITIONS)

  def parseDdbItem(item: java.util.Map[String, AttributeValue]): (String, SensorMapping) =
    val daqId = item.get("sk").s()
    val logicalId = item.get("logical_id").n().toInt
    val readingKind = item.get(LogicalDataSchema.SensorIdentityAttrs.readingKind).s()
    val hierarchyPath = item.get("hierarchy_path").s()
    val ids = HierarchyPathParser.parse(hierarchyPath)
    val energyType = if item.containsKey(LogicalDataSchema.SensorIdentityAttrs.energyType) then
        item.get(LogicalDataSchema.SensorIdentityAttrs.energyType).s() else ""
    val resampleMinutes: java.lang.Integer =
      if item.containsKey("resample_minutes") && item.get("resample_minutes").n() != null then
        Integer.valueOf(item.get("resample_minutes").n().toInt)
      else null

    val mapping = SensorMapping(
      logicalId = logicalId,
      readingKind = readingKind,
      hn1 = ids.hn1, hn2 = ids.hn2,
      hn3 = ids.hn3, hn4 = ids.hn4, hn5 = ids.hn5,
      hn6 = ids.hn6, hn7 = ids.hn7, hn8 = ids.hn8, hn9 = ids.hn9,
      energyType = energyType,
      resampleMinutes = resampleMinutes
    )
    (daqId, mapping)

  /** Parallel DynamoDB Scan to load all meter mappings. Uses parallel segments for throughput. */
  def loadAll(client: DynamoDbClient, tableName: String): Map[String, SensorMapping] =
    val startTime = System.currentTimeMillis()
    val buffer = new ConcurrentHashMap[String, SensorMapping]()
    val executor = Executors.newFixedThreadPool(SCAN_SEGMENTS)

    val futures = (0 until SCAN_SEGMENTS).map { segment =>
      val task: Runnable = () => scanSegment(client, tableName, segment, buffer)
      executor.submit(task)
    }

    futures.foreach(f => f.get())
    executor.shutdown()
    executor.awaitTermination(5, TimeUnit.MINUTES)

    val elapsed = System.currentTimeMillis() - startTime
    logger.info(s"Bootstrap loaded ${buffer.size()} meter mappings in ${elapsed}ms")
    buffer.asScala.toMap

  @tailrec
  private def scanSegment(
      client: DynamoDbClient,
      tableName: String,
      segment: Int,
      buffer: ConcurrentHashMap[String, SensorMapping],
      exclusiveStartKey: Option[java.util.Map[String, AttributeValue]] = None
  ): Unit =
    val requestBuilder = ScanRequest.builder()
      .tableName(tableName)
      .segment(segment)
      .totalSegments(SCAN_SEGMENTS)

    exclusiveStartKey.foreach(key => requestBuilder.exclusiveStartKey(key))

    val response = client.scan(requestBuilder.build())
    response.items().asScala.foreach { item =>
      try
        val (daqId, mapping) = parseDdbItem(item)
        buffer.put(daqId, mapping)
      catch
        case e: Exception =>
          logger.warn(s"Failed to parse DDB item: ${e.getMessage}")
    }

    if response.hasLastEvaluatedKey then
      scanSegment(client, tableName, segment, buffer, Some(response.lastEvaluatedKey()))
