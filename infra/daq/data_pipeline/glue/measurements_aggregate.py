"""
Glue Spark job: aggregate counter consumption from logical_data into the
measurements_aggregate DynamoDB materialized view
(per node / energy_type / purpose / granularity / bucket).

value(node, energy_type, purpose) = SUM(coefficient * reading), where the coefficients
come pre-flattened from crates/model. There is no recursion here, no default handling,
no derived-node detection and no Unallocated subtraction: ancestry and every formula
rule are already baked into the matrix, so this is one join and one grouped sum.

Specs: docs/superpowers/specs/2026-07-28-node-formula-rollup-design.md §8
       docs/superpowers/specs/2026-06-07-measurements-rollup-view-design.md
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


def build_sk(node_path: str, energy_type: str, purpose: str, gran: str, bucket: str) -> str:
    """sk = '<node_path>#<energy_type>#<purpose>#<gran>#<bucket>'.

    The bucket stays LAST so a fixed (energy_type, purpose) is a pure BETWEEN key-range.
    The '#' after node_path keeps a node's own rows sorting before its descendants'
    ('|' > '#')."""
    return "%s#%s#%s#%s#%s" % (node_path, energy_type, purpose, gran, bucket)


def build_gsi1pk(hn2: int, dimension: str, purpose: str) -> str:
    """gsi1pk = 'HN2#<id>#<dimension>#<purpose>'.

    Per-purpose, so a cross-energy_type "all energy" query cannot accidentally sum a
    node's total together with its own purpose breakdown."""
    return "HN2#%d#%s#%s" % (hn2, dimension, purpose)


# Energy carriers roll up together (Wh/J), volumes together (m³/L). The GSI
# partitions by this dimension so a company-wide "all energy" view is one query
# that sums across electricity + heat + gas. Unknown units get their own bucket
# rather than being silently folded into energy.
def dimension_of_unit(unit: str) -> str:
    """Aggregation dimension from a unit: 'energy' | 'volume' | 'other'."""
    u = (unit or "").strip().lower()
    if "wh" in u or u in ("j", "kj", "mj", "gj"):
        return "energy"
    if "m3" in u or "m³" in u or u in ("l", "liter", "litre", "litres"):
        return "volume"
    return "other"


def build_gsi1sk(node_path: str, gran: str, bucket: str) -> str:
    """gsi1sk = '<node_path>#<gran>#<bucket>' — energy_type omitted, so a dimension
    partition ranges across every energy type (and node) by time."""
    return "%s#%s#%s" % (node_path, gran, bucket)


# ── Spark transform ──

from pyspark.sql import DataFrame, functions as F, types as T  # noqa: E402
from pyspark.sql.window import Window  # noqa: E402

_TTL_UDF = F.udf(ttl_for, T.LongType())
_SK_UDF = F.udf(build_sk, T.StringType())
_DIM_UDF = F.udf(dimension_of_unit, T.StringType())
_GSI1PK_UDF = F.udf(build_gsi1pk, T.StringType())
_GSI1SK_UDF = F.udf(build_gsi1sk, T.StringType())

_MATRIX_SCHEMA = T.StructType([
    T.StructField("node_path", T.StringType()),
    T.StructField("m_energy_type", T.StringType()),
    T.StructField("purpose", T.StringType()),
    T.StructField("m_logical_id", T.IntegerType()),
    T.StructField("coefficient", T.DoubleType()),
])


