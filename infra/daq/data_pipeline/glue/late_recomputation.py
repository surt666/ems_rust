"""
Glue Spark job: recompute counter deltas and bins for late-arriving data.

Reads raw cumulative values from the raw_data Iceberg table, joins with
sensor-identity from DynamoDB, computes deltas via LAG() window function,
and appends corrected records to logical_data.

For meters with `resample_minutes` configured, the written timestamp/value are the resampled
ones, per the 2026-05-01 resampling spec:
  - Gauge: linear interpolation between (prev, current) for each bin in (prev_ts, current_ts]
  - Counter: time-proportional split of the delta across overlapping bins
For meters with `resample_minutes IS NULL` the reading passes through: its own timestamp and
value (delta, for counters) are written. Either way logical_data holds one value per timestamp;
raw_data keeps every untouched reading.

Event-sourcing semantics: rows are appended with ingested_time=now(), never merged.
Consumers query for the newest `ingested_time` per (logical_id, timestamp).

Parameters:
  --daq_ids              Comma-separated DAQ IDs, or "*" for full backfill
  --time_range_start     ISO-8601 start (inclusive), optional
  --time_range_end       ISO-8601 end (inclusive), optional
  --region               AWS region
  --sensor_identity_table DynamoDB table name for meter identity
  --table_bucket_name    S3 Tables bucket name
  --account_id           AWS account ID
"""

import sys
import logging
from datetime import datetime, timezone

from pyspark.sql import DataFrame
import pyspark.sql.functions as F
from pyspark.sql.window import Window
from pyspark.sql.types import (
    StructType, StructField, StringType, IntegerType, TimestampType, ArrayType, LongType,
    DoubleType,
)

logger = logging.getLogger(__name__)
logger.setLevel(logging.INFO)

# ── Helper functions ──

def parse_hierarchy_path(path: str) -> dict:
    """Parse hierarchy path "HN0#root|HN1#<int>|HN2#<int>|..." into hn1..hn9 ints.
    HN0 is the root and ignored. Sensor segment (S#<int>) must NOT be present — the sensor
    id is the sensor-identity row's logical_id, not part of hierarchy_path."""
    result: dict = {f"hn{i}": None for i in range(1, 10)}
    for segment in path.split("|"):
        if not segment or segment == "HN0#root":
            continue
        if not segment.startswith("HN") or len(segment) < 4 or segment[3] != "#":
            raise ValueError(f"Unrecognized hierarchy segment: {segment!r}")
        depth = int(segment[2])
        if depth < 1 or depth > 9:
            raise ValueError(f"Unsupported hierarchy level: {segment!r}")
        result[f"hn{depth}"] = int(segment[4:])
    return result


def parse_ddb_item(item: dict) -> dict:
    daq_id = item["sk"]["S"]
    logical_id = int(item["logical_id"]["N"])
    reading_kind = item["reading_kind"]["S"]
    hierarchy_path = item["hierarchy_path"]["S"]
    resample_field = item.get("resample_minutes")
    resample_minutes = int(resample_field["N"]) if resample_field and "N" in resample_field else None
    energy_type_field = item.get("energy_type")
    energy_type = energy_type_field["S"] if energy_type_field and "S" in energy_type_field else None
    ids = parse_hierarchy_path(hierarchy_path)
    return {
        "daq_id": daq_id, "logical_id": logical_id, "reading_kind": reading_kind,
        "resample_minutes": resample_minutes, "energy_type": energy_type, **ids,
    }


def load_sensor_identity(rgn: str, table_name: str, ids: list[str] | None) -> list[dict]:
    import boto3  # deferred: only the job talks to AWS, so the transforms stay importable

    client = boto3.client("dynamodb", region_name=rgn)
    items = []
    scan_kwargs = {"TableName": table_name}
    while True:
        response = client.scan(**scan_kwargs)
        for item in response.get("Items", []):
            try:
                record = parse_ddb_item(item)
                if ids is None or record["daq_id"] in ids:
                    items.append(record)
            except Exception as e:
                logger.warning("Failed to parse DDB item: %s", e)
        if "LastEvaluatedKey" not in response:
            break
        scan_kwargs["ExclusiveStartKey"] = response["LastEvaluatedKey"]
    logger.info("Loaded %d meter identity records from DynamoDB", len(items))
    return items


# Mirrors flink_app_scala/.../Extensions.scala UnitConversions and ResampleFunction.BinMethod.
# Both files implement the same resampling rules and must produce bit-identical output for the
# same input — change in lock-step.

