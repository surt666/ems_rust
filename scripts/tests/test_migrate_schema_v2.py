# scripts/tests/test_migrate_schema_v2.py
# (sys.path setup for the parent dir lives in conftest.py)
from migrate_schema_v2 import label_for_node, transform_schema_v1_to_v2

SEEDCO01_V1 = {
    "version": 1,
    "edges": {
        "hn2": {"hn3": {"group": {}, "property": {}}},
        "hn3": {"hn4": {"building": {}}},
        "hn4": {"hn5": {"area": {}}},
    },
    "metadata": {
        "hn4": {
            "lat": {"type": "number", "required": True, "min": -90, "max": 90},
            "lng": {"type": "number", "required": True, "min": -180, "max": 180},
        }
    },
    "sensors": ["hn4", "hn5"],
}


def test_seedco01_transform():
    v2, warnings = transform_schema_v1_to_v2(SEEDCO01_V1)
    assert v2["version"] == 2
    assert v2["edges"] == {
        "company": {"group": {}, "property": {}},
        "group": {"building": {}},
        "property": {"building": {}},
        "building": {"area": {}},
    }
    assert set(v2["metadata"].keys()) == {"building"}
    assert v2["metadata"]["building"]["lat"]["required"] is True
    assert v2["sensors"] == ["building", "area"]
    assert warnings == []


def test_cardinality_carries_over():
    v1 = {
        "version": 1,
        "edges": {"hn2": {"hn3": {"building": {"min": 1, "max": 5}}}},
        "metadata": {},
        "sensors": [],
    }
    v2, _ = transform_schema_v1_to_v2(v1)
    assert v2["edges"]["company"]["building"] == {"min": 1, "max": 5}


def test_multi_type_level_metadata_warns_and_replicates():
    v1 = {
        "version": 1,
        "edges": {"hn2": {"hn3": {"group": {}, "property": {}}}},
        "metadata": {"hn3": {"x": {"type": "boolean", "required": False}}},
        "sensors": [],
    }
    v2, warnings = transform_schema_v1_to_v2(v1)
    assert "x" in v2["metadata"]["group"]
    assert "x" in v2["metadata"]["property"]
    assert len(warnings) == 1
    assert "replicated" in warnings[0]


def test_label_for_node_fixed_and_edge_derived():
    edges = {"HN3#42": "building"}
    assert label_for_node("HN1#10001", edges) == "partner"
    assert label_for_node("HN2#10003", edges) == "company"
    assert label_for_node("HN3#42", edges) == "building"
    assert label_for_node("HN4#99", edges) is None  # no incoming edge known
