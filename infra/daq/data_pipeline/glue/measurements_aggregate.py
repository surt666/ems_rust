"""
Glue Spark job: aggregate counter consumption from logical_meter_data into the
measurements_aggregate DynamoDB materialized view (per node / purpose / granularity / bucket).
See docs/superpowers/specs/2026-06-07-measurements-rollup-view-design.md.
"""

from datetime import datetime, timezone, timedelta

HOUR_TTL_DAYS = 90
DAY_TTL_DAYS = 730


def hour_bucket(ts: datetime) -> str:
    """UTC hour-bucket label 'YYYY-MM-DDThh'."""
    return ts.astimezone(timezone.utc).strftime("%Y-%m-%dT%H")


def day_bucket(ts: datetime) -> str:
    """UTC day-bucket label 'YYYY-MM-DD'."""
    return ts.astimezone(timezone.utc).strftime("%Y-%m-%d")


def _bucket_end_epoch(gran: str, bucket: str) -> int:
    """Epoch seconds at the END of a bucket window (UTC)."""
    if gran == "h":
        start = datetime.strptime(bucket, "%Y-%m-%dT%H").replace(tzinfo=timezone.utc)
        end = start + timedelta(hours=1)
    else:
        start = datetime.strptime(bucket, "%Y-%m-%d").replace(tzinfo=timezone.utc)
        end = start + timedelta(days=1)
    return int(end.timestamp())


def ttl_for(gran: str, bucket: str) -> int:
    """DynamoDB TTL (epoch seconds): bucket-end + retention (90d hourly / 730d daily)."""
    days = HOUR_TTL_DAYS if gran == "h" else DAY_TTL_DAYS
    return _bucket_end_epoch(gran, bucket) + days * 86400


def ancestor_keys(hns, logical_id):
    """hns = [hn2, hn3, ..., hn9] (ints or None). Returns the full hierarchy node_path for every
    populated level from hn2 down, plus the leaf meter. Paths are '|'-joined hierarchy segments,
    consistent with the rest of the hierarchy: company = 'HN2#<id>', deeper nodes append
    '|HN<d>#<id>', and the leaf meter appends '|L#<logical_id>'."""
    segments = ["HN2#%d" % hns[0]]
    result = [segments[0]]
    for depth, hid in enumerate(hns[1:], start=3):  # hn3..hn9
        # Levels are dense depth indices, not fixed type slots: each company's schema assigns a
        # type to each consecutive level (e.g. hn3=group|property, hn4=building, hn5=area), and a
        # populated path is contiguous from hn2 with only trailing nulls — a building always has
        # its hn3 parent. The first None therefore ends the chain; there is no later populated
        # level to recover.
        if hid is None:
            break
        segments.append("HN%d#%d" % (depth, hid))
        result.append("|".join(segments))
    result.append("|".join(segments + ["L#%d" % logical_id]))
    return result


def build_sk(node_path: str, purpose: str, gran: str, bucket: str) -> str:
    """sk = '<node_path>#<purpose>#<gran>#<bucket>'. The '#' after node_path is the delimiter
    that keeps a node's own rows sorting before its descendants' ('|' > '#')."""
    return "%s#%s#%s#%s" % (node_path, purpose, gran, bucket)


# ── Spark transform ──

from pyspark.sql import DataFrame, functions as F, types as T  # noqa: E402
from pyspark.sql.window import Window  # noqa: E402

_ANCESTOR_SCHEMA = T.ArrayType(T.StringType())


@F.udf(_ANCESTOR_SCHEMA)
def _ancestor_keys_udf(hn2, hn3, hn4, hn5, hn6, hn7, hn8, hn9, logical_id):
    return ancestor_keys([hn2, hn3, hn4, hn5, hn6, hn7, hn8, hn9], logical_id)


_TTL_UDF = F.udf(ttl_for, T.LongType())
_SK_UDF = F.udf(build_sk, T.StringType())


def build_rollups(df: DataFrame, run_at_iso: str) -> DataFrame:
    """Aggregate counter rows into per-node/purpose/granularity/bucket rollup items.

    Input columns: hn2..hn9 (int), logical_id (int), purpose (str), unit (str),
    resample_value (double), value (double), timestamp (ts), resample_timestamp (ts).
    Output columns: pk ('HN2#<id>'), sk ('<full hierarchy path>#<purpose>#<gran>#<bucket>'),
    purpose, unit, sum, count, min, max, last_value, last_ts, updated_at, ttl.
    (`bucket` is intentionally NOT written as its own attribute — the date is the
    last sort-key segment, so the read side ranges on the SK directly.)
    NOTE: caller must set spark.sql.session.timeZone='UTC' so the bucket labels are UTC.
    """
    with_buckets = df.withColumn(
        "gb",
        F.explode(F.array(
            F.struct(F.lit("h").alias("gran"),
                     F.date_format(F.col("resample_timestamp"), "yyyy-MM-dd'T'HH").alias("bucket")),
            F.struct(F.lit("d").alias("gran"),
                     F.date_format(F.col("resample_timestamp"), "yyyy-MM-dd").alias("bucket")),
        )),
    ).select("*", F.col("gb.gran").alias("gran"), F.col("gb.bucket").alias("bucket"))

    with_nodes = with_buckets.withColumn(
        "node_path",
        F.explode(_ancestor_keys_udf(
            *[F.col("hn%d" % i) for i in range(2, 10)], F.col("logical_id"))),
    )

    grouped = with_nodes.groupBy(
        "hn2", "node_path", "purpose", "gran", "bucket"
    ).agg(
        F.sum("resample_value").alias("sum"),
        F.count("resample_value").alias("count"),
        F.min("resample_value").alias("min"),
        F.max("resample_value").alias("max"),
        F.max(F.struct(F.col("timestamp"), F.col("value"))).alias("_last"),
        F.max("unit").alias("unit"),
    )

    return grouped.select(
        F.concat(F.lit("HN2#"), F.col("hn2").cast("string")).alias("pk"),
        _SK_UDF("node_path", "purpose", "gran", "bucket").alias("sk"),
        "purpose", "unit", "sum", "count", "min", "max",
        F.col("_last.value").alias("last_value"),
        F.date_format(F.col("_last.timestamp"), "yyyy-MM-dd'T'HH:mm:ssXXX").alias("last_ts"),
        F.lit(run_at_iso).alias("updated_at"),
        _TTL_UDF("gran", "bucket").alias("ttl"),
    )