_UNIT_CONVERSIONS: dict[str, tuple[str, float]] = {
    "Energy (10 Wh)":                   ("Wh", 10.0),
    "Energy (kWh)":                     ("Wh", 1000.0),
    "Energy (100 Wh)":                  ("Wh", 100.0),
    "Power (1e-1 W)":                   ("W", 1e-1),
    "Power (1e-2 W)":                   ("W", 1e-2),
    "Power (100 W)":                    ("W", 100.0),
    "W":                                ("W", 1.0),
    "VA":                               ("W", 1.0),
    "m A":                              ("A", 1e-3),
    "A":                                ("A", 1.0),
    "1e-1  V":                          ("V", 1e-1),
    "V":                                ("V", 1.0),
    "Hz":                               ("Hz", 1.0),
    "Return temperature (1e-2 deg C)":  ("C", 1e-2),
    "Flow temperature (1e-2 deg C)":    ("C", 1e-2),
    "Flow temperature (deg C)":         ("C", 1.0),
    "Volume (1e-2 m^3)":                ("m^3", 1e-2),
    "Volume flow (m m^3/h)":            ("m^3/h", 1e-3),
    "Volume (m m^3)":                   ("m^3", 1e-3),
    "WATT":                             ("W", 1.0),
    "KILO_WATT":                        ("W", 1000.0),
    "WATT_HOUR":                        ("Wh", 1.0),
    "KILO_WATT_HOUR":                   ("Wh", 1000.0),
    "CUBIC_METRE":                      ("m^3", 1.0),
    "CUBIC_METRE_HOUR":                 ("m^3/h", 1.0),
    "m3PerHour":                        ("m^3/h", 1.0),
    "m^3/h":                            ("m^3/h", 1.0),
    "DEGREE_CELSIUS":                   ("C", 1.0),
    "british-thermal-unit":             ("Wh", 0.293),
    "C":                                ("C", 1.0),
    "Celcius":                          ("C", 1.0),
    "Kelvin":                           ("K", 1.0),
    "fluid-ounce":                      ("m3", 0.000030),
    "fluid-ounce-imperial":             ("m3", 0.000028),
    "foot":                             ("m", 0.305),
    "gallon":                           ("m3", 0.003785),
    "gallon-imperial":                  ("m3", 0.004546),
    "Gcal":                             ("Wh", 1163000.0),
    "GJ":                               ("Wh", 278000.0),
    "Kg":                               ("g", 1000.0),
    "kg-Fgas":                          ("Wh", 13900.0),
    "Kg-træpiller":                     ("Wh", 4865.0),
    "km":                               ("m", 1000.0),
    "Kr.":                              ("Kr", 1.0),
    "kWh":                              ("Wh", 1000.0),
    "KWH":                              ("Wh", 1000.0),
    "Liter":                            ("m3", 0.001),
    "Liter-gasolie":                    ("Wh", 9890.0),
    "m3":                               ("m^3", 1.0),
    "m^3":                              ("m^3", 1.0),
    "m3-10Gr":                          ("Wh", 11627.910),
    "m3-25Gr":                          ("Wh", 29069.770),
    "m3-30Gr":                          ("Wh", 34883.720),
    "m3-35Gr":                          ("Wh", 40705.002),
    "m3-40Gr":                          ("Wh", 46509.998),
    "m3-5Gr":                           ("Wh", 5813.950),
    "m3-Bgas":                          ("Wh", 4380.0),
    "m3-Fgas":                          ("Wh", 34194.0),
    "m3-fjv":                           ("Wh", 34883.720),
    "m3-kond.":                         ("Wh", 700000.0),
    "m3-Ngas":                          ("Wh", 11000.0),
    "mile":                             ("m", 1609.340),
    "MJ":                               ("Wh", 278.0),
    "mm-british-thermal-unit":          ("Wh", 293071.070),
    "MWh":                              ("Wh", 1000000.0),
    "Nautical miles":                   ("m", 1.852),
    "Nm3":                              ("Nm3", 1.0),
    "Bar":                              ("bar", 1.0),
    "ounce":                            ("g", 28.0),
    "Pct":                              ("%", 1.0),
    "Percent":                          ("%", 1.0),
    "pejling":                          ("m3", 0.001),
    "pound":                            ("g", 454.0),
    "påfyldt":                          ("m3", 0.001),
    "Styk":                             ("Units", 1.0),
    "timer":                            ("s", 3600.0),
    "Ton":                              ("g", 1000000.0),
    "Ton-træpiller":                    ("Wh", 4865000.0),
    "Wh":                               ("Wh", 1.0),
    "ppm":                              ("ppm", 1.0),
    "ppb":                              ("ppb", 1.0),
    "RH%":                              ("RH%", 1.0),
    "":                                 ("EMPTY", 1.0),
}


