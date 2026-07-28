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


def test_build_sk_carries_energy_type_then_purpose_then_bucket_last():
    assert m.build_sk("HN2#2", "electricity", "total", "d", "2026-06-07") == \
        "HN2#2#electricity#total#d#2026-06-07"
    assert m.build_sk("HN2#2|HN3#9|HN4#456", "electricity", "lighting", "h", "2026-06-07T08") == \
        "HN2#2|HN3#9|HN4#456#electricity#lighting#h#2026-06-07T08"


def test_bucket_is_last_so_a_series_is_one_key_range():
    """A fixed (node, energy_type, purpose) must be a pure BETWEEN on the sort key."""
    a = m.build_sk("HN2#2", "electricity", "total", "h", "2026-06-07T08")
    b = m.build_sk("HN2#2", "electricity", "total", "h", "2026-06-07T09")
    prefix = "HN2#2#electricity#total#h#"
    assert a.startswith(prefix) and b.startswith(prefix) and a < b


def test_delimiter_invariant_node_sorts_before_descendants():
    own = m.build_sk("HN2#2|HN3#9|HN4#456", "electricity", "total", "d", "2026-06-07")
    child = m.build_sk("HN2#2|HN3#9|HN4#456|HN5#7", "electricity", "total", "d", "2026-06-07")
    assert own < child


def test_build_gsi1pk_separates_purposes():
    """Without the purpose segment an 'all energy' query would sum a node's total
    together with its own purpose breakdown."""
    assert m.build_gsi1pk(2, "energy", "total") == "HN2#2#energy#total"
    assert m.build_gsi1pk(2, "energy", "lighting") == "HN2#2#energy#lighting"


def test_window_start_is_day_aligned_utc():
    now = datetime(2026, 6, 7, 9, 30, tzinfo=timezone.utc)
    assert m.window_start_iso(now, 1) == "2026-06-06T00:00:00+00:00"
    assert m.window_start_iso(now, 0) == "2026-06-07T00:00:00+00:00"


def test_dimension_of_unit_energy_volume_other():
    for u in ("Wh", "kWh", "MWh", "J", "GJ"):
        assert m.dimension_of_unit(u) == "energy", u
    for u in ("m3", "m³", "L", "liter"):
        assert m.dimension_of_unit(u) == "volume", u
    for u in ("", "pcs", "°C"):
        assert m.dimension_of_unit(u) == "other", u


def test_build_gsi1sk_omits_energy_type():
    # gsi1sk drops the energy_type so a dimension partition ranges across energy types.
    assert m.build_gsi1sk("HN2#2|HN3#9", "h", "2026-06-07T08") == \
        "HN2#2|HN3#9#h#2026-06-07T08"
