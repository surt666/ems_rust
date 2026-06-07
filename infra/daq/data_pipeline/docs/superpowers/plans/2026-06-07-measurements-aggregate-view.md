# Measurements Aggregate Materialized View — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a DynamoDB table `measurements_aggregate` and an hourly Glue job that pre-aggregates counter consumption from the `logical_meter_data` Iceberg table into per-node, per-purpose, hourly+daily buckets queryable by hierarchy path.

**Architecture:** A PySpark Glue job reads a configurable trailing-days window of `logical_meter_data`, filters to resampled counters, explodes each reading into its ancestor-node keys (company `hn2` → … → leaf meter), groups by `(node, purpose, granularity, bucket)`, and upserts pre-summed items into DynamoDB. A new Go CDK stack provisions the table, job, hourly schedule, and IAM. See spec: `docs/superpowers/specs/2026-06-07-measurements-rollup-view-design.md`.

**Tech Stack:** PySpark (Glue 5.0 / Spark 3.5), boto3, DynamoDB (on-demand + TTL), AWS CDK (Go), Iceberg/S3 Tables, EventBridge. Tests: pytest + local pyspark.

---

## File Structure

- `glue/measurements_aggregate.py` — the Glue job. Split into: pure helpers (no Spark), a `build_rollups(df)` Spark transform, a `write_to_dynamo(df)` sink, and a `main()` (Spark/Glue init + IO) called only under `__main__`. Importable for tests without triggering Spark init.
- `glue/tests/test_helpers.py` — unit tests for the pure helpers (no Spark).
- `glue/tests/test_rollups.py` — tests for `build_rollups` using a local SparkSession.
- `glue/tests/conftest.py` — session-scoped local SparkSession fixture (UTC).
- `glue/tests/requirements.txt` — `pyspark==3.5.4`, `pytest`.
- `measurements_aggregate_stack.go` — new CDK stack: table + Glue job + schedule + IAM/LF perms.
- `main.go` — register the new stack; add `LookbackDays` context.
- `CLAUDE.md` (repo root) — add the new stack to the data-pipeline deploy list.

---

## Task 1: Pure helper functions + unit tests

**Files:**
- Create: `glue/measurements_aggregate.py`
- Create: `glue/tests/requirements.txt`
- Create: `glue/tests/test_helpers.py`

- [ ] **Step 1: Create the test requirements file**

Create `glue/tests/requirements.txt`:

```
pyspark==3.5.4
pytest==8.3.3
```

- [ ] **Step 2: Write the failing helper tests**

Create `glue/tests/test_helpers.py`:

