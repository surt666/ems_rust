#!/usr/bin/env python3
"""One-time migration: hierarchy schemas v1 (level-keyed) -> v2 (type-keyed),
plus node `label` backfill from `has_<label>` edges.

Usage:
  python migrate_schema_v2.py --table hierarchy_new [--apply]

Without --apply it is a DRY RUN: prints every transformed schema and every
label backfill, writes nothing. Requires SSO admin creds for the hierarchy
account (339712745226), eu-central-1.

Spec: docs/superpowers/specs/2026-06-10-type-graph-schema-design.md

Edge-format note (verified against crates/model/src/domain/values.rs and
crates/model/src/repository/dynamodb/codec.rs, 2026-06-10):
  - `kind` attribute stores "has_label:<label>" (e.g. "has_label:building"),
    NOT "has_<label>".  The sk_verb() helper produces "has_<label>" for the
    sk, but kind_string() uses the strum Display impl "has_label:{0}".
  - `sk` attribute = "<sk_verb>#<child_id>", e.g. "has_building#HN3#42".
    split("#", 1)[1] correctly extracts the child NodeId.
  - We parse the label from `kind` by splitting on ":" after "has_label".
"""
import argparse
import json

# ── pure transform (unit-tested; no AWS imports here) ──────────────────────


def _lvl(s):
    """'hn4' -> 4"""
    return int(s[2:])


def transform_schema_v1_to_v2(schema_v1):
    """v1 level-keyed plain-dict schema -> (v2 type-keyed dict, warnings).

    Labels become types. Parent types of a level-pair edge are the labels of
    the edges INTO the parent level ("company" for hn2). Level-keyed
    metadata/sensors map to the types at that level; if a level hosts several
    types the rules replicate to each (warned, for manual review)."""
    edges_v1 = schema_v1.get("edges", {})
    warnings = []

    # types entering each level
    types_at = {2: ["company"]}
    for parent_lvl, children in edges_v1.items():
        for child_lvl, labels in children.items():
            d = _lvl(child_lvl)
            bucket = types_at.setdefault(d, [])
            for label in labels:
                if label not in bucket:
                    bucket.append(label)

    edges_v2 = {}
    for parent_lvl, children in edges_v1.items():
        parents = types_at.get(_lvl(parent_lvl), [])
        if not parents:
            warnings.append(f"edges from {parent_lvl} dropped: no types at that level")
            continue
        for child_lvl, labels in children.items():
            for label, card in labels.items():
                for pt in parents:
                    edges_v2.setdefault(pt, {})[label] = dict(card)

    metadata_v2 = {}
    for lvl_s, fields in schema_v1.get("metadata", {}).items():
        types = types_at.get(_lvl(lvl_s), [])
        if len(types) > 1:
            warnings.append(
                f"metadata at {lvl_s} replicated to types {types} — review manually"
            )
        for t in types:
            metadata_v2.setdefault(t, {}).update(fields)

    sensors_v2 = []
    for lvl_s in schema_v1.get("sensors", []):
        for t in types_at.get(_lvl(lvl_s), []):
            if t not in sensors_v2:
                sensors_v2.append(t)

    return (
        {"version": 2, "edges": edges_v2, "metadata": metadata_v2, "sensors": sensors_v2},
        warnings,
    )


def label_for_node(pk, edge_labels):
    """Determine the label for a node id 'HN<d>#<id>' given a map
    node_id -> incoming edge label. hn1/hn2 are fixed types."""
    depth = int(pk[2 : pk.index("#")])
    if depth == 1:
        return "partner"
    if depth == 2:
        return "company"
    return edge_labels.get(pk)


# ── AWS driver ──────────────────────────────────────────────────────────────


def main():
    import boto3
    from boto3.dynamodb.types import TypeDeserializer, TypeSerializer

    ap = argparse.ArgumentParser()
    ap.add_argument("--table", default="hierarchy_new")
    ap.add_argument("--region", default="eu-central-1")
    ap.add_argument("--apply", action="store_true", help="write changes (default: dry run)")
    args = ap.parse_args()

    ddb = boto3.client("dynamodb", region_name=args.region)
    deser, ser = TypeDeserializer(), TypeSerializer()

    # full scan, bucket by type
    nodes, edges = [], {}
    paginator = ddb.get_paginator("scan")
    for page in paginator.paginate(TableName=args.table):
        for raw in page["Items"]:
            item = {k: deser.deserialize(v) for k, v in raw.items()}
            if item.get("type") == "node":
                nodes.append(item)
            elif item.get("type") == "edge":
                kind = item.get("kind", "")
                # kind attribute format: "has_label:<label>" (e.g. "has_label:building")
                # NOT "has_<label>" — verified against EdgeKind::kind_string() in
                # crates/model/src/domain/values.rs (strum Display: "has_label:{0}").
                if kind.startswith("has_label:") and kind != "has_label:":
                    label = kind[len("has_label:"):]
                    # sk = "<sk_verb>#<child_id>", e.g. "has_building#HN3#42"
                    # sk_verb() returns "has_<label>"; child_id is after first '#'
                    child_id = item["sk"].split("#", 1)[1]
                    edges[child_id] = label

    schema_writes, label_writes = [], []
    for nd in nodes:
        pk = nd["pk"]
        if "schema" in nd and nd["schema"] and int(nd["schema"].get("version", 0)) == 1:
            v2, warnings = transform_schema_v1_to_v2(_plain(nd["schema"]))
            for w in warnings:
                print(f"WARN {pk}: {w}")
            print(f"SCHEMA {pk}:\n{json.dumps(v2, indent=2, default=str)}")
            schema_writes.append((pk, v2))
        lbl = label_for_node(pk, edges)
        if lbl and nd.get("label") != lbl:
            print(f"LABEL {pk}: {nd.get('label', '<missing>')} -> {lbl}")
            label_writes.append((pk, lbl))

    print(f"\n{len(schema_writes)} schema(s), {len(label_writes)} label backfill(s)")
    if not args.apply:
        print("DRY RUN — rerun with --apply to write")
        return

    for pk, v2 in schema_writes:
        ddb.update_item(
            TableName=args.table,
            Key={"pk": {"S": pk}, "sk": {"S": pk}},
            UpdateExpression="SET #s = :s",
            ExpressionAttributeNames={"#s": "schema"},
            ExpressionAttributeValues={":s": ser.serialize(v2)},
        )
    for pk, lbl in label_writes:
        ddb.update_item(
            TableName=args.table,
            Key={"pk": {"S": pk}, "sk": {"S": pk}},
            UpdateExpression="SET #l = :l",
            ExpressionAttributeNames={"#l": "label"},
            ExpressionAttributeValues={":l": {"S": lbl}},
        )
    print("APPLIED")


def _plain(v):
    """Recursively convert Decimal (boto3) to int/float for JSON-friendly dicts."""
    from decimal import Decimal

    if isinstance(v, dict):
        return {k: _plain(x) for k, x in v.items()}
    if isinstance(v, list):
        return [_plain(x) for x in v]
    if isinstance(v, Decimal):
        return int(v) if v == int(v) else float(v)
    return v


if __name__ == "__main__":
    main()