def build_rollups(df: DataFrame, matrix: list, run_at_iso: str) -> DataFrame:
    """value(node, energy_type, purpose) = SUM(coefficient x reading).

    `matrix` is the flattened coefficient matrix for the companies present in `df`
    (see hierarchy_matrix.load_matrix). It already encodes ancestry, both defaults and
    the Unallocated rows, so this is one broadcast join and one grouped sum — see
    spec 2026-07-28-node-formula-rollup-design.md §8.2.

    Input columns: hn2 (int), logical_id (int), energy_type (str), unit (str),
    value (double), timestamp (ts).
    Output columns: pk ('HN2#<id>'), sk, gsi1pk, gsi1sk, energy_type, purpose, unit,
    sum, count, updated_at, ttl.

    A sensor joins on (logical_id, energy_type): the matrix row carries the energy type
    the formula was declared for, so a sensor cannot contribute to a series of a
    different type even if some other node names it.

    NOTE: caller must set spark.sql.session.timeZone='UTC' so bucket labels are UTC.
    """
    spark = df.sparkSession

    with_buckets = df.withColumn(
        "gb",
        F.explode(F.array(
            F.struct(F.lit("h").alias("gran"),
                     F.date_format(F.col("timestamp"), "yyyy-MM-dd'T'HH").alias("bucket")),
            F.struct(F.lit("d").alias("gran"),
                     F.date_format(F.col("timestamp"), "yyyy-MM-dd").alias("bucket")),
        )),
    ).select("*", F.col("gb.gran").alias("gran"), F.col("gb.bucket").alias("bucket"))

    m_df = spark.createDataFrame(
        [(r["node_path"], r["energy_type"], r["purpose"],
          int(r["sensor_id"]), float(r["coefficient"])) for r in matrix],
        _MATRIX_SCHEMA)

    grouped = (
        with_buckets
        .join(F.broadcast(m_df),
              (with_buckets.logical_id == m_df.m_logical_id)
              & (with_buckets.energy_type == m_df.m_energy_type),
              "inner")
        .withColumn("contrib", F.col("value") * F.col("coefficient"))
        .groupBy("hn2", "node_path", "m_energy_type", "purpose", "gran", "bucket")
        .agg(F.sum("contrib").alias("sum"),
             F.count("contrib").alias("count"),
             F.max("unit").alias("unit"))
        .withColumnRenamed("m_energy_type", "energy_type")
    )

    return grouped.select(
        F.concat(F.lit("HN2#"), F.col("hn2").cast("string")).alias("pk"),
        _SK_UDF("node_path", "energy_type", "purpose", "gran", "bucket").alias("sk"),
        _GSI1PK_UDF("hn2", _DIM_UDF("unit"), "purpose").alias("gsi1pk"),
        _GSI1SK_UDF("node_path", "gran", "bucket").alias("gsi1sk"),
        "energy_type", "purpose", "unit", "sum", "count",
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
    """logical_data is event-sourced (append-only): for a given (logical_id, timestamp) the
    newest ingested_time row supersedes older ones. Keep only that newest row per point, then
    filter to counter rows that carry a company id.

    Counters are the consumption axis: their `value` is a delta over the point's interval, so
    summing them is meaningful. A gauge's value is a level (°C, bar) and summing it is not, so
    gauges are excluded. Input must include ingested_time and reading_kind."""
    newest = Window.partitionBy("logical_id", "timestamp") \
        .orderBy(F.col("ingested_time").desc())
    return (
        df.withColumn("_rn", F.row_number().over(newest))
        .filter((F.col("_rn") == 1)
                & (F.col("reading_kind") == "counter")
                & F.col("value").isNotNull()
                & F.col("hn2").isNotNull())
        .drop("_rn")
    )


def read_counters(spark, window_start: str):
    """Newest-ingested counter rows for buckets at/after window_start.

    Windows by timestamp (the bucket axis) so whole hour/day buckets are recomputed from
    all their points, and dedups to the newest ingested_time per (logical_id, timestamp)
    — matching how every consumer reads the event-sourced logical_data table. Restatements of
    points whose timestamp is older than the window are not picked up (documented hook;
    widen --lookback_days to recompute them)."""
    raw = spark.sql(f"""
        SELECT hn2, logical_id, energy_type, unit,
               value, timestamp, reading_kind, ingested_time
        FROM all.logical_data
        WHERE timestamp >= TIMESTAMP '{window_start}'
    """)
    return latest_counters(raw).select(
        "hn2", "logical_id", "energy_type", "unit", "value", "timestamp")


def bucket_of_sk(sk: str) -> str:
    """The trailing bucket of `<node_path>#<energy_type>#<purpose>#<gran>#<bucket>`."""
    return sk.rsplit("#", 1)[-1] if "#" in sk else ""


def prune_window(table_name: str, region: str, companies: list, keep: set,
                 start_bucket: str, end_bucket: str) -> int:
    """Delete rollup rows in the recomputed window that this run did NOT produce.

    PutItem alone is idempotent for rows the job still writes, but it cannot retract
    rows it has *stopped* writing — and a formula change does exactly that. Declaring
    dhw=0.28 and space_heating=0.72 makes `unallocated` cancel to zero, so the matrix
    (correctly, being sparse) stops emitting those rows; without this the previous
    run's `unallocated` rows survive and the API keeps serving them. They carry a
    90/730-day TTL, so they would otherwise be wrong for months.

    The job recomputes the whole day-aligned window from scratch, so anything in the
    window it did not just write is by definition obsolete.
    """
    import boto3

    ddb = boto3.resource("dynamodb", region_name=region)
    table = ddb.Table(table_name)
    stale = []
    for pk in companies:
        kwargs = {"KeyConditionExpression": boto3.dynamodb.conditions.Key("pk").eq(pk),
                  "ProjectionExpression": "pk, sk"}
        while True:
            page = table.query(**kwargs)
            for item in page.get("Items", []):
                sk = item["sk"]
                if sk in keep:
                    continue
                b = bucket_of_sk(sk)
                if start_bucket <= b <= end_bucket:
                    stale.append({"pk": item["pk"], "sk": sk})
            if "LastEvaluatedKey" not in page:
                break
            kwargs["ExclusiveStartKey"] = page["LastEvaluatedKey"]

    with table.batch_writer() as bw:
        for k in stale:
            bw.delete_item(Key=k)
    return len(stale)


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

    required = ["JOB_NAME", "region", "table_bucket_name", "account_id", "rollup_table",
                "hierarchy_reader_role_arn"]
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
    counters = read_counters(spark, window_start).cache()

    # One matrix query per company actually present in the window, merged into a single
    # broadcast side. Companies with no readings cost nothing; a company with no matrix
    # rows contributes none and simply produces no rollups (the join is inner).
    import hierarchy_matrix
    companies = [r["hn2"] for r in counters.select("hn2").distinct().collect()]
    table = hierarchy_matrix.reader_table(args["hierarchy_reader_role_arn"], region)
    matrix = [row for c in companies for row in hierarchy_matrix.load_matrix(table, c)]
    print("measurements-aggregate: %d companies, %d matrix rows" % (len(companies), len(matrix)))

    if not matrix:
        print("measurements-aggregate: empty matrix - nothing to roll up")
        job.commit()
        return

    rollups = build_rollups(counters, matrix, now.strftime("%Y-%m-%dT%H:%M:%S+00:00")).cache()
    write_to_dynamo(rollups, args["rollup_table"], region)

    # Retract rows this run no longer produces (see prune_window). The window is
    # the day-aligned recompute range, expressed in both bucket labels so hourly
    # ("YYYY-MM-DDThh") and daily ("YYYY-MM-DD") labels both compare correctly:
    # the daily label is a prefix of the hourly one, so a plain string range over
    # [start_day, end_day + "T99"] covers both.
    written = {r["sk"] for r in rollups.select("sk").distinct().collect()}
    pks = [r["pk"] for r in rollups.select("pk").distinct().collect()]
    start_day = window_start[:10]
    end_day = now.strftime("%Y-%m-%d") + "T99"
    pruned = prune_window(args["rollup_table"], region, pks, written, start_day, end_day)
    print("measurements-aggregate: wrote %d rows, pruned %d stale" % (len(written), pruned))
    job.commit()


if __name__ == "__main__":
    main()
