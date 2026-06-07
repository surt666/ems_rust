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
    ttl = m.ttl_for("h", "2026-06-07T08")
    end = int(datetime(2026, 6, 7, 9, tzinfo=timezone.utc).timestamp())
    assert ttl == end + 90 * 86400


def test_ttl_day_is_bucket_end_plus_730d():
    ttl = m.ttl_for("d", "2026-06-07")
    end = int(datetime(2026, 6, 8, 0, tzinfo=timezone.utc).timestamp())
    assert ttl == end + 730 * 86400


def test_ancestor_keys_full_depth():
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
    assert own < child


def test_window_start_is_day_aligned_utc():
    now = datetime(2026, 6, 7, 9, 30, tzinfo=timezone.utc)
    assert m.window_start_iso(now, 1) == "2026-06-06T00:00:00+00:00"
    assert m.window_start_iso(now, 0) == "2026-06-07T00:00:00+00:00"
