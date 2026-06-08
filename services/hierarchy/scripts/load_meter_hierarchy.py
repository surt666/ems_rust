#!/usr/bin/env python3
"""
Load the meter_hierarchy.json export into the hierarchy_new DynamoDB table.

Mapping (sensors are intentionally NOT loaded):
  company  -> HN2#<company_id> node (under partner HN1#10001) + a generated `schema`,
              and a `HN1#10001 has_company HN2#<id>` edge.
  building.parentId (a group/property not present in the file) -> synthesized
              HN3#<parentId> node + `HN2 has_group HN3#<parentId>` edge.
  building -> HN4#<building_id> node carrying all building fields as `metadata`,
              + `HN3#<parentId> has_building HN4#<id>` edge (or under HN2 when parentId is null).

Item shapes match services/hierarchy/lib/repo/codec.ml:
  node : pk=sk="HN<n>#<id>", type=node, name, created, metadata(M), gsi1pk="HN<n>", gsi1sk=path[, schema]
  edge : pk=<parent>, sk="has_<label>#<child>", type=edge, kind="has_label:<label>",
         name=<child name>, created, gsi1pk="HN<child depth>", gsi1sk=<child path>

Usage:
  python load_meter_hierarchy.py meter_hierarchy.json --company 172          # one company
  python load_meter_hierarchy.py meter_hierarchy.json --all                  # every company
  add --dry-run to print counts/sample without writing.
"""

import argparse
import json
import sys
from datetime import datetime, timezone

TABLE = "hierarchy_new"
REGION = "eu-central-1"
PARTNER_ID = "HN1#10001"
PARTNER_PATH = "HN0#root|" + PARTNER_ID
NOW = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

# building fields that are structural (not stored as node metadata)
META_EXCLUDE = {"id", "name", "parentId", "companyId", "buildingId", "meters"}
GROUP_LABEL = "group"  # synthesized HN3 nodes use the 'group' label (schema also allows 'property')


# ── JSON value -> DynamoDB AttributeValue (mirrors codec.ml json_to_attr) ──

def av(v):
    if v is None:
        return {"NULL": True}
    if isinstance(v, bool):
        return {"BOOL": v}
    if isinstance(v, int):
        return {"N": str(v)}
    if isinstance(v, float):
        return {"N": repr(v)}
    if isinstance(v, str):
        return {"S": v}
    if isinstance(v, list):
        return {"L": [av(x) for x in v]}
    if isinstance(v, dict):
        return {"M": {k: av(x) for k, x in v.items()}}
    return {"S": str(v)}


def building_metadata(b):
    return {"M": {k: av(v) for k, v in b.items() if k not in META_EXCLUDE}}


# ── schema generation ──

def _infer_vtype(values):
    for v in values:
        if v is None:
            continue
        if isinstance(v, bool):
            return "boolean"
        if isinstance(v, int):
            return "integer"
        if isinstance(v, float):
            return "number"
        return "string"
    return "string"


def _field_spec(field, vtype):
    if field == "lat":
        return {"M": {"type": {"S": "number"}, "min": {"N": "-90"}, "max": {"N": "90"}, "required": {"BOOL": False}}}
    if field == "lng":
        return {"M": {"type": {"S": "number"}, "min": {"N": "-180"}, "max": {"N": "180"}, "required": {"BOOL": False}}}
    return {"M": {"type": {"S": vtype}, "required": {"BOOL": False}}}


def global_metadata_fields(data):
    """Union of building metadata fields across all companies -> inferred type."""
    seen = {}
    for c in data.values():
        for b in c.get("buildings", {}).values():
            for k, v in b.items():
                if k in META_EXCLUDE:
                    continue
                seen.setdefault(k, []).append(v)
    return {k: _infer_vtype(vs) for k, vs in sorted(seen.items())}


def build_schema(metadata_fields):
    edges = {"M": {
        "hn2": {"M": {
            "hn3": {"M": {"group": {"M": {}}, "property": {"M": {}}}},
            "hn4": {"M": {"building": {"M": {}}}},  # buildings with parentId=null attach to the company
        }},
        "hn3": {"M": {"hn4": {"M": {"building": {"M": {}}}}}},
        "hn4": {"M": {"hn5": {"M": {"area": {"M": {}}}}}},
    }}
    meta_hn4 = {"M": {f: _field_spec(f, t) for f, t in metadata_fields.items()}}
    return {"M": {
        "version": {"N": "1"},
        "edges": edges,
        "metadata": {"M": {"hn4": meta_hn4}},
        "sensors": {"L": [{"S": "hn4"}, {"S": "hn5"}]},
    }}


