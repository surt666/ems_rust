import sys, os
from datetime import datetime, timezone
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import measurements_aggregate as m
from pyspark.sql import types as T


def _input(spark):
    """Two sensors under company HN2#2. 10009 reads 4+6 in the 08 hour, 10010 reads 5."""
    schema = T.StructType([
        T.StructField("hn2", T.IntegerType()),
        T.StructField("logical_id", T.IntegerType()),
        T.StructField("energy_type", T.StringType()),
        T.StructField("unit", T.StringType()),
        T.StructField("value", T.DoubleType()),
        T.StructField("timestamp", T.TimestampType()),
    ])

    def ts(h, mi=0):
        return datetime(2026, 6, 7, h, mi, tzinfo=timezone.utc)

    rows = [
        (2, 10009, "electricity", "kWh", 4.0, ts(8, 15)),
        (2, 10009, "electricity", "kWh", 6.0, ts(8, 45)),
        (2, 10010, "electricity", "kWh", 5.0, ts(8, 30)),
    ]
    return spark.createDataFrame(rows, schema)


def _matrix():
    """Two nodes. HN2#2 sums both sensors; HN2#2|HN3#9 has only 10009. Lighting claims
    10009 and rolls up. Unallocated arrives pre-computed by crates/model."""
    def row(node, pur, sid, c):
        return {"node_path": node, "energy_type": "electricity", "purpose": pur,
                "sensor_id": sid, "coefficient": c}
    return [
        row("HN2#2", "total", 10009, 1.0), row("HN2#2", "total", 10010, 1.0),
        row("HN2#2|HN3#9", "total", 10009, 1.0),
        row("HN2#2", "lighting", 10009, 1.0),
        row("HN2#2|HN3#9", "lighting", 10009, 1.0),
        row("HN2#2", "unallocated", 10010, 1.0),
    ]


def _by_sk(df):
    return {r["sk"]: r for r in df.collect()}


