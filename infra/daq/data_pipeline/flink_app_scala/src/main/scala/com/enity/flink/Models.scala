package com.enity.flink

/**
 * Case class representing a standardized sensor record.
 * This is the output format produced by all transformers.
 */
case class SensorRecord(
  daqId: String,
  `type`: String,
  gatewayId: String,
  meterId: String,
  timestamp: String,
  ingestedTime: String,
  sensorId: String,
  value: String,
  unit: String
)
