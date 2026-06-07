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