# ── IO + entrypoint ──

def window_start_iso(now: datetime, lookback_days: int) -> str:
    """00:00 UTC of (today - lookback_days)."""
    start_day = (now.astimezone(timezone.utc) - timedelta(days=lookback_days)).date()
    return datetime(start_day.year, start_day.month, start_day.day, tzinfo=timezone.utc) \
        .strftime("%Y-%m-%dT%H:%M:%S+00:00")


def latest_counters(df: DataFrame) -> DataFrame:
    """logical_meter_data is event-sourced (append-only): for a given
    (logical_id, resample_timestamp) the newest ingested_time row supersedes older ones. Keep only
    that newest row per point, then filter to resampled counters that carry a company id.
    Input must include ingested_time and resample_method (plus the rollup columns)."""
    newest = Window.partitionBy("logical_id", "resample_timestamp") \
        .orderBy(F.col("ingested_time").desc())
    return (
        df.withColumn("_rn", F.row_number().over(newest))
        .filter((F.col("_rn") == 1)
                & (F.col("resample_method") == "time_proportional")
                & F.col("resample_value").isNotNull()
                & F.col("hn2").isNotNull())
        .drop("_rn")
    )


def read_counters(spark, window_start: str):
    """Newest-ingested resampled counter rows for buckets at/after window_start.

    Windows by resample_timestamp (the bucket axis) so whole hour/day buckets are recomputed from
    all their points, and dedups to the newest ingested_time per (logical_id, resample_timestamp)
    — matching how every consumer reads the event-sourced logical_meter_data table. Restatements of
    points whose resample_timestamp is older than the window are not picked up (documented hook;
    widen --lookback_days to recompute them)."""
    raw = spark.sql(f"""
        SELECT hn2, hn3, hn4, hn5, hn6, hn7, hn8, hn9, logical_id, purpose, unit,
               resample_value, value, timestamp, resample_timestamp,
               resample_method, ingested_time
        FROM all.logical_meter_data
        WHERE resample_timestamp >= TIMESTAMP '{window_start}'
    """)
    return latest_counters(raw).select(
        "hn2", "hn3", "hn4", "hn5", "hn6", "hn7", "hn8", "hn9", "logical_id", "purpose", "unit",
        "resample_value", "value", "timestamp", "resample_timestamp")


def write_to_dynamo(df: DataFrame, table_name: str, region: str) -> None:
    """Upsert rollup items into DynamoDB, partition-parallel. PutItem overwrites (idempotent)."""
    from decimal import Decimal

    cols = df.columns

    def _write(rows):
        import boto3
        table = boto3.resource("dynamodb", region_name=region).Table(table_name)
        with table.batch_writer(overwrite_by_pkeys=["pk", "sk"]) as bw:
            for r in rows:
                item = {}
                for c in cols:
                    v = r[c]
                    if v is None:
                        continue
                    item[c] = Decimal(str(v)) if isinstance(v, float) else v
                bw.put_item(Item=item)

    df.foreachPartition(_write)


def main():
    import sys
    from awsglue.context import GlueContext
    from awsglue.job import Job
    from awsglue.utils import getResolvedOptions
    from pyspark.context import SparkContext

    sc = SparkContext()
    glue_context = GlueContext(sc)
    spark = glue_context.spark_session
    job = Job(glue_context)
    job.init("measurements-aggregate", {})

    required = ["JOB_NAME", "region", "table_bucket_name", "account_id", "rollup_table"]
    args = getResolvedOptions(sys.argv, required)
    try:
        args["lookback_days"] = getResolvedOptions(sys.argv, ["lookback_days"])["lookback_days"]
    except Exception:
        args["lookback_days"] = "1"

    region = args["region"]
    table_bucket = args["table_bucket_name"]
    account_id = args["account_id"]
    warehouse = f"arn:aws:s3tables:{region}:{account_id}:bucket/{table_bucket}"
    glue_id = f"{account_id}:s3tablescatalog/{table_bucket}"

    spark.conf.set("spark.sql.session.timeZone", "UTC")
    spark.conf.set("spark.sql.defaultCatalog", "s3tables")
    spark.conf.set("spark.sql.catalog.s3tables", "org.apache.iceberg.spark.SparkCatalog")
    spark.conf.set("spark.sql.catalog.s3tables.catalog-impl", "org.apache.iceberg.aws.glue.GlueCatalog")
    spark.conf.set("spark.sql.catalog.s3tables.glue.id", glue_id)
    spark.conf.set("spark.sql.catalog.s3tables.warehouse", warehouse)

    now = datetime.now(timezone.utc)
    window_start = window_start_iso(now, int(args["lookback_days"]))
    rollups = build_rollups(read_counters(spark, window_start), now.strftime("%Y-%m-%dT%H:%M:%S+00:00"))
    write_to_dynamo(rollups, args["rollup_table"], region)
    job.commit()


if __name__ == "__main__":
    main()
