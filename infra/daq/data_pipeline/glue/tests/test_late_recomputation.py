"""Cross-artifact guard: the Glue writer's column list vs the CDK table definition.

`late_recomputation` is the second writer of `all.logical_data` (Flink is the first, and
`LogicalDataSchemaSpec` guards that one). A rename inside this file is self-consistent by
construction — code and fixtures move together — so no ordinary test notices that the
Iceberg table still has the old column. This one reads the Go source that creates it.
"""
import os
import re
import sys
from datetime import datetime, timezone

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import late_recomputation as lr
from pyspark.sql import types as T

_STACK = os.path.join(os.path.dirname(__file__), "..", "..", "s3tables_stack.go")


def _go_schema_fields():
    """Field names from the `logical_data` table block of s3tables_stack.go, in order."""
    with open(_STACK) as f:
        go = f.read()
    start = go.index('jsii.String("logical_data")')
    end = go.index("icebergPartitionSpec", start)
    return re.findall(r'field\("([a-z0-9_]+)"', go[start:end])


def _joined(spark, rows):
    """A frame shaped like the raw_data ⨝ sensor-identity join that feeds build_output."""
    schema = T.StructType([
        T.StructField("logical_id", T.IntegerType()),
        T.StructField("timestamp", T.TimestampType()),
        T.StructField("value", T.DoubleType()),
        T.StructField("unit", T.StringType()),
        T.StructField("hn1", T.IntegerType()),
        T.StructField("hn2", T.IntegerType()),
        T.StructField("hn3", T.IntegerType()),
        T.StructField("hn4", T.IntegerType()),
        T.StructField("hn5", T.IntegerType()),
        T.StructField("hn6", T.IntegerType()),
        T.StructField("hn7", T.IntegerType()),
        T.StructField("hn8", T.IntegerType()),
        T.StructField("hn9", T.IntegerType()),
        T.StructField("energy_type", T.StringType()),
        T.StructField("reading_kind", T.StringType()),
        T.StructField("resample_value", T.DoubleType()),
        T.StructField("resample_timestamp", T.TimestampType()),
    ])
    return spark.createDataFrame(rows, schema)


def _ts(h, mi=0):
    return datetime(2026, 6, 7, h, mi, tzinfo=timezone.utc)


def _same_instant(collected, expected):
    """Spark hands back naive datetimes in the local zone; compare the instant, not the wall clock."""
    return collected.timestamp() == expected.timestamp()


def test_output_columns_match_the_iceberg_table(spark):
    out = lr.build_output(_joined(spark, [
        (10009, _ts(8, 7), 4.0, "kWh", 1, 2, None, None, None, None, None, None, None,
         "electricity", "counter", 3.0, _ts(8, 15)),
    ]))
    assert out.columns == _go_schema_fields()


def test_resampled_row_carries_the_resampled_pair(spark):
    out = lr.build_output(_joined(spark, [
        (10009, _ts(8, 7), 4.0, "kWh", 1, 2, None, None, None, None, None, None, None,
         "electricity", "counter", 3.0, _ts(8, 15)),
    ])).collect()
    assert _same_instant(out[0]["timestamp"], _ts(8, 15))
    assert out[0]["value"] == 3000.0  # kWh -> Wh


def test_passthrough_row_carries_its_own_reading(spark):
    """resample_minutes IS NULL: the reading is its own bin, so its raw pair is written
    rather than dropped — otherwise an unconfigured sensor would write empty rows."""
    out = lr.build_output(_joined(spark, [
        (10010, _ts(8, 7), 4.0, "kWh", 1, 2, None, None, None, None, None, None, None,
         "electricity", "counter", None, None),
    ])).collect()
    assert _same_instant(out[0]["timestamp"], _ts(8, 7))
    assert out[0]["value"] == 4000.0