def test_sort_key_carries_the_purpose_segment(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    assert "HN2#2#electricity#total#h#2026-06-07T08" in out
    assert "HN2#2|HN3#9#electricity#lighting#h#2026-06-07T08" in out


def test_value_is_the_weighted_sum_of_the_matrix(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    # 10009 contributes 4+6 = 10, 10010 contributes 5.
    assert out["HN2#2#electricity#total#h#2026-06-07T08"]["sum"] == 15.0
    assert out["HN2#2|HN3#9#electricity#total#h#2026-06-07T08"]["sum"] == 10.0


def test_unallocated_needs_no_arithmetic_in_the_job(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2#electricity#unallocated#h#2026-06-07T08"]["sum"] == 5.0


def test_a_negative_coefficient_subtracts(spark):
    matrix = [r for r in _matrix() if r["purpose"] == "total"]
    matrix.append({"node_path": "HN2#2|HN3#9", "energy_type": "electricity",
                   "purpose": "total", "sensor_id": 10010, "coefficient": -1.0})
    out = _by_sk(m.build_rollups(_input(spark), matrix, run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2|HN3#9#electricity#total#h#2026-06-07T08"]["sum"] == 5.0


def test_a_sensor_cannot_cross_energy_types(spark):
    """The join is on (logical_id, energy_type), so a district-heating formula naming an
    electricity sensor contributes nothing rather than silently mixing carriers."""
    matrix = [{"node_path": "HN2#2", "energy_type": "district_heating", "purpose": "total",
               "sensor_id": 10009, "coefficient": 1.0}]
    out = _by_sk(m.build_rollups(_input(spark), matrix, run_at_iso="2026-06-07T09:05:00Z"))
    assert out == {}


def test_gsi1pk_carries_the_purpose(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    row = out["HN2#2#electricity#total#h#2026-06-07T08"]
    assert row["gsi1pk"] == "HN2#2#energy#total"
    assert row["gsi1sk"] == "HN2#2#h#2026-06-07T08"


def test_day_and_hour_buckets_are_both_emitted(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    assert out["HN2#2#electricity#total#d#2026-06-07"]["sum"] == 15.0


def test_rollup_is_idempotent(spark):
    a = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    b = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T10:05:00Z"))
    assert a.keys() == b.keys()
    for k in a:
        assert a[k]["sum"] == b[k]["sum"] and a[k]["count"] == b[k]["count"]


def test_min_max_and_last_value_are_gone(spark):
    df = m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z")
    for dropped in ("min", "max", "last_value", "last_ts"):
        assert dropped not in df.columns
    assert "count" in df.columns


def test_ttl_and_pk_present(spark):
    out = _by_sk(m.build_rollups(_input(spark), _matrix(), run_at_iso="2026-06-07T09:05:00Z"))
    row = out["HN2#2#electricity#total#d#2026-06-07"]
    assert row["pk"] == "HN2#2"
    assert row["ttl"] == m.ttl_for("d", "2026-06-07")
    assert row["updated_at"] == "2026-06-07T09:05:00Z"


def test_the_job_holds_no_formula_logic():
    """Guard against formula semantics creeping back into PySpark. The recursion, the two
    defaults, derived-node detection and the Unallocated arithmetic all belong to
    crates/model::logic::formulas.

    Checked structurally, over identifiers rather than raw text: the module docstring
    legitimately says the job does *no* derived-node detection, and a substring match
    fails on its own explanation.
    """
    import ast

    src = open(os.path.join(os.path.dirname(__file__), "..",
                            "measurements_aggregate.py")).read()
    tree = ast.parse(src)

    forbidden = {"ancestor_keys", "coeffs", "allocates", "is_derived", "coeffs_within"}

    defined = {n.name for n in ast.walk(tree)
               if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))}
    assert not (defined & forbidden), defined & forbidden

    referenced = {n.id for n in ast.walk(tree) if isinstance(n, ast.Name)}
    referenced |= {n.attr for n in ast.walk(tree) if isinstance(n, ast.Attribute)}
    assert not (referenced & forbidden), referenced & forbidden

    # Column names are strings, so a stray "allocates" would slip past the AST identifiers.
    literals = {n.value for n in ast.walk(tree)
                if isinstance(n, ast.Constant) and isinstance(n.value, str)}
    assert not (literals & forbidden), literals & forbidden


# ── read-side dedup (unchanged semantics, renamed columns) ──

def _raw(spark, rows):
    schema = T.StructType([
        T.StructField("hn2", T.IntegerType()),
        T.StructField("logical_id", T.IntegerType()),
        T.StructField("energy_type", T.StringType()),
        T.StructField("unit", T.StringType()),
        T.StructField("value", T.DoubleType()),
        T.StructField("timestamp", T.TimestampType()),
        T.StructField("reading_kind", T.StringType()),
        T.StructField("ingested_time", T.TimestampType()),
    ])
    return spark.createDataFrame(rows, schema)


def test_latest_counters_dedup_and_filters(spark):
    rt = datetime(2026, 6, 7, 8, tzinfo=timezone.utc)

    def ing(h):
        return datetime(2026, 6, 7, h, tzinfo=timezone.utc)

    rows = [
        # same point (10009, rt): original 4, then a later-ingested restatement 9 -> 9 wins
        (2, 10009, "electricity", "kWh", 4.0, rt, "counter", ing(9)),
        (2, 10009, "electricity", "kWh", 9.0, rt, "counter", ing(11)),
        # a gauge reading is a level, not a delta -> excluded
        (2, 10010, "district_heating", "kWh", 5.0, rt, "gauge", ing(9)),
        # null company id -> excluded
        (None, 10011, "electricity", "kWh", 3.0, rt, "counter", ing(9)),
    ]
    out = m.latest_counters(_raw(spark, rows)).collect()
    assert len(out) == 1
    assert out[0]["value"] == 9.0
