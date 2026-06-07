import sys, os
from datetime import datetime, timezone
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import measurements_aggregate as m
from pyspark.sql import types as T


def _input(spark):
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
        (2, 9, 456, None, None, None, None, None, 10009, "Electricity", 4.0, 100.0, ts(8, 15), ts(8, 15)),
        (2, 9, 456, None, None, None, None, None, 10009, "Electricity", 6.0, 106.0, ts(8, 45), ts(8, 45)),
        (2, 9, None, None, None, None, None, None, 10010, "Electricity", 5.0, 50.0, ts(8, 30), ts(8, 30)),
    ]
    return spark.createDataFrame(rows, schema)


def _by_sk(df):
    return {r["sk"]: r for r in df.collect()}


def test_rollup_sums_at_every_level(spark):
    out = _by_sk(m.build_rollups(_input(spark), run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2#Electricity#h#2026-06-07T08"]["sum"] == 15.0
    assert out["HN2#2|HN3#9#Electricity#h#2026-06-07T08"]["sum"] == 15.0
    assert out["HN2#2|HN3#9|HN4#456#Electricity#h#2026-06-07T08"]["sum"] == 10.0
    leaf = out["HN2#2|HN3#9|HN4#456|L#10009#Electricity#h#2026-06-07T08"]
    assert leaf["sum"] == 10.0 and leaf["count"] == 2 and leaf["last_value"] == 106.0
    assert out["HN2#2#Electricity#d#2026-06-07"]["sum"] == 15.0


def test_rollup_is_idempotent(spark):
    a = _by_sk(m.build_rollups(_input(spark), run_at_iso="2026-06-07T09:05:00Z"))
    b = _by_sk(m.build_rollups(_input(spark), run_at_iso="2026-06-07T10:05:00Z"))
    assert a.keys() == b.keys()
    for k in a:
        assert a[k]["sum"] == b[k]["sum"] and a[k]["count"] == b[k]["count"]


def test_ttl_and_pk_present(spark):
    out = _by_sk(m.build_rollups(_input(spark), run_at_iso="2026-06-07T09:05:00Z"))
    row = out["HN2#2#Electricity#d#2026-06-07"]
    assert row["pk"] == "HN2#2"
    assert row["ttl"] == m.ttl_for("d", "2026-06-07")


def _raw(spark, rows):
    schema = T.StructType([
        T.StructField("hn2", T.IntegerType()), T.StructField("hn3", T.IntegerType()),
        T.StructField("hn4", T.IntegerType()), T.StructField("hn5", T.IntegerType()),
        T.StructField("hn6", T.IntegerType()), T.StructField("hn7", T.IntegerType()),
        T.StructField("hn8", T.IntegerType()), T.StructField("hn9", T.IntegerType()),
        T.StructField("logical_id", T.IntegerType()), T.StructField("purpose", T.StringType()),
        T.StructField("resample_value", T.DoubleType()), T.StructField("value", T.DoubleType()),
        T.StructField("timestamp", T.TimestampType()),
        T.StructField("resample_timestamp", T.TimestampType()),
        T.StructField("resample_method", T.StringType()),
        T.StructField("ingested_time", T.TimestampType()),
    ])
    return spark.createDataFrame(rows, schema)


def test_latest_counters_dedup_and_filters(spark):
    rt = datetime(2026, 6, 7, 8, tzinfo=timezone.utc)
    def ing(h):
        return datetime(2026, 6, 7, h, tzinfo=timezone.utc)
    rows = [
        # same point (10009, rt): original value 4 then a later-ingested restatement 9 -> 9 wins
        (2, 9, 456, None, None, None, None, None, 10009, "Electricity", 4.0, 100.0, rt, rt, "time_proportional", ing(9)),
        (2, 9, 456, None, None, None, None, None, 10009, "Electricity", 9.0, 106.0, rt, rt, "time_proportional", ing(11)),
        # a gauge point (linear_interpolation) -> excluded
        (2, 9, None, None, None, None, None, None, 10010, "Temperature", 5.0, 5.0, rt, rt, "linear_interpolation", ing(9)),
        # null company id -> excluded
        (None, None, None, None, None, None, None, None, 10011, "Electricity", 3.0, 3.0, rt, rt, "time_proportional", ing(9)),
    ]
    out = m.latest_counters(_raw(spark, rows)).collect()
    assert len(out) == 1
    r = out[0]
    assert r["logical_id"] == 10009 and r["resample_value"] == 9.0  # newest ingestion, not 4.0
