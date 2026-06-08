"""
Lambda (daq account) behind a public Function URL. Reads the measurements_aggregate DynamoDB
materialized view and returns a per-purpose time series for one hierarchy node over a date range,
at hour or day resolution — for the Resource Insights chart (frontend AggregationChart).

GET /aggregations?level_id=<hierarchy path>&resolution=<hourly|daily>&start=<ISO>&end=<ISO>

  level_id   the selected node's full hierarchy path, e.g.
             "H#root#HN1#1#HN2#2#HN3#9#HN4#456" (or any string containing the HN<n>#<id> segments).
  resolution "hourly" | "daily"
  start,end  ISO-8601 timestamps (inclusive), UTC.

Response: JSON array, one row per (purpose, bucket), sorted by purpose then time:
  [{ level_id, purpose, unit, resolution, timestamp, value, contributor_count }, ...]
"""

import json
import os
import re
from datetime import datetime, timezone

ROLLUP_TABLE = os.environ.get("ROLLUP_TABLE", "measurements_aggregate")
REGION = os.environ.get("AWS_REGION", "eu-central-1")

_HN_RE = re.compile(r"HN(\d+)#(\d+)")

# ── pure helpers (no boto3) ──


def parse_node_keys(level_id):
    """Extract the hierarchy nodes from HN2 down out of a frontend node path. Returns
    (pk, sk_path): pk = 'HN2#<id>' (partition), sk_path = the full '|'-joined path from HN2
    ('HN2#..|HN3#..|..'). Raises ValueError if there is no HN2 (company) segment."""
    segs = ["HN%s#%s" % (n, i) for (n, i) in _HN_RE.findall(level_id or "")]
    hn2_at = next((k for k, s in enumerate(segs) if s.startswith("HN2#")), None)
    if hn2_at is None:
        raise ValueError("level_id has no HN2 (company) segment: %r" % (level_id,))
    path = segs[hn2_at:]
    return path[0], "|".join(path)


def gran_of(resolution):
    return "d" if resolution == "daily" else "h"


def _parse_iso(s):
    return datetime.fromisoformat(s.strip().replace("Z", "+00:00")).astimezone(timezone.utc)


def bucket_label(iso, gran):
    """UTC bucket label for an ISO timestamp: hour 'YYYY-MM-DDThh' | day 'YYYY-MM-DD'."""
    d = _parse_iso(iso)
    return d.strftime("%Y-%m-%dT%H") if gran == "h" else d.strftime("%Y-%m-%d")


def bucket_to_iso(bucket, gran):
    """Bucket label -> ISO-8601 UTC instant (the bucket's start)."""
    fmt = "%Y-%m-%dT%H" if gran == "h" else "%Y-%m-%d"
    return datetime.strptime(bucket, fmt).replace(tzinfo=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_sk(sk):
    """sk = '<node_path>#<purpose>#<gran>#<bucket>' -> (node_path, purpose, gran, bucket)."""
    node_path, purpose, gran, bucket = sk.rsplit("#", 3)
    return node_path, purpose, gran, bucket


def to_rows(items, level_id, resolution, gran):
    """Turn raw aggregate items into the chart's response rows, grouped by purpose, time-sorted.
    Only items at the requested granularity are kept."""
    by_purpose = {}
    for it in items:
        _, purpose, g, bucket = parse_sk(it["sk"])
        if g != gran:
            continue
        by_purpose.setdefault(purpose, []).append((bucket, it))
    rows = []
    for purpose in sorted(by_purpose):
        for bucket, it in sorted(by_purpose[purpose], key=lambda x: x[0]):
            rows.append({
                "level_id": level_id,
                "purpose": purpose,
                "unit": it.get("unit", ""),
                "resolution": resolution,
                "timestamp": bucket_to_iso(bucket, gran),
                "value": float(it["sum"]),
                "contributor_count": int(it["count"]),
            })
    return rows


# ── DynamoDB query + handler ──


def _query_node(pk, sk_path, gran, start_bucket, end_bucket, purpose=None):
    import boto3
    from boto3.dynamodb.conditions import Key, Attr

    table = boto3.resource("dynamodb", region_name=REGION).Table(ROLLUP_TABLE)
    if purpose:
        # Efficient range query: fix <path>#<purpose>#<gran># and range the trailing bucket.
        prefix = "%s#%s#%s#" % (sk_path, purpose, gran)
        kwargs = {
            "KeyConditionExpression":
                Key("pk").eq(pk) & Key("sk").between(prefix + start_bucket, prefix + end_bucket),
        }
    else:
        # No purpose: return all purposes for the node, narrowed to the bucket range.
        kwargs = {
            "KeyConditionExpression": Key("pk").eq(pk) & Key("sk").begins_with(sk_path + "#"),
            "FilterExpression": Attr("bucket").between(start_bucket, end_bucket),
        }
    items = []
    while True:
        resp = table.query(**kwargs)
        items.extend(resp.get("Items", []))
        lek = resp.get("LastEvaluatedKey")
        if not lek:
            break
        kwargs["ExclusiveStartKey"] = lek
    return items


def _resp(status, body):
    # CORS headers are added by the Function URL's CORS config — do NOT set
    # Access-Control-Allow-Origin here too, or the browser sees duplicate headers.
    return {
        "statusCode": status,
        "headers": {"Content-Type": "application/json"},
        "body": json.dumps(body),
    }


def handler(event, _context):
    qs = (event or {}).get("queryStringParameters") or {}
    level_id = qs.get("level_id", "")
    resolution = qs.get("resolution", "hourly")
    purpose = qs.get("purpose") or None
    start = qs.get("start")
    end = qs.get("end")
    if not start or not end:
        return _resp(400, {"error": "start and end are required (ISO-8601)"})

    gran = gran_of(resolution)
    try:
        pk, sk_path = parse_node_keys(level_id)
    except ValueError:
        # node above company level (HN0/HN1) — nothing to aggregate at a single partition.
        return _resp(200, [])

    try:
        start_bucket = bucket_label(start, gran)
        end_bucket = bucket_label(end, gran)
    except ValueError:
        return _resp(400, {"error": "start/end must be ISO-8601 timestamps"})

    items = _query_node(pk, sk_path, gran, start_bucket, end_bucket, purpose)
    return _resp(200, to_rows(items, level_id, resolution, gran))
