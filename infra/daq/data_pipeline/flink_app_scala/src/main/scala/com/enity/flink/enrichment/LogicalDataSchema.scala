package com.enity.flink.enrichment

/** The wire contract for `all.logical_data`.
  *
  * These names are not internal to this app: they must match, exactly and in order,
  * the Iceberg `schemaFieldList` declared for the `logical_data` table in
  * `infra/daq/data_pipeline/s3tables_stack.go`. `LogicalDataSchemaSpec` reads that
  * Go file and asserts the two agree.
  *
  * A rename that touches only Scala compiles and passes every other test — the
  * fixtures get renamed along with the code, so nothing disagrees — and then fails
  * at runtime against a table whose columns never moved. That is exactly what the
  * 2026-07-28 `purpose` → `energy_type` rename did before this guard existed.
  */
object LogicalDataSchema:
  val columns: Array[String] = Array(
    "logical_id", "timestamp", "value", "unit", "ingested_time",
    "hn1", "hn2", "hn3", "hn4", "hn5", "hn6", "hn7", "hn8", "hn9",
    "energy_type", "reading_kind"
  )

  /** The DynamoDB attribute names the bridge writes into `sensor-identity`.
    * Must match the `_item` builder in `infra/hierarchy/app.go`.
    */
  object SensorIdentityAttrs:
    val logicalId     = "logical_id"
    val readingKind   = "reading_kind"
    val energyType    = "energy_type"
    val hierarchyPath = "hierarchy_path"
    val resampleMins  = "resample_minutes"