# ── item builders ──

def node_item(node_id, name, path, metadata_av, schema=None):
    level = node_id.split("#", 1)[0]  # "HN2"
    item = {
        "pk": {"S": node_id}, "sk": {"S": node_id}, "type": {"S": "node"},
        "name": {"S": name}, "created": {"S": NOW},
        "metadata": metadata_av, "gsi1pk": {"S": level}, "gsi1sk": {"S": path},
    }
    if schema is not None:
        item["schema"] = schema
    return item


def edge_item(parent_id, child_id, label, child_name, child_path):
    child_level = child_id.split("#", 1)[0]
    return {
        "pk": {"S": parent_id}, "sk": {"S": "has_%s#%s" % (label, child_id)},
        "type": {"S": "edge"}, "kind": {"S": "has_label:%s" % label},
        "name": {"S": child_name}, "created": {"S": NOW},
        "gsi1pk": {"S": child_level}, "gsi1sk": {"S": child_path},
    }


def items_for_company(cid, company, metadata_fields):
    items = []
    cname = company.get("name") or ("Company %s" % cid)
    c_node = "HN2#%s" % cid
    c_path = "%s|%s" % (PARTNER_PATH, c_node)
    items.append(node_item(c_node, cname, c_path, {"M": {}}, build_schema(metadata_fields)))
    items.append(edge_item(PARTNER_ID, c_node, "company", cname, c_path))

    buildings = company.get("buildings", {})
    parents = sorted({b.get("parentId") for b in buildings.values()} - {None})
    for pid in parents:
        g_node = "HN3#%s" % pid
        g_path = "%s|%s" % (c_path, g_node)
        gname = "Group %s" % pid
        items.append(node_item(g_node, gname, g_path, {"M": {}}))
        items.append(edge_item(c_node, g_node, GROUP_LABEL, gname, g_path))

    for b in buildings.values():
        pid = b.get("parentId")
        if pid is None:
            parent_node, parent_path = c_node, c_path
        else:
            parent_node = "HN3#%s" % pid
            parent_path = "%s|%s" % (c_path, parent_node)
        b_node = "HN4#%s" % b["id"]
        b_path = "%s|%s" % (parent_path, b_node)
        bname = b.get("name") or ("Building %s" % b["id"])
        items.append(node_item(b_node, bname, b_path, building_metadata(b)))
        items.append(edge_item(parent_node, b_node, "building", bname, b_path))
    return items


def write_items(items):
    import boto3
    client = boto3.client("dynamodb", region_name=REGION)
    for i in range(0, len(items), 25):
        req = {TABLE: [{"PutRequest": {"Item": it}} for it in items[i:i + 25]]}
        resp = client.batch_write_item(RequestItems=req)
        unp = resp.get("UnprocessedItems") or {}
        while unp:
            resp = client.batch_write_item(RequestItems=unp)
            unp = resp.get("UnprocessedItems") or {}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--company", help="single company id (top-level key)")
    ap.add_argument("--all", action="store_true", help="load every company")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    with open(args.file) as fh:
        data = json.load(fh)
    meta_fields = global_metadata_fields(data)

    if args.company:
        cids = [args.company]
    elif args.all:
        cids = list(data.keys())
    else:
        ap.error("pass --company <id> or --all")

    items = []
    for cid in cids:
        if cid not in data:
            print("company %s not in file" % cid, file=sys.stderr)
            continue
        items += items_for_company(cid, data[cid], meta_fields)

    nodes = sum(1 for it in items if it["type"]["S"] == "node")
    edges = sum(1 for it in items if it["type"]["S"] == "edge")
    print("companies=%d  items=%d  (nodes=%d edges=%d)" % (len(cids), len(items), nodes, edges))
    print("metadata schema fields (%d): %s" % (len(meta_fields), meta_fields))

    if args.dry_run:
        print(json.dumps(items[:4], indent=1))
        return
    write_items(items)
    print("wrote %d items to %s" % (len(items), TABLE))


if __name__ == "__main__":
    main()
