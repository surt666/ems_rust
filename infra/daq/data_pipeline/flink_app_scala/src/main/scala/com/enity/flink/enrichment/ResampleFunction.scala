package com.enity.flink.enrichment

import com.enity.flink.utils.Extensions
import org.apache.flink.api.common.state.{MapState, MapStateDescriptor, ValueState, ValueStateDescriptor}
import org.apache.flink.api.common.typeinfo.{TypeInformation, Types}
import org.apache.flink.configuration.Configuration
import org.apache.flink.streaming.api.functions.KeyedProcessFunction
import org.apache.flink.util.Collector
import org.slf4j.LoggerFactory

import java.time.Instant
import scala.jdk.CollectionConverters.*

/** Resamples irregular readings onto a fixed time grid. Keyed by logicalId, holds one-reading
  * lag per meter and emits one row per grid point when the next reading arrives; the grid
  * point and its value become the `timestamp` / `value` written to `logical_data`. See
  * `docs/superpowers/specs/2026-05-01-resampling-rules-design.md`.
  *
  * (Internal algorithm vocabulary still says "bin" for the grid points/windows — e.g.
  * `binSizeMs`, `computeBins` — but nothing named "bin" is persisted or part of any contract.) */
class ResampleFunction(bufferRetentionMs: Long = 6 * 3600 * 1000L)
    extends KeyedProcessFunction[Integer, (EnrichedRecord, SensorMapping), EnrichedRecord]:

  @transient private lazy val logger = LoggerFactory.getLogger(getClass)

  /** Buffer: event timestamp millis → BufferedReadingV2.
    * Carries the EnrichedRecord plus its mapping (readingKind + resampleMinutes). */
  @transient private var readingBuffer: MapState[java.lang.Long, BufferedReadingV2] = _

  /** Track the latest timestamp for which we've emitted bins, to avoid re-emission. */
  @transient private var lastEmittedTs: ValueState[java.lang.Long] = _

  /** Largest event-time millis currently in the buffer. Lets the in-order fast path
    * skip the O(N) scan of `findPredecessor` — the predecessor is just this entry. */
  @transient private var latestBufferedTs: ValueState[java.lang.Long] = _

  override def open(parameters: Configuration): Unit =
    readingBuffer = getRuntimeContext.getMapState(
      new MapStateDescriptor[java.lang.Long, BufferedReadingV2](
        "resample-reading-buffer",
        Types.LONG.asInstanceOf[TypeInformation[java.lang.Long]],
        TypeInformation.of(classOf[BufferedReadingV2])
      )
    )
    lastEmittedTs = getRuntimeContext.getState(
      new ValueStateDescriptor[java.lang.Long](
        "resample-last-emitted-ts",
        Types.LONG.asInstanceOf[TypeInformation[java.lang.Long]]
      )
    )
    latestBufferedTs = getRuntimeContext.getState(
      new ValueStateDescriptor[java.lang.Long](
        "resample-latest-buffered-ts",
        Types.LONG.asInstanceOf[TypeInformation[java.lang.Long]]
      )
    )

  override def processElement(
      value: (EnrichedRecord, SensorMapping),
      ctx: KeyedProcessFunction[Integer, (EnrichedRecord, SensorMapping), EnrichedRecord]#Context,
      out: Collector[EnrichedRecord]
  ): Unit =
    val (record, mapping) = value

    // Backward-compat fast path for unconfigured meters (resampleMinutes=null):
    // gauges pass through immediately; counters fall through to the buffer-and-delta path.
    if mapping.resampleMinutes == null && mapping.readingKind == "gauge" then
      out.collect(record)
      return

    val eventTs = Extensions.parseTimestamp(record.timestamp).toEpochMilli
    val priorLatest = Option(latestBufferedTs.value()).fold(Long.MinValue)(_.longValue())
    readingBuffer.put(eventTs, BufferedReadingV2(record.value, record, mapping))
    if eventTs > priorLatest then latestBufferedTs.update(eventTs)
    emitFromBuffer(eventTs, priorLatest, ctx, out)
    ctx.timerService().registerEventTimeTimer(eventTs)

  override def onTimer(
      timerTs: Long,
      ctx: KeyedProcessFunction[Integer, (EnrichedRecord, SensorMapping), EnrichedRecord]#OnTimerContext,
      out: Collector[EnrichedRecord]
  ): Unit =
    val entries = sortedEntries
    if entries.nonEmpty then
      val lastEmitted = Option(lastEmittedTs.value()).fold(Long.MinValue)(_.longValue())
      val currentWatermark = ctx.timerService().currentWatermark()

      val newLastEmitted = entries.sliding(2).foldLeft(lastEmitted) {
        case (acc, Seq((prevTs, prev), (ts, current))) if ts > acc && ts <= currentWatermark =>
          emitBins(prev, current, prevTs, ts, ctx, out)
          math.max(acc, ts)
        case (acc, _) => acc
      }

      lastEmittedTs.update(newLastEmitted)
      purgeOldEntries(entries, currentWatermark)

  /** Immediately compute and emit bins for `eventTs` against its predecessor.
    * Handles backfill/historical streams where the watermark may jump past all timestamps
    * and timers never fire. If the predecessor was purged, route to LATE_ARRIVAL.
    *
    * `priorLatest` is the largest buffered ts BEFORE the current event was inserted.
    * In the in-order common case `priorLatest < eventTs` and is the predecessor — no scan needed. */
  private def emitFromBuffer(
      eventTs: Long,
      priorLatest: Long,
      ctx: KeyedProcessFunction[Integer, (EnrichedRecord, SensorMapping), EnrichedRecord]#Context,
      out: Collector[EnrichedRecord]
  ): Unit =
    val current = readingBuffer.get(eventTs)
    val predecessor =
      if priorLatest != Long.MinValue && priorLatest < eventTs then
        Some((priorLatest, readingBuffer.get(priorLatest)))
      else
        findPredecessor(eventTs)

    predecessor match
      case Some((prevTs, prev)) =>
        emitBins(prev, current, prevTs, eventTs, ctx, out)
        val lastEmitted = Option(lastEmittedTs.value()).fold(Long.MinValue)(_.longValue())
        if eventTs > lastEmitted then
          lastEmittedTs.update(eventTs)

      case None =>
        val lastEmitted = Option(lastEmittedTs.value()).fold(Long.MinValue)(_.longValue())
        if lastEmitted > Long.MinValue then
          logger.info(s"Late arrival for ${ctx.getCurrentKey} at ${Instant.ofEpochMilli(eventTs)} — predecessor purged")
          ctx.output(SideOutputTags.LATE_ARRIVAL, ErrorRecord(
            errorType = "late_arrival",
            timestamp = current.record.timestamp,
            daqId = ctx.getCurrentKey.toString,
            payload = s"value=${current.record.value}",
            error = "No predecessor in buffer — needs batch recomputation"
          ))

  /** Compute and emit bin rows between two readings. Branches on readingKind inside computeBins. */
  private def emitBins(
      prev: BufferedReadingV2,
      current: BufferedReadingV2,
      prevTs: Long,
      currentTs: Long,
      ctx: KeyedProcessFunction[Integer, (EnrichedRecord, SensorMapping), EnrichedRecord]#Context,
      out: Collector[EnrichedRecord]
  ): Unit =
    val mapping = current.mapping

    // Counter with resampleMinutes=null: preserve old CounterDeltaFunction behavior — emit one
    // row per (prev, current) pair with value=delta, resample_* fields null.
    if mapping.resampleMinutes == null then
      mapping.readingKind match
        case "counter" =>
          if current.cumulativeValue < prev.cumulativeValue then
            ctx.output(SideOutputTags.ANOMALY, ErrorRecord(
              errorType = "anomaly",
              timestamp = current.record.timestamp,
              daqId = ctx.getCurrentKey.toString,
              payload = s"value=${current.cumulativeValue}, previous=${prev.cumulativeValue}",
              error = s"Negative counter delta: ${current.cumulativeValue} - ${prev.cumulativeValue}"
            ))
          else
            out.collect(current.record.copy(value = current.cumulativeValue - prev.cumulativeValue))
        case _ =>
          out.collect(current.record)
      return

    ResampleFunction.computeBins(prev, current, prevTs, currentTs, mapping) match
      case ResampleFunction.Anomaly =>
        ctx.output(SideOutputTags.ANOMALY, ErrorRecord(
          errorType = "anomaly",
          timestamp = current.record.timestamp,
          daqId = ctx.getCurrentKey.toString,
          payload = s"value=${current.cumulativeValue}, previous=${prev.cumulativeValue}",
          error = s"Negative counter delta: ${current.cumulativeValue} - ${prev.cumulativeValue}"
        ))
      case ResampleFunction.Bins(rows) =>
        rows.foreach(out.collect)

  /** Find the entry with the largest timestamp strictly less than the given timestamp.
    * Linear scan over MapState — same approach as the prior CounterDeltaFunction. */
  private def findPredecessor(eventTs: Long): Option[(Long, BufferedReadingV2)] =
    var best: (Long, BufferedReadingV2) = null
    readingBuffer.entries().iterator().asScala.foreach { e =>
      val ts = e.getKey.longValue()
      if ts < eventTs && (best == null || ts > best._1) then
        best = (ts, e.getValue)
    }
    Option(best)

  private def purgeOldEntries(entries: Seq[(Long, BufferedReadingV2)], currentWatermark: Long): Unit =
    val purgeThreshold = currentWatermark - bufferRetentionMs
    val toPurge = entries.collect { case (ts, _) if ts < purgeThreshold => ts }
    if toPurge.nonEmpty then
      val keep = toPurge.max
      toPurge.filter(_ != keep).foreach(ts => readingBuffer.remove(ts))

  private def sortedEntries: Seq[(Long, BufferedReadingV2)] =
    readingBuffer.entries().iterator().asScala
      .map(e => (e.getKey.longValue(), e.getValue))
      .toSeq
      .sortBy(_._1)

