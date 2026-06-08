import sys, os
import pytest
sys.path.insert(0, os.path.dirname(__file__))
import handler as h


def test_parse_node_keys_full_path():
    pk, sk = h.parse_node_keys("H#root#HN1#1#HN2#2#HN3#9#HN4#456")
    assert pk == "HN2#2"
    assert sk == "HN2#2|HN3#9|HN4#456"


def test_parse_node_keys_company_only():
    assert h.parse_node_keys("H#root#HN1#1#HN2#2") == ("HN2#2", "HN2#2")


def test_parse_node_keys_no_company_raises():
    with pytest.raises(ValueError):
        h.parse_node_keys("H#root#HN1#1")


def test_gran_of():
    assert h.gran_of("daily") == "d"
    assert h.gran_of("hourly") == "h"
    assert h.gran_of("anything-else") == "h"


def test_bucket_label_utc():
    assert h.bucket_label("2026-06-07T08:45:00Z", "h") == "2026-06-07T08"
    assert h.bucket_label("2026-06-07T08:45:00+00:00", "d") == "2026-06-07"


def test_bucket_to_iso():
    assert h.bucket_to_iso("2026-06-07T08", "h") == "2026-06-07T08:00:00Z"
    assert h.bucket_to_iso("2026-06-07", "d") == "2026-06-07T00:00:00Z"


def test_parse_sk():
    assert h.parse_sk("HN2#2|HN3#9#Energy#d#2026-06-07") == \
        ("HN2#2|HN3#9", "Energy", "d", "2026-06-07")


def test_to_rows_groups_by_purpose_and_keeps_gran():
    items = [
        {"sk": "HN2#2#Energy#d#2026-06-02", "sum": "2.0", "count": "10", "unit": "Wh"},
        {"sk": "HN2#2#Energy#d#2026-06-01", "sum": "1.0", "count": "5", "unit": "Wh"},
        {"sk": "HN2#2#Water#d#2026-06-01", "sum": "9.0", "count": "3", "unit": "m3"},
        {"sk": "HN2#2#Energy#h#2026-06-01T08", "sum": "99.0", "count": "1", "unit": "Wh"},  # wrong gran
    ]
    rows = h.to_rows(items, "HN2#2", "daily", "d")
    # hourly row dropped; Energy sorted by time then Water; alphabetical by purpose
    assert [(r["purpose"], r["timestamp"], r["value"], r["contributor_count"], r["unit"]) for r in rows] == [
        ("Energy", "2026-06-01T00:00:00Z", 1.0, 5, "Wh"),
        ("Energy", "2026-06-02T00:00:00Z", 2.0, 10, "Wh"),
        ("Water", "2026-06-01T00:00:00Z", 9.0, 3, "m3"),
    ]