```python
from datetime import datetime, timezone
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import measurements_aggregate as m


def test_hour_bucket_is_utc():
    ts = datetime(2026, 6, 7, 8, 45, tzinfo=timezone.utc)
    assert m.hour_bucket(ts) == "2026-06-07T08"


def test_day_bucket_is_utc():
    ts = datetime(2026, 6, 7, 8, 45, tzinfo=timezone.utc)
    assert m.day_bucket(ts) == "2026-06-07"


def test_ttl_hour_is_bucket_end_plus_90d():
    # hour 2026-06-07T08 ends at 09:00Z; +90 days
    ttl = m.ttl_for("h", "2026-06-07T08")
    end = int(datetime(2026, 6, 7, 9, tzinfo=timezone.utc).timestamp())
    assert ttl == end + 90 * 86400


def test_ttl_day_is_bucket_end_plus_730d():
    ttl = m.ttl_for("d", "2026-06-07")
    end = int(datetime(2026, 6, 8, 0, tzinfo=timezone.utc).timestamp())
    assert ttl == end + 730 * 86400


def test_ancestor_keys_full_depth():
    # hns = [hn2..hn9]; meter under hn2=2, hn3=9, hn4=456
    keys = m.ancestor_keys([2, 9, 456, None, None, None, None, None], 10009)
    assert keys == [
        ("2", ""),
        ("3", "HN3#9"),
        ("4", "HN3#9|HN4#456"),
        ("leaf", "HN3#9|HN4#456|L#10009"),
    ]


def test_ancestor_keys_meter_directly_under_company():
    keys = m.ancestor_keys([2, None, None, None, None, None, None, None], 10009)
    assert keys == [("2", ""), ("leaf", "L#10009")]


def test_build_sk_company_and_node_and_leaf():
    assert m.build_sk("", "Electricity", "d", "2026-06-07") == "#Electricity#d#2026-06-07"
    assert m.build_sk("HN3#9|HN4#456", "Electricity", "d", "2026-06-07") == \
        "HN3#9|HN4#456#Electricity#d#2026-06-07"
    assert m.build_sk("HN3#9|HN4#456|L#10009", "Electricity", "h", "2026-06-07T08") == \
        "HN3#9|HN4#456|L#10009#Electricity#h#2026-06-07T08"


def test_delimiter_invariant_node_sorts_before_descendants():
    own = m.build_sk("HN3#9|HN4#456", "Electricity", "d", "2026-06-07")
    child = m.build_sk("HN3#9|HN4#456|L#10009", "Electricity", "d", "2026-06-07")
    # a node's own row must sort strictly before any descendant row
    assert own < child
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd glue && python -m pytest tests/test_helpers.py -q`
Expected: FAIL — `AttributeError: module 'measurements_aggregate' has no attribute 'hour_bucket'` (module/functions don't exist yet).

- [ ] **Step 4: Write the helper implementation**

Create `glue/measurements_aggregate.py` with ONLY the pure helpers for now (no Spark imports at module top so the test import is cheap):

```python
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
    """hns = [hn2, hn3, ..., hn9] (ints or None). Returns [(level_label, node_path)] for every
    populated level from hn2 down, plus the leaf meter.
      level_label: "2".."9" for hn nodes, "leaf" for the meter.
      node_path:   sk path-below-hn2 (company => "")."""
    result = [("2", "")]
    segments = []
    for depth, hid in enumerate(hns[1:], start=3):  # hn3..hn9
        if hid is None:
            break
        segments.append("HN%d#%d" % (depth, hid))
        result.append((str(depth), "|".join(segments)))
    leaf_path = "|".join(segments + ["L#%d" % logical_id]) if segments else "L#%d" % logical_id
    result.append(("leaf", leaf_path))
    return result


def build_sk(node_path: str, purpose: str, gran: str, bucket: str) -> str:
    """sk = '<node_path>#<purpose>#<gran>#<bucket>'. The '#' after node_path is the delimiter
    that keeps a node's own rows sorting before its descendants' ('|' > '#')."""
    return "%s#%s#%s#%s" % (node_path, purpose, gran, bucket)
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd glue && python -m pytest tests/test_helpers.py -q`
Expected: PASS (8 passed). (If pyspark/pytest not installed: `pip install -r tests/requirements.txt` first.)

- [ ] **Step 6: Commit**

```bash
git add glue/measurements_aggregate.py glue/tests/requirements.txt glue/tests/test_helpers.py
git commit -m "feat(agg): pure helpers for measurements_aggregate rollup (buckets, ttl, sk keys)"
```

---

## Task 2: Spark rollup transform + tests

**Files:**
- Modify: `glue/measurements_aggregate.py` (add `build_rollups`)
- Create: `glue/tests/conftest.py`
- Create: `glue/tests/test_rollups.py`

- [ ] **Step 1: Add the local SparkSession fixture**

Create `glue/tests/conftest.py`:

```python
import pytest
from pyspark.sql import SparkSession


@pytest.fixture(scope="session")
def spark():
    s = (
        SparkSession.builder.master("local[1]")
        .appName("measurements-aggregate-tests")
        .config("spark.sql.session.timeZone", "UTC")
        .config("spark.sql.shuffle.partitions", "1")
        .getOrCreate()
    )
    yield s
    s.stop()
```

- [ ] **Step 2: Write the failing transform test**

Create `glue/tests/test_rollups.py`:

```python
import sys, os
from datetime import datetime, timezone
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import measurements_aggregate as m
from pyspark.sql import types as T


def _input(spark):
    # two meters under company hn2=2: meter 10009 under hn3=9|hn4=456, meter 10010 under hn3=9.
    # all Electricity counters (resample_method='time_proportional').
    schema = T.StructType([
        T.StructField("hn2", T.IntegerType()), T.StructField("hn3", T.IntegerType()),
        T.StructField("hn4", T.IntegerType()), T.StructField("hn5", T.IntegerType()),
        T.StructField("hn6", T.IntegerType()), T.StructField("hn7", T.IntegerType()),
        T.StructField("hn8", T.IntegerType()), T.StructField("hn9", T.IntegerType()),
        T.StructField("logical_id", T.IntegerType()), T.StructField("purpose", T.StringType()),
        T.StructField("resample_value", T.DoubleType()), T.StructField("value", T.DoubleType()),
        T.StructField("timestamp", T.TimestampType()),
        T.StructField("resample_timestamp", T.TimestampType()),
    ])
    def ts(h, mi=0):
        return datetime(2026, 6, 7, h, mi, tzinfo=timezone.utc)
    rows = [
        # meter 10009: two readings in hour 08
        (2, 9, 456, None, None, None, None, None, 10009, "Electricity", 4.0, 100.0, ts(8, 15), ts(8, 15)),
        (2, 9, 456, None, None, None, None, None, 10009, "Electricity", 6.0, 106.0, ts(8, 45), ts(8, 45)),
        # meter 10010 (under hn3=9 directly): one reading in hour 08
        (2, 9, None, None, None, None, None, None, 10010, "Electricity", 5.0, 50.0, ts(8, 30), ts(8, 30)),
    ]
    return spark.createDataFrame(rows, schema)


def _by_sk(df):
    return {r["sk"]: r for r in df.collect()}


def test_rollup_sums_at_every_level(spark):
    out = _by_sk(m.build_rollups(_input(spark), run_at_iso="2026-06-07T09:05:00Z"))
    # hourly company total = 4+6+5 = 15
    assert out["#Electricity#h#2026-06-07T08"]["sum"] == 15.0
    # hn3=9 total = 15 (both meters under it)
    assert out["HN3#9#Electricity#h#2026-06-07T08"]["sum"] == 15.0
    # hn4=456 total = 10 (only meter 10009)
    assert out["HN3#9|HN4#456#Electricity#h#2026-06-07T08"]["sum"] == 10.0
    # leaf 10009 = 10, count=2, last_value at latest ts = 106
    leaf = out["HN3#9|HN4#456|L#10009#Electricity#h#2026-06-07T08"]
    assert leaf["sum"] == 10.0 and leaf["count"] == 2 and leaf["last_value"] == 106.0
    # daily company total also 15 (only hour 08 present)
    assert out["#Electricity#d#2026-06-07"]["sum"] == 15.0


def test_rollup_is_idempotent(spark):
    a = _by_sk(m.build_rollups(_input(spark), run_at_iso="2026-06-07T09:05:00Z"))
    b = _by_sk(m.build_rollups(_input(spark), run_at_iso="2026-06-07T10:05:00Z"))
    # same pk/sk/sum/count regardless of run time (updated_at may differ)
    assert a.keys() == b.keys()
    for k in a:
        assert a[k]["sum"] == b[k]["sum"] and a[k]["count"] == b[k]["count"]


def test_ttl_and_pk_present(spark):
    out = _by_sk(m.build_rollups(_input(spark), run_at_iso="2026-06-07T09:05:00Z"))
    row = out["#Electricity#d#2026-06-07"]
    assert row["pk"] == "2"
    assert row["ttl"] == m.ttl_for("d", "2026-06-07")
```

- [ ] **Step 3: Run to verify it fails**

Run: `cd glue && python -m pytest tests/test_rollups.py -q`
Expected: FAIL — `AttributeError: module 'measurements_aggregate' has no attribute 'build_rollups'`.

- [ ] **Step 4: Implement `build_rollups`**

Append to `glue/measurements_aggregate.py` (the Spark imports go here, NOT at module top, so Task-1 tests stay Spark-free — but pyspark is installed, so a top import is fine too; keep them here for clarity):

```python
# ── Spark transform ──

from pyspark.sql import DataFrame, functions as F, types as T  # noqa: E402

_ANCESTOR_SCHEMA = T.ArrayType(T.StructType([
    T.StructField("level", T.StringType()),
    T.StructField("node_path", T.StringType()),
]))


@F.udf(_ANCESTOR_SCHEMA)
def _ancestor_keys_udf(hn2, hn3, hn4, hn5, hn6, hn7, hn8, hn9, logical_id):
    return [{"level": lvl, "node_path": p}
            for (lvl, p) in ancestor_keys([hn2, hn3, hn4, hn5, hn6, hn7, hn8, hn9], logical_id)]


_TTL_UDF = F.udf(ttl_for, T.LongType())
_SK_UDF = F.udf(build_sk, T.StringType())


def build_rollups(df: DataFrame, run_at_iso: str) -> DataFrame:
    """Aggregate counter rows into per-node/purpose/granularity/bucket rollup items.

    Input columns: hn2..hn9 (int), logical_id (int), purpose (str), resample_value (double),
    value (double), timestamp (ts), resample_timestamp (ts).
    Output columns: pk, sk, level, purpose, gran, bucket, sum, count, min, max,
    last_value, last_ts, unit?, updated_at, ttl.
    NOTE: caller must set spark.sql.session.timeZone='UTC' so the bucket labels are UTC.
    """
    # 1) hour + day bucket per row, exploded into (gran, bucket).
    with_buckets = df.withColumn(
        "gb",
        F.explode(F.array(
            F.struct(F.lit("h").alias("gran"),
                     F.date_format(F.col("resample_timestamp"), "yyyy-MM-dd'T'HH").alias("bucket")),
            F.struct(F.lit("d").alias("gran"),
                     F.date_format(F.col("resample_timestamp"), "yyyy-MM-dd").alias("bucket")),
        )),
    ).select("*", F.col("gb.gran").alias("gran"), F.col("gb.bucket").alias("bucket"))

    # 2) explode each row into its ancestor node keys (+ leaf).
    with_nodes = with_buckets.withColumn(
        "node",
        F.explode(_ancestor_keys_udf(
            *[F.col("hn%d" % i) for i in range(2, 10)], F.col("logical_id"))),
    ).select(
        "*", F.col("node.level").alias("level"), F.col("node.node_path").alias("node_path"))

    # 3) group by (company pk, node, purpose, gran, bucket) and aggregate.
    grouped = with_nodes.groupBy(
        "hn2", "node_path", "level", "purpose", "gran", "bucket"
    ).agg(
        F.sum("resample_value").alias("sum"),
        F.count("resample_value").alias("count"),
        F.min("resample_value").alias("min"),
        F.max("resample_value").alias("max"),
        F.max(F.struct(F.col("timestamp"), F.col("value"))).alias("_last"),
    )

    # 4) project to the item shape.
    return grouped.select(
        F.col("hn2").cast("string").alias("pk"),
        _SK_UDF("node_path", "purpose", "gran", "bucket").alias("sk"),
        "level", "purpose", "gran", "bucket", "sum", "count", "min", "max",
        F.col("_last.value").alias("last_value"),
        F.date_format(F.col("_last.timestamp"), "yyyy-MM-dd'T'HH:mm:ssXXX").alias("last_ts"),
        F.lit(run_at_iso).alias("updated_at"),
        _TTL_UDF("gran", "bucket").alias("ttl"),
    )
```

- [ ] **Step 5: Run to verify it passes**

Run: `cd glue && python -m pytest tests/test_rollups.py -q`
Expected: PASS (3 passed).

- [ ] **Step 6: Run the full suite + commit**

Run: `cd glue && python -m pytest tests/ -q` → all pass.

```bash
git add glue/measurements_aggregate.py glue/tests/conftest.py glue/tests/test_rollups.py
git commit -m "feat(agg): build_rollups Spark transform (all-level rollup via ancestor explode)"
```

---

## Task 3: Job IO wiring (`main`)

**Files:**
- Modify: `glue/measurements_aggregate.py` (add `read_window`, `write_to_dynamo`, `main`, `__main__` guard)

- [ ] **Step 1: Append the IO + main wiring**

Append to `glue/measurements_aggregate.py`:

```python
# ── IO + entrypoint ──

def window_start_iso(now: datetime, lookback_days: int) -> str:
    """00:00 UTC of (today - lookback_days)."""
    start_day = (now.astimezone(timezone.utc) - timedelta(days=lookback_days)).date()
    return datetime(start_day.year, start_day.month, start_day.day, tzinfo=timezone.utc) \
        .strftime("%Y-%m-%dT%H:%M:%S+00:00")


def read_counters(spark, window_start: str):
    """Resampled counter rows in [window_start, now], by ingested_time so restatements are caught."""
    return spark.sql(f"""
        SELECT hn2, hn3, hn4, hn5, hn6, hn7, hn8, hn9, logical_id, purpose,
               resample_value, value, timestamp, resample_timestamp
        FROM all.logical_meter_data
        WHERE resample_method = 'time_proportional'
          AND resample_value IS NOT NULL
          AND hn2 IS NOT NULL
          AND ingested_time >= TIMESTAMP '{window_start}'
    """)


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
```

- [ ] **Step 2: Write the failing test for `window_start_iso`**

Append to `glue/tests/test_helpers.py`:

```python
def test_window_start_is_day_aligned_utc():
    now = datetime(2026, 6, 7, 9, 30, tzinfo=timezone.utc)
    assert m.window_start_iso(now, 1) == "2026-06-06T00:00:00+00:00"
    assert m.window_start_iso(now, 0) == "2026-06-07T00:00:00+00:00"
```

- [ ] **Step 3: Run to verify it passes (function now exists)**

Run: `cd glue && python -m pytest tests/test_helpers.py -q`
Expected: PASS (9 passed). The `main`/Glue imports stay inside functions, so importing the module in tests never touches `awsglue`.

- [ ] **Step 4: Commit**

```bash
git add glue/measurements_aggregate.py glue/tests/test_helpers.py
git commit -m "feat(agg): job IO wiring (day-aligned read window, DynamoDB upsert, main)"
```

---

## Task 4: CDK stack + registration

**Files:**
- Create: `measurements_aggregate_stack.go`
- Modify: `main.go`

- [ ] **Step 1: Deploy the new script to the existing Glue script bucket**

The `LateRecomputationStack` already creates `glue-scripts-<acct>-<region>` and deploys `./glue` under `late-recomputation/`. Add a second deployment prefix for this job by creating the new stack's script under `measurements-aggregate/`. Create `measurements_aggregate_stack.go`:

```go
package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsdynamodb"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsevents"
	"github.com/aws/aws-cdk-go/awscdk/v2/awseventstargets"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsglue"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslakeformation"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3deployment"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type MeasurementsAggregateStackProps struct {
	awscdk.StackProps
	TableBucket  string
	LookbackDays string
}

func NewMeasurementsAggregateStack(scope constructs.Construct, id string, props *MeasurementsAggregateStackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, &props.StackProps)
	region := *stack.Region()
	account := *stack.Account()

	// ── DynamoDB materialized-view table ──
	table := awsdynamodb.NewTable(stack, jsii.String("MeasurementsAggregate"), &awsdynamodb.TableProps{
		TableName:    jsii.String("measurements_aggregate"),
		PartitionKey: &awsdynamodb.Attribute{Name: jsii.String("pk"), Type: awsdynamodb.AttributeType_STRING},
		SortKey:      &awsdynamodb.Attribute{Name: jsii.String("sk"), Type: awsdynamodb.AttributeType_STRING},
		BillingMode:  awsdynamodb.BillingMode_PAY_PER_REQUEST,
		TimeToLiveAttribute: jsii.String("ttl"),
		RemovalPolicy: awscdk.RemovalPolicy_RETAIN,
	})

	// ── Glue script bucket (separate from late-recomputation's, to keep stacks independent) ──
	scriptBucket := awss3.NewBucket(stack, jsii.String("AggScriptBucket"), &awss3.BucketProps{
		BucketName:        jsii.String("glue-agg-scripts-" + account + "-" + region),
		RemovalPolicy:     awscdk.RemovalPolicy_DESTROY,
		AutoDeleteObjects: jsii.Bool(true),
	})
	awss3deployment.NewBucketDeployment(stack, jsii.String("DeployAggScript"), &awss3deployment.BucketDeploymentProps{
		Sources:              &[]awss3deployment.ISource{awss3deployment.Source_Asset(jsii.String("./glue"), nil)},
		DestinationBucket:    scriptBucket,
		DestinationKeyPrefix: jsii.String("measurements-aggregate/"),
	})

	// ── Glue job IAM role ──
	managedPolicies := []awsiam.IManagedPolicy{
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("service-role/AWSGlueServiceRole")),
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AmazonS3TablesFullAccess")),
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AWSLakeFormationDataAdmin")),
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AmazonS3FullAccess")),
	}
	glueRole := awsiam.NewRole(stack, jsii.String("AggGlueJobRole"), &awsiam.RoleProps{
		AssumedBy:       awsiam.NewServicePrincipal(jsii.String("glue.amazonaws.com"), nil),
		ManagedPolicies: &managedPolicies,
	})
	addPolicy := func(actions, resources *[]*string) {
		glueRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
			Effect: awsiam.Effect_ALLOW, Actions: actions, Resources: resources,
		}))
	}
	addPolicy(jsii.Strings("glue:GetDatabase", "glue:GetDatabases", "glue:GetTable", "glue:GetTables", "glue:GetCatalog"),
		jsii.Strings(
			"arn:aws:glue:"+region+":"+account+":catalog",
			"arn:aws:glue:"+region+":"+account+":catalog/s3tablescatalog",
			"arn:aws:glue:"+region+":"+account+":catalog/s3tablescatalog/"+props.TableBucket,
			"arn:aws:glue:"+region+":"+account+":database/s3tablescatalog/"+props.TableBucket+"/*",
			"arn:aws:glue:"+region+":"+account+":table/s3tablescatalog/"+props.TableBucket+"/*/*",
		))
	table.GrantWriteData(glueRole)
	scriptBucket.GrantRead(glueRole, nil)

	// ── Lake Formation: read logical_meter_data ──
	s3tablesCatalogId := account + ":s3tablescatalog/" + props.TableBucket
	dlPrincipal := &awslakeformation.CfnPermissions_DataLakePrincipalProperty{
		DataLakePrincipalIdentifier: glueRole.RoleArn(),
	}
	awslakeformation.NewCfnPermissions(stack, jsii.String("AggLfDbPermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dlPrincipal,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			DatabaseResource: &awslakeformation.CfnPermissions_DatabaseResourceProperty{
				Name: jsii.String("all"), CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("DESCRIBE"),
	})
	awslakeformation.NewCfnPermissions(stack, jsii.String("AggLfTablePermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dlPrincipal,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			TableResource: &awslakeformation.CfnPermissions_TableResourceProperty{
				DatabaseName: jsii.String("all"), Name: jsii.String("logical_meter_data"),
				CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("SELECT", "DESCRIBE"),
	})

	// ── Glue job ──
	awsglue.NewCfnJob(stack, jsii.String("MeasurementsAggregateJob"), &awsglue.CfnJobProps{
		Name: jsii.String("measurements-aggregate"),
		Role: glueRole.RoleArn(),
		Command: &awsglue.CfnJob_JobCommandProperty{
			Name:           jsii.String("glueetl"),
			PythonVersion:  jsii.String("3"),
			ScriptLocation: jsii.String("s3://" + *scriptBucket.BucketName() + "/measurements-aggregate/measurements_aggregate.py"),
		},
		GlueVersion:     jsii.String("5.0"),
		WorkerType:      jsii.String("G.1X"),
		NumberOfWorkers: jsii.Number(2),
		Timeout:         jsii.Number(60),
		DefaultArguments: &map[string]string{
			"--region":                           region,
			"--table_bucket_name":                props.TableBucket,
			"--account_id":                       account,
			"--rollup_table":                     *table.TableName(),
			"--lookback_days":                    props.LookbackDays,
			"--conf":                             "spark.sql.extensions=org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions",
			"--enable-glue-datacatalog":          "true",
			"--enable-metrics":                   "true",
			"--enable-continuous-cloudwatch-log": "true",
			"--job-language":                     "python",
		},
	})

	// ── Hourly schedule (a few minutes past the hour) ──
	rule := awsevents.NewRule(stack, jsii.String("AggHourlySchedule"), &awsevents.RuleProps{
		Schedule: awsevents.Schedule_Cron(&awsevents.CronOptions{Minute: jsii.String("5")}),
	})
	rule.AddTarget(awseventstargets.NewAwsApi(&awseventstargets.AwsApiProps{
		Service: jsii.String("Glue"),
		Action:  jsii.String("startJobRun"),
		Parameters: &map[string]interface{}{"JobName": "measurements-aggregate"},
	}))

	return stack
}
```

- [ ] **Step 2: Register the stack + add the `LookbackDays` context**

In `main.go`, after the existing `tableBucketName := contextString(...)` line, add:

```go
	lookbackDays := contextString(app, "LookbackDays", "1")
```

And after the `NewS3TablesStack(...)` registration block, add:

```go
	NewMeasurementsAggregateStack(app, "MeasurementsAggregateStack", &MeasurementsAggregateStackProps{
		StackProps:   awscdk.StackProps{Env: env},
		TableBucket:  tableBucketName,
		LookbackDays: lookbackDays,
	})
```

(Use the same `env` value the sibling stacks pass — copy the `Env:` field from the `NewS3TablesStack` call's `StackProps`.)

- [ ] **Step 3: Verify the CDK app synthesizes**

Run:
```bash
cd infra/daq/data_pipeline
unset GOROOT
npx cdk synth MeasurementsAggregateStack \
  -c SHA="test" -c RUN_NR="1" -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements >/dev/null
```
Expected: synthesizes with no error (prints CloudFormation to stdout, discarded). If `awseventstargets.NewAwsApi` is unavailable in the pinned CDK version, fall back to a `targets.LambdaFunction` invoking `glue.StartJobRun`, or `awsevents.Schedule` on a `CfnTrigger` (Glue scheduled trigger) — verify the API in `go doc github.com/aws/aws-cdk-go/awscdk/v2/awseventstargets`.

- [ ] **Step 4: Commit**

```bash
git add infra/daq/data_pipeline/measurements_aggregate_stack.go infra/daq/data_pipeline/main.go
git commit -m "feat(agg): CDK stack — measurements_aggregate table + hourly Glue job + schedule"
```

---

## Task 5: Deploy docs + verification

**Files:**
- Modify: `CLAUDE.md` (repo root)

- [ ] **Step 1: Add the stack to the deploy policy**

In `CLAUDE.md`, under "Stack 2 — Data pipeline", add `MeasurementsAggregateStack` to the `cdk deploy` list and note the new context flag, e.g.:

```
npx cdk deploy DaqPipelineStack LateRecomputationStack OcamlBridgeWriterRoleStack MeasurementsAggregateStack \
  --require-approval never \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
```

Add a bullet: "`MeasurementsAggregateStack` owns `measurements_aggregate` (DynamoDB, RETAIN) + the hourly `measurements-aggregate` Glue job; `-c LookbackDays=N` sets the day-aligned recompute window (default 1)."

- [ ] **Step 2: Commit the docs**

```bash
git add CLAUDE.md
git commit -m "docs(CLAUDE.md): add MeasurementsAggregateStack to data-pipeline deploy list"
```

- [ ] **Step 3: (Deploy — operator step, requires daq `891` creds)**

```bash
cd infra/daq/data_pipeline
unset GOROOT
npx cdk diff MeasurementsAggregateStack -c TableBucketName=measurements -c LookbackDays=1   # confirm NEW resources only
npx cdk deploy MeasurementsAggregateStack --require-approval never \
  -c SHA="$(git rev-parse --short HEAD)" -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 -c MaxKPU=4 -c TableBucketName=measurements -c LookbackDays=1
```

- [ ] **Step 4: (Verify — operator step) run the job once and spot-check the table**

```bash
aws glue start-job-run --job-name measurements-aggregate
# wait for SUCCEEDED:
aws glue get-job-runs --job-name measurements-aggregate --query 'JobRuns[0].JobRunState'
# read a company's daily electricity series:
aws dynamodb query --table-name measurements_aggregate \
  --key-condition-expression 'pk = :p AND begins_with(sk, :s)' \
  --expression-attribute-values '{":p":{"S":"<hn2>"},":s":{"S":"#Electricity#d#"}}'
```
Expected: items returned with `sum/count/min/max/last_value/ttl`; a node's `begins_with` does not leak descendant rows (the `#`-vs-`|` invariant).

---

## Self-Review

**Spec coverage:**
- Table schema (pk/sk/delimiter/attrs/TTL) → Task 1 (helpers) + Task 4 (table). ✓
- All-levels rollup → Task 2 (`build_rollups` ancestor explode). ✓
- Counters-only filter (`time_proportional`) → Task 3 (`read_counters`). ✓
- Day-aligned configurable lookback → Task 3 (`window_start_iso`) + Task 4 (`--lookback_days` / `LookbackDays`). ✓
- Hourly schedule, idempotent upsert → Task 3 (`write_to_dynamo`, `overwrite_by_pkeys`) + Task 4 (schedule). ✓
- UTC buckets → Task 3 (`spark.sql.session.timeZone=UTC`) + Task 1 tests. ✓
- Tests (bucket/sk/ttl/delimiter, aggregation, idempotency) → Tasks 1–2. ✓
- Read API deferred → not in plan (correct, out of scope). ✓

**Placeholder scan:** none — every code/test/command step is concrete.

**Type consistency:** `ancestor_keys`/`build_sk`/`ttl_for`/`window_start_iso` signatures match between implementation and tests; `build_rollups(df, run_at_iso)` matches its tests; CDK prop names (`TableBucket`, `LookbackDays`) match between stack and `main.go`; Glue arg `--rollup_table` matches `required` in `main()`.

**Note for the implementer:** verify `awseventstargets.NewAwsApi` exists in the repo's pinned aws-cdk-go version (Task 4 Step 3) before relying on it; the fallback options are listed inline.