object ResampleFunction:

  /** Which rule produced a grid point. Provenance only — `logical_data` has no such column,
    * it holds one value per timestamp however that value was arrived at. Kept because it is
    * the only thing that tells the counter path from the gauge path in a test. */
  object BinMethod:
    val LinearInterpolation = "linear_interpolation"
    val TimeProportional = "time_proportional"
    val NearestNeighbor = "nearest_neighbor"

  sealed trait Result
  case object Anomaly extends Result
  case class Bins(rows: Seq[EnrichedRecord]) extends Result

  def computeBins(
      prev: BufferedReadingV2,
      current: BufferedReadingV2,
      prevTs: Long,
      currentTs: Long,
      mapping: SensorMapping
  ): Result =
    val binSizeMs = mapping.resampleMinutes.intValue().toLong * 60L * 1000L

    mapping.readingKind match
      case "counter" =>
        if current.cumulativeValue < prev.cumulativeValue then Anomaly
        else
          val delta = current.cumulativeValue - prev.cumulativeValue
          val totalPeriod = (currentTs - prevTs).toDouble
          val rows = enumerateOverlappingBins(prevTs, currentTs, binSizeMs).map { b =>
            val binStart = math.max(prevTs, b - binSizeMs)
            val binEnd = math.min(currentTs, b)
            val overlap = (binEnd - binStart).toDouble
            val binValue = if totalPeriod <= 0 then 0.0 else delta * (overlap / totalPeriod)
            current.record.copy(
              value = delta,
              resampleTimestamp = java.lang.Long.valueOf(b),
              resampleValue = java.lang.Double.valueOf(binValue),
              resampleMethod = BinMethod.TimeProportional
            )
          }
          Bins(rows)

      case "gauge" =>
        val totalPeriod = (currentTs - prevTs).toDouble
        val rows = enumerateBinsIn(prevTs, currentTs, binSizeMs).map { b =>
          val binValue =
            if totalPeriod <= 0 then current.cumulativeValue
            else prev.cumulativeValue + (current.cumulativeValue - prev.cumulativeValue) * (b - prevTs) / totalPeriod
          current.record.copy(
            resampleTimestamp = java.lang.Long.valueOf(b),
            resampleValue = java.lang.Double.valueOf(binValue),
            resampleMethod = BinMethod.LinearInterpolation
          )
        }
        Bins(rows)

      case _ =>
        Bins(Seq(current.record))

  /** Bin boundaries B with prevTs < B <= currentTs, on the grid (multiples of binSizeMs from epoch). */
  def enumerateBinsIn(prevTs: Long, currentTs: Long, binSizeMs: Long): Seq[Long] =
    if binSizeMs <= 0 || currentTs <= prevTs then Seq.empty
    else
      val first = ((prevTs / binSizeMs) + 1) * binSizeMs
      if first > currentTs then Seq.empty
      else (first to currentTs by binSizeMs).toSeq

  /** Bin boundaries B whose window [B-binSizeMs, B] overlaps [prevTs, currentTs]. May extend
    * past currentTs when the period straddles a bin boundary — required for energy conservation. */
  def enumerateOverlappingBins(prevTs: Long, currentTs: Long, binSizeMs: Long): Seq[Long] =
    if binSizeMs <= 0 || currentTs <= prevTs then Seq.empty
    else
      val first = ((prevTs / binSizeMs) + 1) * binSizeMs
      val last = ((currentTs - 1) / binSizeMs + 1) * binSizeMs
      if first > last then Seq.empty
      else (first to last by binSizeMs).toSeq