def normalize_unit(unit: str | None) -> tuple[str, float]:
    """Return (normalized_unit, factor) for the given raw unit string. Unknown units
    pass through unchanged with factor 1.0. Mirrors Extensions.normalizeUnit in Scala."""
    if unit is None:
        return ("EMPTY", 1.0)
    if unit in _UNIT_CONVERSIONS:
        return _UNIT_CONVERSIONS[unit]
    return (unit, 1.0)


_normalize_unit_name_udf = F.udf(lambda u: normalize_unit(u)[0], StringType())
_normalize_unit_factor_udf = F.udf(lambda u: float(normalize_unit(u)[1]), DoubleType())


def enumerate_overlapping_bins(prev_ts_ms: int, current_ts_ms: int, resample_minutes: int) -> list:
    """Bin boundaries B (epoch millis) such that the bin window [B-binSize, B] overlaps
    with [prev_ts, current_ts]. Used for counter time-proportional split — energy-conservation
    requires every bin the period touches to receive a share. Matches
    ResampleFunction.enumerateOverlappingBins in the Flink operator."""
    if prev_ts_ms is None or resample_minutes is None or resample_minutes <= 0:
        return []
    if current_ts_ms <= prev_ts_ms:
        return []
    bin_size_ms = resample_minutes * 60 * 1000
    first = ((prev_ts_ms // bin_size_ms) + 1) * bin_size_ms
    last = ((current_ts_ms - 1) // bin_size_ms + 1) * bin_size_ms
    if first > last:
        return []
    out = []
    b = first
    while b <= last:
        out.append(int(b))
        b += bin_size_ms
    return out


def enumerate_bins(prev_ts_ms: int, current_ts_ms: int, resample_minutes: int) -> list:
    """Bin boundaries B (epoch millis) where prev_ts < B <= current_ts.
    Used for gauge linear interpolation — only emit bins that have been "passed" by
    the current reading."""
    if prev_ts_ms is None or resample_minutes is None or resample_minutes <= 0:
        return []
    if current_ts_ms <= prev_ts_ms:
        return []
    bin_size_ms = resample_minutes * 60 * 1000
    first = ((prev_ts_ms // bin_size_ms) + 1) * bin_size_ms
    if first > current_ts_ms:
        return []
    out = []
    b = first
    while b <= current_ts_ms:
        out.append(int(b))
        b += bin_size_ms
    return out


# Spark UDFs wrapping the bin enumerators. Each returns array<long> of epoch-millis boundaries.
enumerate_bins_udf = F.udf(enumerate_bins, ArrayType(LongType()))
enumerate_overlapping_bins_udf = F.udf(enumerate_overlapping_bins, ArrayType(LongType()))


def compute_counter_bins(joined_df: DataFrame) -> DataFrame:
    """For counter readings: compute delta, then for resampled meters fan out time-proportionally
    across overlapping bins in (prev_ts, current_ts]. Unbinned meters get one row per reading
    with delta in `value` and bin_* = NULL."""
    window = Window.partitionBy("logical_id").orderBy("timestamp")
    counters = joined_df.filter(F.col("reading_kind") == "counter")
    counters = counters.withColumn("prev_ts", F.lag("timestamp").over(window))
    counters = counters.withColumn("prev_value", F.lag("value").over(window))
    counters = counters.filter(F.col("prev_value").isNotNull())
    counters = counters.withColumn("delta", F.col("value") - F.col("prev_value"))
    counters = counters.filter(F.col("delta") >= 0)
    counters = counters.withColumn("value", F.col("delta"))

    resampled = counters.filter(F.col("resample_minutes").isNotNull())
    unresampled = counters.filter(F.col("resample_minutes").isNull()) \
        .withColumn("resample_timestamp", F.lit(None).cast(TimestampType())) \
        .withColumn("resample_value", F.lit(None).cast("double"))

    resampled = resampled.withColumn(
        "bins",
        enumerate_overlapping_bins_udf(
            (F.unix_timestamp("prev_ts") * 1000).cast("long"),
            (F.unix_timestamp("timestamp") * 1000).cast("long"),
            F.col("resample_minutes"),
        ),
    )
    resampled = resampled.withColumn("resample_timestamp_ms", F.explode("bins"))
    resampled = resampled.withColumn(
        "resample_timestamp",
        (F.col("resample_timestamp_ms") / 1000).cast(TimestampType()),
    )
    bin_size_ms = F.col("resample_minutes").cast("long") * F.lit(60 * 1000)
    prev_ts_ms = F.unix_timestamp("prev_ts") * 1000
    cur_ts_ms = F.unix_timestamp("timestamp") * 1000
    bin_start_ms = F.greatest(prev_ts_ms, F.col("resample_timestamp_ms") - bin_size_ms)
    bin_end_ms = F.least(cur_ts_ms, F.col("resample_timestamp_ms"))
    overlap_ms = bin_end_ms - bin_start_ms
    period_ms = cur_ts_ms - prev_ts_ms
    resampled = resampled.withColumn(
        "resample_value",
        F.col("delta") * (overlap_ms.cast("double") / period_ms.cast("double")),
    )
    resampled = resampled.drop("bins", "resample_timestamp_ms")

    return resampled.unionByName(unresampled, allowMissingColumns=True).drop("delta", "prev_ts", "prev_value")


def compute_gauge_bins(joined_df: DataFrame) -> DataFrame:
    """For gauge readings: for resampled meters, fan out across bins in (prev_ts, current_ts]
    with linear interpolation between (prev, current). Unbinned gauges pass through with
    bin_* = NULL. Single-reading-only meters produce no bin rows (consistent with Flink)."""
    window = Window.partitionBy("logical_id").orderBy("timestamp")
    gauges = joined_df.filter(F.col("reading_kind") == "gauge")
    gauges = gauges.withColumn("prev_ts", F.lag("timestamp").over(window))
    gauges = gauges.withColumn("prev_value", F.lag("value").over(window))

    unresampled = gauges.filter(F.col("resample_minutes").isNull()) \
        .withColumn("resample_timestamp", F.lit(None).cast(TimestampType())) \
        .withColumn("resample_value", F.lit(None).cast("double")) \
        .drop("prev_ts", "prev_value")

    resampled = gauges.filter(F.col("resample_minutes").isNotNull() & F.col("prev_ts").isNotNull())
    resampled = resampled.withColumn(
        "bins",
        enumerate_bins_udf(
            (F.unix_timestamp("prev_ts") * 1000).cast("long"),
            (F.unix_timestamp("timestamp") * 1000).cast("long"),
            F.col("resample_minutes"),
        ),
    )
    resampled = resampled.withColumn("resample_timestamp_ms", F.explode("bins"))
    resampled = resampled.withColumn(
        "resample_timestamp",
        (F.col("resample_timestamp_ms") / 1000).cast(TimestampType()),
    )
    prev_ts_ms = F.unix_timestamp("prev_ts") * 1000
    cur_ts_ms = F.unix_timestamp("timestamp") * 1000
    resampled = resampled.withColumn(
        "resample_value",
        F.col("prev_value") + (F.col("value") - F.col("prev_value")) *
        (F.col("resample_timestamp_ms") - prev_ts_ms).cast("double") /
        (cur_ts_ms - prev_ts_ms).cast("double"),
    )
    resampled = resampled.drop("bins", "resample_timestamp_ms", "prev_ts", "prev_value")

    return resampled.unionByName(unresampled, allowMissingColumns=True)


def build_output(df: DataFrame) -> DataFrame:
    now = datetime.now(timezone.utc)
    df = df.withColumn("_unit_factor", _normalize_unit_factor_udf(F.col("unit")))
    df = df.withColumn("_unit_norm", _normalize_unit_name_udf(F.col("unit")))
    # logical_data holds the resampled pair. A reading from a sensor with no resample
    # interval is its own bin: its timestamp and value (delta, for counters) go through
    # unchanged. Mirrors the `isResampled` branch in the Flink sink.
    resampled_ts = F.coalesce(F.col("resample_timestamp"), F.col("timestamp"))
    resampled_value = F.coalesce(F.col("resample_value"), F.col("value")) * F.col("_unit_factor")
    return df.select(
        F.col("logical_id"),
        resampled_ts.alias("timestamp"),
        resampled_value.alias("value"),
        F.col("_unit_norm").alias("unit"),
        F.lit(now).cast(TimestampType()).alias("ingested_time"),
        F.col("hn1").cast(IntegerType()),
        F.col("hn2").cast(IntegerType()),
        F.col("hn3").cast(IntegerType()),
        F.col("hn4").cast(IntegerType()),
        F.col("hn5").cast(IntegerType()),
        F.col("hn6").cast(IntegerType()),
        F.col("hn7").cast(IntegerType()),
        F.col("hn8").cast(IntegerType()),
        F.col("hn9").cast(IntegerType()),
        F.col("energy_type"),
        F.col("reading_kind"),
    )


# ── Entrypoint ──

def main():
    from awsglue.context import GlueContext
    from awsglue.job import Job
    from awsglue.utils import getResolvedOptions
    from pyspark.context import SparkContext

    sc = SparkContext()
    glue_context = GlueContext(sc)
    spark = glue_context.spark_session
    job = Job(glue_context)
    job.init("late-data-recomputation", {})

    required = ["JOB_NAME", "region", "sensor_identity_table", "table_bucket_name", "account_id"]
    optional = {"daq_ids": "*", "time_range_start": "", "time_range_end": ""}
    args = getResolvedOptions(sys.argv, required)
    for key, default in optional.items():
        try:
            args[key] = getResolvedOptions(sys.argv, [key])[key]
        except Exception:
            args[key] = default

    region = args["region"]
    account_id = args["account_id"]
    table_bucket_name = args["table_bucket_name"]
    raw_daq_ids = args["daq_ids"]
    daq_ids = None if raw_daq_ids == "*" else [d.strip() for d in raw_daq_ids.split(",") if d.strip()]
    time_start = args["time_range_start"]
    time_end = args["time_range_end"]

    logger.info(
        "Late recomputation starting: daq_ids=%s, time_range=[%s, %s]",
        raw_daq_ids, time_start or "-inf", time_end or "+inf",
    )

    # S3 Tables catalog config
    spark.conf.set("spark.sql.defaultCatalog", "s3tables")
    spark.conf.set("spark.sql.catalog.s3tables", "org.apache.iceberg.spark.SparkCatalog")
    spark.conf.set(
        "spark.sql.catalog.s3tables.catalog-impl",
        "org.apache.iceberg.aws.glue.GlueCatalog",
    )
    spark.conf.set("spark.sql.catalog.s3tables.glue.id",
                   f"{account_id}:s3tablescatalog/{table_bucket_name}")
    spark.conf.set("spark.sql.catalog.s3tables.warehouse",
                   f"s3://{table_bucket_name}/warehouse/")

    identity_records = load_sensor_identity(region, args["sensor_identity_table"], daq_ids)
    if not identity_records:
        logger.info("No meter identity records found, nothing to recompute")
        job.commit()
        return

    identity_schema = StructType([
        StructField("daq_id", StringType(), False),
        StructField("logical_id", IntegerType(), False),
        StructField("reading_kind", StringType(), False),
        StructField("resample_minutes", IntegerType(), True),
        StructField("energy_type", StringType(), True),
        StructField("hn1", IntegerType(), True),
        StructField("hn2", IntegerType(), True),
        StructField("hn3", IntegerType(), True),
        StructField("hn4", IntegerType(), True),
        StructField("hn5", IntegerType(), True),
        StructField("hn6", IntegerType(), True),
        StructField("hn7", IntegerType(), True),
        StructField("hn8", IntegerType(), True),
        StructField("hn9", IntegerType(), True),
    ])
    identity_df = spark.createDataFrame(identity_records, schema=identity_schema)

    raw_df = spark.table("all.raw_data")
    if daq_ids is not None:
        raw_df = raw_df.filter(F.col("daq_id").isin(daq_ids))
    if time_start:
        raw_df = raw_df.filter(F.col("timestamp") >= F.lit(time_start).cast(TimestampType()))
    if time_end:
        raw_df = raw_df.filter(F.col("timestamp") <= F.lit(time_end).cast(TimestampType()))

    record_count = raw_df.count()
    logger.info("Read %d raw records from raw_data", record_count)

    if record_count > 0:
        joined_df = raw_df.join(identity_df, on="daq_id", how="inner")
        result_df = compute_counter_bins(joined_df).unionByName(
            compute_gauge_bins(joined_df), allowMissingColumns=True)
        output_df = build_output(result_df)

        output_count = output_df.count()
        logger.info("Writing %d recomputed records to logical_data", output_count)

        if output_count > 0:
            # Collect and re-create DataFrame to sever lineage to raw_data's S3 Tables
            # managed bucket. writeTo().append() fails with S3 403 when the execution
            # plan references a different S3 Tables managed bucket (Glue 5.0 credential
            # vending issue).
            output_schema = output_df.schema
            output_rows = output_df.collect()
            spark.createDataFrame(output_rows, output_schema).writeTo("all.logical_data").append()
    else:
        logger.info("No raw records found for the given filters")

    logger.info("Late recomputation complete")
    job.commit()


if __name__ == "__main__":
    main()
