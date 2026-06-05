package com.enity.flink.enrichment

import org.apache.flink.api.common.typeinfo.TypeInformation
import org.apache.flink.util.OutputTag

object SideOutputTags:
  private def errorTag(id: String): OutputTag[ErrorRecord] =
    new OutputTag[ErrorRecord](id, TypeInformation.of(classOf[ErrorRecord]))

  val PARSE_ERROR: OutputTag[ErrorRecord] = errorTag("parse-error")
  val DEAD_LETTER: OutputTag[ErrorRecord] = errorTag("dead-letter")
  val ANOMALY: OutputTag[ErrorRecord] = errorTag("anomaly")
  val LATE_ARRIVAL: OutputTag[ErrorRecord] = errorTag("late-arrival")
