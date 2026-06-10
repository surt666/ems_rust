# Type-Graph Hierarchy Schema (v2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the level-keyed company schema with a type graph so node types (building, group, property, area) can appear at variable depths with type-dependent containment rules.

**Architecture:** `Schema` edges/metadata/sensors are re-keyed from `Level` to type-name strings; node level becomes pure depth (`parent + 1`); nodes gain a persisted `label` (type). Clean break: codecs read/write v2 only, a one-time Python script migrates stored schemas and backfills labels. DAQ pipeline untouched.

**Tech Stack:** Rust (crates/model, crates/services/hierarchy), Python+boto3 (migration), Gherkin (guardrails).

**Spec:** `docs/superpowers/specs/2026-06-10-type-graph-schema-design.md`

**Compile-unit warning:** Tasks 2–6 change the `model` crate's core types; the workspace will NOT compile between Task 2 and the end of Task 6. Do not run `cargo test` mid-phase; commit once at the end of Task 6. Task 1 and Tasks 7+ compile independently.

---

### Task 1: Node gains a `label` field (independent, compiles on its own)

**Files:**
- Modify: `crates/model/src/domain/node.rs`
- Modify: `crates/model/src/repository/dynamodb/codec.rs` (node_to_item / node_of_item)

- [ ] **Step 1: Add the field and test**

In `crates/model/src/domain/node.rs`, add to the `Node` struct (after `metadata`):

```rust
    /// Node type from the company schema ("partner", "company", "building", …).
    /// Empty string when not yet backfilled (pre-migration items).
    #[builder(default)]
    pub label: String,
```

Add to the `tests` module:

```rust
    /// label defaults to empty and is settable.
    #[test]
    fn label_default_and_set() {
        let mut n = make(
            42, Level::Hn3, "x", NodeId::root(), "HN0#root",
            serde_json::json!({}), None,
        );
        assert_eq!(n.label, "");
        n.label = "building".to_string();
        assert_eq!(n.label, "building");
    }
```

- [ ] **Step 2: Persist + read it in the codec**

In `crates/model/src/repository/dynamodb/codec.rs`:

In `node_to_item`, after the `gsi1sk` insert add:

```rust
    if !nd.label.is_empty() {
        item.insert("label".to_string(), s(nd.label.clone()));
    }
```

In `node_of_item`, before the final `Ok(...)` add:

```rust
    let label = match item.get("label") {
        Some(AttributeValue::S(v)) => v.clone(),
        _ => String::new(),
    };
```

and add `.label(label)` to the `Node::builder()` chain.

Find the codec's existing node round-trip test (`grep -n "node_to_item" crates/model/src/repository/dynamodb/codec.rs` — tests near the bottom) and add one:

```rust
    #[test]
    fn node_label_roundtrip() {
        let mut nd = crate::domain::node::make(
            7, Level::Hn3, "B1", NodeId::parse("HN2#1").unwrap(),
            "HN0#root|HN1#1|HN2#1", serde_json::json!({}), None,
        );
        nd.label = "building".to_string();
        let item = node_to_item(&nd);
        let back = node_of_item(&item).unwrap();
        assert_eq!(back.label, "building");
    }

    #[test]
    fn node_missing_label_decodes_empty() {
        let nd = crate::domain::node::make(
            8, Level::Hn3, "B2", NodeId::parse("HN2#1").unwrap(),
            "HN0#root|HN1#1|HN2#1", serde_json::json!({}), None,
        );
        let item = node_to_item(&nd); // label empty → attribute omitted
        let back = node_of_item(&item).unwrap();
        assert_eq!(back.label, "");
    }
```

- [ ] **Step 3: Run tests, expect pass**

Run: `cargo test -p model`
Expected: PASS (all existing + 3 new).

- [ ] **Step 4: Commit**

```bash
git add crates/model/src/domain/node.rs crates/model/src/repository/dynamodb/codec.rs
git commit -m "feat(model): persist node label (type) attribute"
```

---

### Task 2: Schema domain v2 (`schema.rs`) — START OF NON-COMPILING PHASE

**Files:**
- Modify: `crates/model/src/domain/schema.rs`

- [ ] **Step 1: Replace `EdgeSpec` and `Schema`**

Replace the `EdgeSpec` struct (keep `TypedBuilder`, drop `label`):

```rust
/// Cardinality for a directed type edge. The child type name is the key in
/// `Schema::edges`; v1's separate `label` is gone — the child type IS the label.
#[derive(Clone, Debug, PartialEq, TypedBuilder)]
pub struct EdgeSpec {
    #[builder(default)]
    pub min: Option<i32>,
    #[builder(default)]
    pub max: Option<i32>,
}
```

Replace the `Schema` struct and add reserved-name consts:

```rust
/// Reserved type of the hn2 node itself — root of the type graph.
pub const COMPANY_TYPE: &str = "company";
/// Reserved type of hn1 nodes — outside the schema.
pub const PARTNER_TYPE: &str = "partner";

/// A hierarchy schema (v2): a DAG of node types rooted at "company".
/// Levels (hnN) are NOT a schema concept — a node's level is its depth.
#[derive(Clone, Debug, PartialEq, TypedBuilder)]
pub struct Schema {
    pub version: u32,
    /// parent type → (child type → cardinality)
    pub edges: Vec<(String, Vec<(String, EdgeSpec)>)>,
    /// type → (field name → field spec)
    pub metadata: Vec<(String, Vec<(String, FieldSpec)>)>,
    /// types at which sensors may attach
    pub sensors: Vec<String>,
}
```

Remove the now-unused `use crate::domain::ids::Level;` import.

- [ ] **Step 2: Replace the query methods**

```rust
impl Schema {
    /// Allowed child types (with cardinality) under `parent` type.
    pub fn allowed_children(&self, parent: &str) -> &[(String, EdgeSpec)] {
        self.edges
            .iter()
            .find(|(t, _)| t == parent)
            .map(|(_, children)| children.as_slice())
            .unwrap_or(&[])
    }

    /// Cardinality of the `parent` → `child` type edge, if allowed.
    pub fn edge_between(&self, parent: &str, child: &str) -> Option<&EdgeSpec> {
        self.allowed_children(parent)
            .iter()
            .find(|(t, _)| t == child)
            .map(|(_, spec)| spec)
    }

    /// Metadata field specs for a type.
    pub fn metadata_for(&self, typ: &str) -> &[(String, FieldSpec)] {
        self.metadata
            .iter()
            .find(|(t, _)| t == typ)
            .map(|(_, fields)| fields.as_slice())
            .unwrap_or(&[])
    }

    /// Whether sensors may attach to nodes of this type.
    pub fn allows_sensors(&self, typ: &str) -> bool {
        self.sensors.iter().any(|t| t == typ)
    }
}
```

- [ ] **Step 3: Replace `Schema::validate`**

```rust
impl Schema {
    /// Validate v2 invariants:
    /// 1. names non-empty; "partner" nowhere; "company" never a child; no self-edges;
    ///    per parent no duplicate child; min <= max.
    /// 2. the type graph is a DAG.
    /// 3. every referenced type (parents, metadata, sensors) reachable from "company".
    /// 4. longest path from "company" <= 7 edges (deepest node fits hn9).
    /// 5. metadata field specs valid; no duplicate sensors entries.
    pub fn validate(&self) -> Result<(), String> {
        use std::collections::{HashMap, HashSet};

        // 1: local name/cardinality rules
        for (parent, children) in &self.edges {
            if parent.is_empty() {
                return Err("empty parent type name".to_string());
            }
            if parent == PARTNER_TYPE {
                return Err("\"partner\" is reserved and cannot appear in the schema".to_string());
            }
            let mut seen: HashSet<&str> = HashSet::new();
            for (child, spec) in children {
                if child.is_empty() {
                    return Err(format!("edge from {:?}: empty child type name", parent));
                }
                if child == COMPANY_TYPE || child == PARTNER_TYPE {
                    return Err(format!(
                        "edge {} -> {}: reserved type cannot be a child",
                        parent, child
                    ));
                }
                if child == parent {
                    return Err(format!("self edge {} -> {}", parent, child));
                }
                if !seen.insert(child.as_str()) {
                    return Err(format!("duplicate child {} under {}", child, parent));
                }
                if let (Some(a), Some(b)) = (spec.min, spec.max) {
                    if a > b {
                        return Err(format!("edge {} -> {} has min > max", parent, child));
                    }
                }
            }
        }

        // adjacency map
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for (parent, children) in &self.edges {
            let entry = adj.entry(parent.as_str()).or_default();
            for (child, _) in children {
                entry.push(child.as_str());
            }
        }

        // 2: DAG check — 3-color DFS from every declared parent
        #[derive(Clone, Copy, PartialEq)]
        enum Color { White, Gray, Black }
        fn dfs<'a>(
            node: &'a str,
            adj: &HashMap<&'a str, Vec<&'a str>>,
            color: &mut HashMap<&'a str, Color>,
        ) -> Result<(), String> {
            color.insert(node, Color::Gray);
            for next in adj.get(node).map(|v| v.as_slice()).unwrap_or(&[]) {
                match color.get(next).copied().unwrap_or(Color::White) {
                    Color::Gray => return Err(format!("cycle involving type {:?}", next)),
                    Color::White => dfs(next, adj, color)?,
                    Color::Black => {}
                }
            }
            color.insert(node, Color::Black);
            Ok(())
        }
        let mut roots: Vec<&str> = adj.keys().copied().collect();
        roots.sort(); // deterministic error messages
        let mut color: HashMap<&str, Color> = HashMap::new();
        for r in roots {
            if color.get(r).copied().unwrap_or(Color::White) == Color::White {
                dfs(r, &adj, &mut color)?;
            }
        }

        // 3: reachability from "company"
        let mut reach: HashSet<&str> = HashSet::new();
        reach.insert(COMPANY_TYPE);
        let mut queue: Vec<&str> = vec![COMPANY_TYPE];
        while let Some(t) = queue.pop() {
            for c in adj.get(t).map(|v| v.as_slice()).unwrap_or(&[]) {
                if reach.insert(c) {
                    queue.push(c);
                }
            }
        }
        for (parent, _) in &self.edges {
            if !reach.contains(parent.as_str()) {
                return Err(format!("type {:?} not reachable from \"company\"", parent));
            }
        }
        for (typ, _) in &self.metadata {
            if !reach.contains(typ.as_str()) {
                return Err(format!("metadata for unreachable type {:?}", typ));
            }
        }
        let mut seen_sensors: HashSet<&str> = HashSet::new();
        for typ in &self.sensors {
            if !reach.contains(typ.as_str()) {
                return Err(format!("sensors for unreachable type {:?}", typ));
            }
            if !seen_sensors.insert(typ.as_str()) {
                return Err(format!("duplicate sensors entry {:?}", typ));
            }
        }

        // 4: longest path from "company" ≤ 7 (DAG → DFS with memo)
        fn longest<'a>(
            node: &'a str,
            adj: &HashMap<&'a str, Vec<&'a str>>,
            memo: &mut HashMap<&'a str, usize>,
        ) -> usize {
            if let Some(d) = memo.get(node) {
                return *d;
            }
            let d = adj
                .get(node)
                .map(|v| v.iter().map(|c| 1 + longest(c, adj, memo)).max().unwrap_or(0))
                .unwrap_or(0);
            memo.insert(node, d);
            d
        }
        let mut memo: HashMap<&str, usize> = HashMap::new();
        let depth = longest(COMPANY_TYPE, &adj, &mut memo);
        if depth > 7 {
            return Err(format!(
                "longest type chain from \"company\" is {} edges; max 7 (deepest node must fit hn9)",
                depth
            ));
        }

        // 5: metadata field specs
        for (typ, fields) in &self.metadata {
            for (name, spec) in fields {
                validate_spec(spec).map_err(|msg| format!("{}.{}: {}", typ, name, msg))?;
            }
        }
        Ok(())
    }
}
```

(`validate_spec`, `validate_one`, `validate`, `MetadataError` are unchanged.)

- [ ] **Step 4: Replace the test module's schema fixtures and validate tests**

Replace `sample_schema()` in the tests module with the canonical v2 fixture (this exact fixture is reused in Tasks 3, 4, 7, 8 — repeat it verbatim there):

```rust
    fn sample_schema() -> Schema {
        Schema {
            version: 2,
            edges: vec![
                ("company".to_string(), vec![
                    ("group".to_string(), EdgeSpec::builder().build()),
                    ("property".to_string(), EdgeSpec::builder().build()),
                    ("building".to_string(), EdgeSpec::builder().build()),
                ]),
                ("group".to_string(), vec![
                    ("building".to_string(), EdgeSpec::builder().build()),
                ]),
                ("property".to_string(), vec![
                    ("building".to_string(), EdgeSpec::builder().build()),
                ]),
                ("building".to_string(), vec![
                    ("area".to_string(), EdgeSpec::builder().build()),
                ]),
            ],
            metadata: vec![(
                "building".to_string(),
                vec![
                    ("lat".to_string(), FieldSpec {
                        typ: FieldType::Number { min: Some(-90.0), max: Some(90.0) },
                        required: true,
                    }),
                    ("lng".to_string(), FieldSpec {
                        typ: FieldType::Number { min: Some(-180.0), max: Some(180.0) },
                        required: true,
                    }),
                ],
            )],
            sensors: vec!["building".to_string(), "area".to_string()],
        }
    }
```

Replace the level-based validate tests with these (keep all metadata/`validate_one` tests unchanged):

```rust
    #[test]
    fn self_check_accepts_sample() {
        sample_schema().validate().expect("sample schema must validate");
    }

    #[test]
    fn rejects_cycle() {
        let mut bad = sample_schema();
        // area -> group closes a cycle group -> building -> area -> group
        bad.edges.push((
            "area".to_string(),
            vec![("group".to_string(), EdgeSpec::builder().build())],
        ));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("cycle"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_self_edge() {
        let mut bad = sample_schema();
        bad.edges.push((
            "area".to_string(),
            vec![("area".to_string(), EdgeSpec::builder().build())],
        ));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("self edge"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_unreachable_parent() {
        let mut bad = sample_schema();
        bad.edges.push((
            "warehouse".to_string(),
            vec![("area".to_string(), EdgeSpec::builder().build())],
        ));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("not reachable"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_company_as_child() {
        let mut bad = sample_schema();
        bad.edges[1].1.push(("company".to_string(), EdgeSpec::builder().build()));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("reserved"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_partner_anywhere() {
        let mut bad = sample_schema();
        bad.edges.push((
            "partner".to_string(),
            vec![("building".to_string(), EdgeSpec::builder().build())],
        ));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("reserved"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_chain_deeper_than_hn9() {
        // company -> t1 -> t2 -> ... -> t8 = 8 edges (one too many)
        let mut edges: Vec<(String, Vec<(String, EdgeSpec)>)> = Vec::new();
        let mut parent = "company".to_string();
        for i in 1..=8 {
            let child = format!("t{}", i);
            edges.push((parent.clone(), vec![(child.clone(), EdgeSpec::builder().build())]));
            parent = child;
        }
        let bad = Schema { version: 2, edges, metadata: vec![], sensors: vec![] };
        let err = bad.validate().unwrap_err();
        assert!(err.contains("max 7"), "wrong error: {}", err);

        // exactly 7 edges is fine
        let mut edges: Vec<(String, Vec<(String, EdgeSpec)>)> = Vec::new();
        let mut parent = "company".to_string();
        for i in 1..=7 {
            let child = format!("t{}", i);
            edges.push((parent.clone(), vec![(child.clone(), EdgeSpec::builder().build())]));
            parent = child;
        }
        let ok = Schema { version: 2, edges, metadata: vec![], sensors: vec![] };
        ok.validate().expect("7-edge chain must validate");
    }

    #[test]
    fn rejects_duplicate_child_type() {
        let mut bad = sample_schema();
        bad.edges[1].1.push(("building".to_string(), EdgeSpec::builder().build()));
        let err = bad.validate().unwrap_err();
        assert!(err.contains("duplicate child"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_min_greater_than_max() {
        let mut bad = sample_schema();
        bad.edges[3].1[0].1 = EdgeSpec::builder().min(Some(10)).max(Some(5)).build();
        let err = bad.validate().unwrap_err();
        assert!(err.contains("min > max"), "wrong error: {}", err);
    }

    #[test]
    fn rejects_unreachable_sensor_and_metadata_types() {
        let mut bad = sample_schema();
        bad.sensors.push("warehouse".to_string());
        assert!(bad.validate().unwrap_err().contains("unreachable"));

        let mut bad2 = sample_schema();
        bad2.metadata.push(("warehouse".to_string(), vec![]));
        assert!(bad2.validate().unwrap_err().contains("unreachable"));
    }

    #[test]
    fn rejects_duplicate_sensor_entry() {
        let mut bad = sample_schema();
        bad.sensors.push("building".to_string());
        assert!(bad.validate().unwrap_err().contains("duplicate sensors"));
    }

    #[test]
    fn allowed_children_lookup() {
        let s = sample_schema();
        let kids = s.allowed_children("group");
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].0, "building");
        assert!(s.allowed_children("area").is_empty());
        assert_eq!(s.allowed_children("company").len(), 3);
    }

    #[test]
    fn edge_between_lookup() {
        let s = sample_schema();
        assert!(s.edge_between("company", "building").is_some());
        assert!(s.edge_between("group", "area").is_none());
    }

    #[test]
    fn metadata_for_lookup() {
        let s = sample_schema();
        assert_eq!(s.metadata_for("building").len(), 2);
        assert!(s.metadata_for("area").is_empty());
    }

    #[test]
    fn allows_sensors_lookup() {
        let s = sample_schema();
        assert!(s.allows_sensors("building"));
        assert!(s.allows_sensors("area"));
        assert!(!s.allows_sensors("group"));
    }
```

Do NOT run tests yet (crate won't compile until Task 6). Proceed.

---

### Task 3: `hierarchy.rs` — type-keyed add_node, derived levels

**Files:**
- Modify: `crates/model/src/logic/hierarchy.rs`

- [ ] **Step 1: Rewrite level resolution & label handling in `add_node`**

Replace the body between "Fetch parent node" and "Allocate node and write edge atomically" with:

```rust
    let parent_level = parent_node.level();

    // Child level is always parent + 1 (derived, never chosen).
    let derived_level = Level::of_depth(parent_level.depth() + 1)
        .ok_or_else(|| bad("parent is at maximum depth (hn9)"))?;
    if let Some(lv) = level {
        if lv != derived_level {
            return Err(bad(format!(
                "level must be {} (parent level + 1), got {}",
                derived_level, lv
            )));
        }
    }
    let resolved_level = derived_level;

    // Determine the node type (edge label) and (optional) node schema.
    let (edge_label, node_schema) = match parent_level {
        Level::Hn0 => {
            if schema.is_some() {
                return Err(RepositoryError::BadRequest(
                    "schema only allowed on hn2 nodes".to_string(),
                ));
            }
            match label.as_deref() {
                None | Some(crate::domain::schema::PARTNER_TYPE) => {}
                Some(other) => {
                    return Err(bad(format!(
                        "hn1 nodes have reserved type \"partner\", got {:?}",
                        other
                    )))
                }
            }
            (crate::domain::schema::PARTNER_TYPE.to_string(), None)
        }
        Level::Hn1 => {
            let sch = match schema {
                None => {
                    return Err(RepositoryError::BadRequest(
                        "schema is required when creating an hn2 node".to_string(),
                    ))
                }
                Some(s) => match s.validate() {
                    Ok(()) => s,
                    Err(msg) => {
                        return Err(RepositoryError::BadRequest(format!(
                            "invalid schema: {}",
                            msg
                        )))
                    }
                },
            };
            match label.as_deref() {
                None | Some(crate::domain::schema::COMPANY_TYPE) => {}
                Some(other) => {
                    return Err(bad(format!(
                        "hn2 nodes have reserved type \"company\", got {:?}",
                        other
                    )))
                }
            }
            (crate::domain::schema::COMPANY_TYPE.to_string(), Some(sch))
        }
        _ => {
            if schema.is_some() {
                return Err(RepositoryError::BadRequest(
                    "schema only allowed on hn2 nodes".to_string(),
                ));
            }
            let lbl = add_under_schema(
                &parent,
                &parent_node.label,
                label.as_deref(),
                &metadata,
                &get_node_fn,
                list_children_fn,
            )
            .await?;
            (lbl, None)
        }
    };
```

Delete the whole `resolve_child_level` function and the old depth check (`parent_level.depth() >= resolved_level.depth()`) — both are subsumed.

- [ ] **Step 2: Set the child's label in the build closure**

In the `add_node_fn` closure, after `let child = node::make(...)` change to:

```rust
            let mut child = node::make(
                raw_id,
                resolved_level,
                &name.clone(),
                parent_clone.clone(),
                &parent_path,
                metadata.clone(),
                node_schema.clone(),
            );
            child.label = edge_label.clone();
```

(`edge_label` is already captured by the closure for the `EdgeSpec`; keep that usage.)

- [ ] **Step 3: Rewrite `add_under_schema` (type-keyed)**

```rust
/// Validate and select the child type for a schema-governed add.
/// Returns the resolved child type (= edge label) on success.
async fn add_under_schema<FGN, FGNFut, FLC, FLCFut>(
    parent: &NodeId,
    parent_type: &str,
    label: Option<&str>,
    metadata: &serde_json::Value,
    get_node_fn: &FGN,
    list_children_fn: FLC,
) -> Result<String, RepositoryError>
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FLC: FnOnce(NodeId, Option<EdgeKind>) -> FLCFut,
    FLCFut: Future<Output = Result<Vec<Node>, RepositoryError>>,
{
    let (_host, schema) = schema_check::find_for(parent.clone(), get_node_fn).await?;

    let allowed = schema.allowed_children(parent_type);
    let (child_type, spec) = match (label, allowed) {
        (Some(l), _) => match allowed.iter().find(|(t, _)| t == l) {
            Some((t, sp)) => (t.clone(), sp.clone()),
            None => {
                return Err(bad(format!(
                    "type {:?} not allowed under {:?}",
                    l, parent_type
                )))
            }
        },
        (None, [(only, sp)]) => (only.clone(), sp.clone()),
        (None, []) => {
            return Err(bad(format!(
                "no child types allowed under {:?}",
                parent_type
            )))
        }
        (None, _many) => {
            return Err(bad(format!(
                "multiple child types allowed under {:?}; specify label",
                parent_type
            )))
        }
    };

    // Validate metadata against the child type's specs.
    let specs = schema.metadata_for(&child_type);
    validate(specs, metadata).map_err(RepositoryError::Validation)?;

    // Enforce cardinality max.
    if let Some(max) = spec.max {
        let kind = EdgeKind::HasLabel(child_type.clone());
        let existing = list_children_fn(parent.clone(), Some(kind))
            .await
            .unwrap_or_default();
        if existing.len() >= max as usize {
            return Err(bad(format!(
                "max {} {} per parent already reached",
                max, child_type
            )));
        }
    }

    Ok(child_type)
}
```

- [ ] **Step 4: Update the test module**

Replace the test fixture `sample_schema()` with the canonical v2 fixture from Task 2 Step 4 (verbatim). Update existing tests mechanically: level-pair expectations become type expectations. Then add the driving-scenario tests:

```rust
    /// THE driving scenario: building under company (hn3) AND under group (hn4),
    /// children validating as "area" at both depths.
    #[tokio::test]
    async fn building_at_variable_depth() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store).await; // existing helper; company node w/ sample_schema

        // building directly under company → hn3
        let b1 = add_node_helper(&store, c2.clone(), Some("building"), "B1",
            serde_json::json!({"lat": 55.0, "lng": 12.0})).await.unwrap();
        assert_eq!(b1.level(), Level::Hn3);
        assert_eq!(b1.label, "building");

        // group under company → hn3; building under group → hn4
        let g = add_node_helper(&store, c2.clone(), Some("group"), "G",
            serde_json::json!({})).await.unwrap();
        let b2 = add_node_helper(&store, g.id.clone(), Some("building"), "B2",
            serde_json::json!({"lat": 55.0, "lng": 12.0})).await.unwrap();
        assert_eq!(b2.level(), Level::Hn4);
        assert_eq!(b2.label, "building");

        // both buildings allow only "area" children
        let a1 = add_node_helper(&store, b1.id.clone(), None, "A1",
            serde_json::json!({})).await.unwrap();
        assert_eq!(a1.label, "area");
        assert_eq!(a1.level(), Level::Hn4);
        let a2 = add_node_helper(&store, b2.id.clone(), None, "A2",
            serde_json::json!({})).await.unwrap();
        assert_eq!(a2.label, "area");
        assert_eq!(a2.level(), Level::Hn5);

        // group under building is rejected
        let err = add_node_helper(&store, b1.id.clone(), Some("group"), "X",
            serde_json::json!({})).await.unwrap_err();
        assert!(format!("{:?}", err).contains("not allowed under"));
    }

    /// Omitted label: ambiguous under company (3 child types), unique under building.
    #[tokio::test]
    async fn omitted_label_resolution() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store).await;
        let err = add_node_helper(&store, c2.clone(), None, "X",
            serde_json::json!({})).await.unwrap_err();
        assert!(format!("{:?}", err).contains("specify label"));
    }

    /// Explicit level param must equal parent + 1.
    #[tokio::test]
    async fn level_param_must_match_derived() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store).await;
        // company is hn2 → only hn3 children; requesting hn4 is rejected
        let err = add_node_level_helper(&store, c2, Some(Level::Hn4), Some("building"), "B",
            serde_json::json!({"lat": 1.0, "lng": 2.0})).await.unwrap_err();
        assert!(format!("{:?}", err).contains("parent level + 1"));
    }

    /// Metadata required fields enforced per type at any depth.
    #[tokio::test]
    async fn metadata_enforced_per_type() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store).await;
        let err = add_node_helper(&store, c2, Some("building"), "B",
            serde_json::json!({})).await.unwrap_err(); // missing lat/lng
        assert!(matches!(err, RepositoryError::Validation(_)));
    }
```

Adapt the helper names (`seed_company`, `add_node_helper`, `add_node_level_helper`) to whatever the existing test module already uses for wiring `add_node` against `Store` — the existing tests (below line 389) have this wiring; reuse their pattern, do not invent a new harness. Existing tests asserting custom partner/company labels must change to expect the reserved-type rejection instead.

Do NOT run tests yet. Proceed.

---

### Task 4: `sensors.rs` + `schema_check.rs` — type-keyed sensor placement

**Files:**
- Modify: `crates/model/src/logic/sensors.rs`
- Modify: `crates/model/src/logic/schema_check.rs` (tests only — logic unchanged)

- [ ] **Step 1: Sensor placement by node type**

In `sensors.rs` replace the level check (around line 129-137):

```rust
    // Check schema allows sensors on this node type.
    let (_host, schema) = schema_check::find_for(parent.clone(), &get_node).await?;
    if !schema.allows_sensors(&parent_node.label) {
        return Err(validation_err(format!(
            "sensors not allowed on {:?} nodes",
            parent_node.label
        )));
    }
```

Remove the now-unused `let parent_level = parent_node.level();` binding if nothing else uses it.

- [ ] **Step 2: Update both files' test fixtures**

Replace each level-keyed `Schema { ... }` fixture in `sensors.rs` and `schema_check.rs` tests with the canonical v2 fixture from Task 2 Step 4 (verbatim, or trimmed to the edges the test needs — e.g. `schema_check.rs`'s `sample_schema()` becomes):

```rust
    fn sample_schema() -> Schema {
        Schema {
            version: 2,
            edges: vec![
                ("company".to_string(), vec![
                    ("property".to_string(), EdgeSpec::builder().build()),
                ]),
                ("property".to_string(), vec![
                    ("building".to_string(), EdgeSpec::builder().min(Some(1)).build()),
                ]),
            ],
            metadata: vec![],
            sensors: vec![],
        }
    }
```

In `sensors.rs` tests: nodes that sensors attach to must now carry `label` matching a `sensors` entry — after constructing the test node, set `node.label = "building".to_string();` (or whatever type the fixture allows) before putting it in the store. The schema fixture's `sensors` list uses types: `sensors: vec!["building".to_string()]`.

Add one new test. The existing tests in `sensors.rs` (from ~line 412) already wire `attach` against `Store` — copy the setup of the nearest passing `attach` test verbatim and vary only the node labels and assertions as follows:

```rust
    /// Sensors are allowed by node TYPE, regardless of depth.
    /// Setup: company (hn2, label "company", schema with sensors=["building"]),
    /// building B1 at hn3 (label "building"), group G at hn3 (label "group"),
    /// building B2 at hn4 under G (label "building").
    #[tokio::test]
    async fn sensors_by_type_at_any_depth() {
        // <copy the store/company/node setup from the neighbouring attach test;
        //  after each node::make(...), set `node.label = "...".to_string()`
        //  to "building" for B1/B2 and "group" for G before store.put_node>

        // attach to building at hn3 → Ok
        assert!(attach_helper(&store, b1_id.clone()).await.is_ok());
        // attach to building at hn4 → Ok (same type, different depth)
        assert!(attach_helper(&store, b2_id.clone()).await.is_ok());
        // attach to group → rejected, message names the type
        let err = attach_helper(&store, g_id.clone()).await.unwrap_err();
        assert!(format!("{:?}", err).contains("not allowed on \"group\""));
    }
```

(`attach_helper` = whatever closure-bundle helper the neighbouring tests already use to call `sensors::attach`; reuse it unchanged. The one structural addition this test makes over its neighbours is the explicit `label` assignments.)

Do NOT run tests yet. Proceed.

---

### Task 5: Codec v2 — schema encode/decode + v1 rejection

**Files:**
- Modify: `crates/model/src/repository/dynamodb/codec.rs`

- [ ] **Step 1: Replace `edge_spec_to_av` / `schema_to_av`**

`edge_spec_to_av` is unchanged (min/max map). Replace `schema_to_av`:

```rust
/// Encode a `Schema` (v2) as a nested `M` AttributeValue.
/// edges: M{ parent_type -> M{ child_type -> M{min?,max?} } }
pub fn schema_to_av(sch: &Schema) -> AttributeValue {
    let edges_m: HashMap<String, AttributeValue> = sch
        .edges
        .iter()
        .map(|(parent, children)| {
            let inner: HashMap<String, AttributeValue> = children
                .iter()
                .map(|(child, spec)| (child.clone(), edge_spec_to_av(spec)))
                .collect();
            (parent.clone(), AttributeValue::M(inner))
        })
        .collect();

    let metadata_m: HashMap<String, AttributeValue> = sch
        .metadata
        .iter()
        .map(|(typ, fields)| {
            let inner: HashMap<String, AttributeValue> = fields
                .iter()
                .map(|(name, fs)| (name.clone(), field_spec_to_av(fs)))
                .collect();
            (typ.clone(), AttributeValue::M(inner))
        })
        .collect();

    let sensors_l: Vec<AttributeValue> =
        sch.sensors.iter().map(|t| s(t.clone())).collect();

    let mut m: HashMap<String, AttributeValue> = HashMap::new();
    m.insert("version".to_string(), n(sch.version.to_string()));
    m.insert("edges".to_string(), AttributeValue::M(edges_m));
    m.insert("metadata".to_string(), AttributeValue::M(metadata_m));
    m.insert("sensors".to_string(), AttributeValue::L(sensors_l));
    AttributeValue::M(m)
}
```

- [ ] **Step 2: Replace `decode_edge_spec` / `schema_of_av` with explicit v1 rejection**

```rust
fn decode_edge_spec(body: &AttributeValue) -> Result<EdgeSpec, RepositoryError> {
    let kvs = as_m(body)?;
    Ok(EdgeSpec {
        min: opt_int_of_n(kvs.get("min")),
        max: opt_int_of_n(kvs.get("max")),
    })
}

/// Decode a `Schema` (v2 only) from a nested `M` AttributeValue.
/// Version-1 (level-keyed) schemas are rejected with an explicit error.
pub fn schema_of_av(v: &AttributeValue) -> Result<Schema, RepositoryError> {
    let kvs = as_m(v)?;
    let version = match field_map(kvs, "version")? {
        AttributeValue::N(s) => s.parse::<u32>().map_err(|_| {
            RepositoryError::Codec("bad version N".to_string())
        })?,
        _ => return Err(RepositoryError::Codec("expected N for version".to_string())),
    };
    if version != 2 {
        return Err(RepositoryError::Codec(format!(
            "schema version {} — run the v2 migration (scripts/migrate_schema_v2.py); only version 2 is supported",
            version
        )));
    }

    let edges_kvs = as_m(field_map(kvs, "edges")?)?;
    let mut edges: Vec<(String, Vec<(String, EdgeSpec)>)> = Vec::new();
    for (parent, inner_v) in edges_kvs {
        let inner_kvs = as_m(inner_v)?;
        let mut children: Vec<(String, EdgeSpec)> = Vec::new();
        for (child, body) in inner_kvs {
            children.push((child.clone(), decode_edge_spec(body)?));
        }
        edges.push((parent.clone(), children));
    }

    let metadata = match kvs.get("metadata") {
        None => vec![],
        Some(m_v) => {
            let m_kvs = as_m(m_v)?;
            let mut result: Vec<(String, Vec<(String, FieldSpec)>)> = Vec::new();
            for (typ, inner_v) in m_kvs {
                let inner_kvs = as_m(inner_v)?;
                let mut fields: Vec<(String, FieldSpec)> = Vec::new();
                for (fname, spec_v) in inner_kvs {
                    fields.push((fname.clone(), decode_field_spec(spec_v)?));
                }
                result.push((typ.clone(), fields));
            }
            result
        }
    };

    let sensors = match kvs.get("sensors") {
        None => vec![],
        Some(v) => {
            let xs = as_l(v)?;
            let mut result: Vec<String> = Vec::new();
            for item in xs {
                result.push(as_s(item)?.to_string());
            }
            result
        }
    };

    Ok(Schema { version, edges, metadata, sensors })
}
```

Remove the now-unused `Level` import from the schema section if it becomes unused (node section may still need it).

- [ ] **Step 3: Propagate schema decode errors in `node_of_item`**

Change:

```rust
    let schema = match item.get("schema") {
        None => None,
        Some(v) => Some(schema_of_av(v)?),
    };
```

(was `schema_of_av(v).ok()` — silent None hid v1 schemas; clean break wants a loud error).

- [ ] **Step 4: Update codec tests**

Replace level-keyed schema fixtures in the codec test module with the canonical v2 fixture (Task 2 Step 4, verbatim). Add:

```rust
    #[test]
    fn schema_v1_rejected_with_migration_hint() {
        // hand-build a v1-shaped attribute: version 1
        let mut m: HashMap<String, AttributeValue> = HashMap::new();
        m.insert("version".to_string(), n("1".to_string()));
        m.insert("edges".to_string(), AttributeValue::M(HashMap::new()));
        let err = schema_of_av(&AttributeValue::M(m)).unwrap_err();
        let msg = format!("{:?}", err);
        assert!(msg.contains("version 1"), "got: {}", msg);
        assert!(msg.contains("migration"), "got: {}", msg);
    }

    #[test]
    fn schema_v2_roundtrip() {
        let s0 = sample_schema(); // v2 fixture
        let av = schema_to_av(&s0);
        let s1 = schema_of_av(&av).unwrap();
        // av maps are unordered — compare as sets
        assert_eq!(s1.version, 2);
        assert_eq!(s1.edges.len(), s0.edges.len());
        for (parent, children) in &s0.edges {
            let dec = s1.edges.iter().find(|(p, _)| p == parent).expect(parent);
            assert_eq!(dec.1.len(), children.len());
        }
        assert_eq!(
            s1.sensors.iter().collect::<std::collections::HashSet<_>>(),
            s0.sensors.iter().collect::<std::collections::HashSet<_>>()
        );
    }
```

Do NOT run tests yet. Proceed.

---

### Task 6: Make the `model` crate green (end of non-compiling phase)

**Files:**
- Modify: any remaining `crates/model` files with compile errors (expected: `repository/memory.rs` fixtures, leftover `EdgeSpec { label: ... }` literals, unused `Level` imports)

- [ ] **Step 1: Compile and fix fallout**

Run: `cargo build -p model 2>&1 | head -60`

Every remaining error is one of three mechanical fixes:
1. An `EdgeSpec`/`Schema` fixture still level-keyed → replace with the canonical v2 fixture from Task 2 Step 4.
2. A call to a removed API (`edges_between`, `allowed_children(Level)`, `allows_sensors(Level)`) → switch to the type-keyed equivalent (`edge_between(&str, &str)`, `allowed_children(&str)`, `allows_sensors(&str)`).
3. Unused imports (`Level` in schema-only modules) → remove.

- [ ] **Step 2: Run the full model test suite**

Run: `cargo test -p model`
Expected: PASS. If a test asserts old behavior (e.g. custom partner labels, level-keyed lookups), update the assertion to the v2 behavior defined in Tasks 2–5 — do not weaken v2 logic to satisfy an old test.

- [ ] **Step 3: Clippy**

Run: `cargo clippy -p model -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit the whole model-crate migration**

```bash
git add crates/model
git commit -m "feat(model)!: type-graph schema v2 — type-keyed edges, derived levels, DAG validation"
```

---

### Task 7: Hierarchy service JSON — v2 schema shape + node label

**Files:**
- Modify: `crates/services/hierarchy/src/json.rs`

- [ ] **Step 1: Rewrite `schema_to_json`**

```rust
/// Serialise a `Schema` (v2) to JSON:
/// { "version": 2, "edges": { "company": { "building": { "min": 1 } } },
///   "metadata": { "building": { ... } }, "sensors": ["building"] }
pub fn schema_to_json(sch: &Schema) -> Value {
    let edges_obj: serde_json::Map<String, Value> = sch
        .edges
        .iter()
        .map(|(parent, children)| {
            let inner_obj: serde_json::Map<String, Value> = children
                .iter()
                .map(|(child, spec)| {
                    let mut kv = serde_json::Map::new();
                    if let Some(m) = spec.min {
                        kv.insert("min".to_string(), json!(m));
                    }
                    if let Some(m) = spec.max {
                        kv.insert("max".to_string(), json!(m));
                    }
                    (child.clone(), Value::Object(kv))
                })
                .collect();
            (parent.clone(), Value::Object(inner_obj))
        })
        .collect();

    let metadata_obj: serde_json::Map<String, Value> = sch
        .metadata
        .iter()
        .map(|(typ, fields)| {
            let field_obj: serde_json::Map<String, Value> = fields
                .iter()
                .map(|(name, fs)| (name.clone(), field_spec_to_json(fs)))
                .collect();
            (typ.clone(), Value::Object(field_obj))
        })
        .collect();

    let sensors_arr: Vec<Value> =
        sch.sensors.iter().map(|t| Value::String(t.clone())).collect();

    json!({
        "version":  sch.version,
        "edges":    Value::Object(edges_obj),
        "metadata": Value::Object(metadata_obj),
        "sensors":  sensors_arr,
    })
}
```

- [ ] **Step 2: Rewrite `schema_of_json`**

```rust
/// Deserialise a `Schema` from JSON. Only version 2 is accepted.
pub fn schema_of_json(v: &Value) -> Result<Schema, String> {
    let kvs = as_object(v)?;

    let version: u32 = match kvs.get("version") {
        Some(Value::Number(n)) if n.is_u64() => n.as_u64().unwrap() as u32,
        Some(Value::Number(n)) if n.is_i64() => {
            let i = n.as_i64().unwrap();
            if i < 0 { return Err("bad version".to_string()); }
            i as u32
        }
        _ => return Err("missing or non-integer field \"version\"".to_string()),
    };
    if version != 2 {
        return Err(format!(
            "unsupported schema version {}; expected 2 (type-keyed)",
            version
        ));
    }

    let edges_v = kvs.get("edges").ok_or_else(|| "missing field \"edges\"".to_string())?;
    let edges_map = as_object(edges_v)?;
    let mut edges: Vec<(String, Vec<(String, EdgeSpec)>)> = Vec::new();
    for (parent, inner_v) in edges_map {
        let inner_map = as_object(inner_v)?;
        let mut children: Vec<(String, EdgeSpec)> = Vec::new();
        for (child, body) in inner_map {
            let body_map = as_object(body)?;
            let min = body_map.get("min").and_then(opt_i32_of_json);
            let max = body_map.get("max").and_then(opt_i32_of_json);
            children.push((child.clone(), EdgeSpec { min, max }));
        }
        edges.push((parent.clone(), children));
    }

    let metadata: Vec<(String, Vec<(String, FieldSpec)>)> = match kvs.get("metadata") {
        None => vec![],
        Some(m_v) => {
            let m_map = as_object(m_v)?;
            let mut result = Vec::new();
            for (typ, inner_v) in m_map {
                let inner_map = as_object(inner_v)?;
                let mut fields: Vec<(String, FieldSpec)> = Vec::new();
                for (fname, spec_v) in inner_map {
                    fields.push((fname.clone(), decode_field_spec(spec_v)?));
                }
                result.push((typ.clone(), fields));
            }
            result
        }
    };

    let sensors: Vec<String> = match kvs.get("sensors") {
        None => vec![],
        Some(sensors_v) => {
            let arr = as_array(sensors_v)?;
            arr.iter()
                .map(|item| Ok(as_string(item)?.to_string()))
                .collect::<Result<Vec<_>, String>>()?
        }
    };

    Ok(Schema { version, edges, metadata, sensors })
}
```

Remove the now-unused `Level` import if nothing else in the file needs it.

- [ ] **Step 3: Expose `label` in `node_to_json`**

In `node_to_json`, after the `created` insert add:

```rust
    map.insert("label".to_string(), Value::String(n.label.clone()));
```

- [ ] **Step 4: Update json.rs tests**

Replace level-keyed schema JSON fixtures with the v2 shape; add a roundtrip + version-rejection test:

```rust
    #[test]
    fn schema_json_v2_roundtrip() {
        let v = json!({
            "version": 2,
            "edges": {
                "company": { "group": {}, "property": {}, "building": {} },
                "group": { "building": {} },
                "property": { "building": {} },
                "building": { "area": { "min": 1 } }
            },
            "metadata": {
                "building": {
                    "lat": { "type": "number", "required": true, "min": -90.0, "max": 90.0 }
                }
            },
            "sensors": ["building", "area"]
        });
        let sch = schema_of_json(&v).expect("must parse");
        assert!(sch.edge_between("company", "building").is_some());
        assert_eq!(sch.edge_between("building", "area").unwrap().min, Some(1));
        let back = schema_to_json(&sch);
        let sch2 = schema_of_json(&back).expect("roundtrip");
        assert_eq!(sch, sch2);
    }

    #[test]
    fn schema_json_v1_rejected() {
        let v = json!({ "version": 1, "edges": {} });
        let err = schema_of_json(&v).unwrap_err();
        assert!(err.contains("version 1"), "got: {}", err);
    }

    #[test]
    fn node_json_includes_label() {
        let mut nd = model::domain::node::make(
            7, Level::Hn3, "B1", NodeId::parse("HN2#1").unwrap(),
            "HN0#root|HN1#1|HN2#1", serde_json::json!({}), None,
        );
        nd.label = "building".to_string();
        let j = node_to_json(&nd);
        assert_eq!(j["label"], json!("building"));
    }
```

Do NOT run service tests yet (dispatch/query fixtures still v1). Proceed.

---

### Task 8: dispatch.rs + query.rs — fixtures and service green

**Files:**
- Modify: `crates/services/hierarchy/src/dispatch.rs` (test fixtures; `handle_add_node` logic itself is unchanged — equality enforcement lives in `model`)
- Modify: `crates/services/hierarchy/src/query.rs` (seed fixtures)

- [ ] **Step 1: Replace the dispatch test fixtures**

`company_schema()` (line ~1339) and `building_schema()` (line ~1356) become:

```rust
    fn company_schema() -> Schema {
        Schema {
            version: 2,
            edges: vec![
                ("company".to_string(), vec![
                    ("group".to_string(), SchemaEdgeSpec::builder().build()),
                    ("property".to_string(), SchemaEdgeSpec::builder().build()),
                    ("building".to_string(), SchemaEdgeSpec::builder().build()),
                ]),
                ("group".to_string(), vec![
                    ("building".to_string(), SchemaEdgeSpec::builder().build()),
                ]),
                ("property".to_string(), vec![
                    ("building".to_string(), SchemaEdgeSpec::builder().build()),
                ]),
                ("building".to_string(), vec![
                    ("area".to_string(), SchemaEdgeSpec::builder().build()),
                ]),
            ],
            metadata: vec![],
            sensors: vec!["building".to_string(), "area".to_string()],
        }
    }
```

(`building_schema()` — used where a schema with metadata is needed — same shape plus the lat/lng metadata block from Task 2 Step 4, keyed `"building"`.)

The `add_node_parses_api_json_schema_shape` test's inline `schema_json` (line ~1477) becomes the v2 JSON from Task 7 Step 4.

- [ ] **Step 2: Update query.rs seeds**

`seed_with_one_child` (line ~1263) and any other seed building a `Schema` use the same v2 `company_schema()` shape (repeat the code — separate file). Where seeds pass `label` values for children, use type names (`"building"`, `"group"`); where they relied on level-skipping (e.g. hn2 → hn4 direct), change to a contiguous chain (company → building at hn3).

- [ ] **Step 3: Service tests green**

Run: `cargo test -p hierarchy`
Expected: PASS. Same rule as Task 6 Step 2: update assertions to v2 behavior, never weaken v2 logic.

Run: `cargo clippy -p hierarchy -- -D warnings`
Expected: clean.

- [ ] **Step 4: Full workspace check + commit**

Run: `cargo test && cargo clippy -- -D warnings`
Expected: PASS / clean.

```bash
git add crates/services/hierarchy
git commit -m "feat(hierarchy)!: v2 type-keyed schema JSON API; node label exposed"
```

---

### Task 9: Migration script (`scripts/migrate_schema_v2.py`)

**Files:**
- Create: `scripts/migrate_schema_v2.py`
- Create: `scripts/tests/test_migrate_schema_v2.py`

- [ ] **Step 1: Write the failing transform test (SeedCo01 fixture)**

```python
# scripts/tests/test_migrate_schema_v2.py
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from migrate_schema_v2 import transform_schema_v1_to_v2

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
```

- [ ] **Step 2: Run, expect failure**

Run: `cd scripts && uv run --with pytest python -m pytest tests/test_migrate_schema_v2.py -q`
Expected: FAIL ("No module named migrate_schema_v2").

- [ ] **Step 3: Write the script**

```python
#!/usr/bin/env python3
"""One-time migration: hierarchy schemas v1 (level-keyed) -> v2 (type-keyed),
plus node `label` backfill from `has_<label>` edges.

Usage:
  python migrate_schema_v2.py --table hierarchy_new [--apply]

Without --apply it is a DRY RUN: prints every transformed schema and every
label backfill, writes nothing. Requires SSO admin creds for the hierarchy
account (339712745226), eu-central-1.

Spec: docs/superpowers/specs/2026-06-10-type-graph-schema-design.md
"""
import argparse
import json
import sys

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
                if kind.startswith("has_") and kind != "has_sensor":
                    # sk = "<verb>#HN<d>#<id>" → child id is everything after the first '#'
                    child_id = item["sk"].split("#", 1)[1]
                    edges[child_id] = kind[len("has_"):]

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
```

- [ ] **Step 4: Verify the edge-item format assumption**

The script parses edge items as `kind = "has_<label>"` and `sk = "<verb>#<child_id>"`. Confirm
against the encoder before trusting the dry run:

Run: `grep -n "sk_verb\|kind_string" crates/model/src/repository/mod.rs crates/model/src/repository/dynamodb/edge.rs | head -20`

and read `anchor_edge_to_item` in `crates/model/src/repository/dynamodb/codec.rs` (~line 739) to
confirm the `pk`/`sk`/`kind` attribute layout for `HasLabel` edges. If `sk` carries a different
prefix shape, adjust the `item["sk"].split("#", 1)[1]` line accordingly. The dry run output
(LABEL lines showing sensible `HN<d>#<id> -> <type>` pairs) is the final check.

- [ ] **Step 5: Run tests, expect pass**

Run: `cd scripts && uv run --with pytest python -m pytest tests/test_migrate_schema_v2.py -q`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add scripts/migrate_schema_v2.py scripts/tests/test_migrate_schema_v2.py
git commit -m "feat(scripts): schema v1->v2 migration with dry-run + label backfill"
```

---

### Task 10: BDD guardrails

**Files:**
- Create: `features/hierarchy/schema_type_graph.feature`
- Modify: `features/data_pipeline/meter_enrichment.feature` (ENFORCEMENT note)
- Modify: `features/data_pipeline/measurements_rollup.feature` (dense-level note)
- Modify: `features/README.md` (map table)

- [ ] **Step 1: Write the new feature file**

```gherkin
Feature: Company schema as a type graph (v2)
  A company (hn2) schema declares node TYPES and a DAG of allowed containment
  between them, rooted at the reserved type "company". A node's level (hnN) is
  purely its depth — always parent + 1 — so the same type may appear at
  different depths and a node's rules follow its type, not its level.

  Background:
    Given a company schema: company → {group, property, building},
      group → {building}, property → {building}, building → {area}
    And metadata lat/lng required on type "building"
    And sensors allowed on types "building" and "area"

  # source: crates/model hierarchy.rs — building_at_variable_depth
  Scenario: The same type is valid at different depths with the same rules
    When a building is created directly under the company
    Then it sits at hn3 with label "building" and requires lat/lng metadata
    When a building is created under a group
    Then it sits at hn4 with label "building" and requires lat/lng metadata
    And both buildings may only contain "area" children

  # source: crates/model hierarchy.rs — add_under_schema
  Scenario: Containment follows the parent's type, not its depth
    Given a group node and a building node both at hn3
    When an "area" child is requested under each
    Then it is rejected under the group (group allows only building)
    And it is accepted under the building

  # source: crates/model hierarchy.rs — level derivation
  Scenario: A child's level is always the parent's depth plus one
    When any node is created
    Then its level is parent.depth + 1, derived — never chosen by the caller
    And an explicit level parameter that disagrees is rejected
    # paths are therefore always dense/contiguous — the invariant the DAQ
    # rollup's ancestor_keys depends on

  # source: crates/model schema.rs — Schema::validate
  Scenario Outline: Schema validation rejects malformed type graphs
    Given a schema with <defect>
    When it is validated on company creation
    Then it is rejected

    Examples:
      | defect                                              |
      | a cycle between types                               |
      | a self-edge (type contains itself)                  |
      | a type unreachable from "company"                   |
      | an edge targeting the reserved type "company"       |
      | the type "partner" appearing anywhere               |
      | a chain longer than 7 edges from "company" (> hn9)  |
      | duplicate child types under one parent              |
      | min greater than max on an edge                     |

  # source: crates/model sensors.rs — allows_sensors by type
  Scenario: Sensor placement follows node type at any depth
    Given sensors are allowed on type "building"
    Then a sensor attaches to a building at hn3 and to a building at hn4
    And a sensor on a "group" node is rejected

  # source: crates/model codec.rs / json.rs — clean break
  Scenario: Version-1 schemas are rejected loudly, never misread
    Given a stored schema with version 1
    When it is read
    Then the error names the version and points at scripts/migrate_schema_v2.py
```

- [ ] **Step 2: Update the two data_pipeline notes**

In `features/data_pipeline/meter_enrichment.feature`, replace the INVARIANT/ENFORCEMENT comment block (above "The parser tolerates a non-contiguous path...") with:

```gherkin
  # INVARIANT: hn1..hn9 are dense DEPTH indices. Under the v2 type-graph schema
  # (see features/hierarchy/schema_type_graph.feature) a child's level is always
  # the parent's depth + 1, derived by the model layer — so a valid
  # hierarchy_path is contiguous from hn2 with only trailing nulls. The same
  # TYPE (e.g. building) may appear at different depths; never assume a fixed
  # type-per-level mapping (hn4 is not always "building").
```

In `features/data_pipeline/measurements_rollup.feature`, replace the "This holds because..." sentence in the dense-level comment with:

```gherkin
  # at the first None (no interior hole to skip). This holds because the v2
  # type-graph model derives every child's level as parent + 1 — see
  # features/hierarchy/schema_type_graph.feature.
```

- [ ] **Step 3: Add the README map row**

In `features/README.md` map table add:

```markdown
| `hierarchy/schema_type_graph.feature` | v2 type-graph schema: variable-depth types, DAG validation, derived levels | `crates/model` schema/hierarchy/sensors tests, 2026-06-10 spec |
```

- [ ] **Step 4: Commit**

```bash
git add features
git commit -m "docs(bdd): type-graph schema guardrails; update dense-level invariant notes"
```

---

### Task 11: Final verification

- [ ] **Step 1: Full workspace**

Run: `cargo test && cargo clippy -- -D warnings && cargo build`
Expected: all green.

Run: `cd scripts && uv run --with pytest python -m pytest tests/test_migrate_schema_v2.py -q`
Expected: 3 passed.

- [ ] **Step 2: Commit anything outstanding**

```bash
git status --short   # expect clean; commit stragglers if any
```

---

## Deployment runbook (manual — NOT executed by this plan)

Per `CLAUDE.md` deployment policy (test accounts; breaking changes accepted):

1. `cargo lambda build --release --arm64 -p hierarchy`
2. `cd infra/hierarchy && unset GOROOT && cdk diff OcamlHierarchyStack` — confirm only Lambda Code `[~]`
3. `cdk deploy OcamlHierarchyStack --require-approval never`
4. `python scripts/migrate_schema_v2.py --table hierarchy_new` (dry run, review output)
5. `python scripts/migrate_schema_v2.py --table hierarchy_new --apply`
6. Verify: create a building directly under a company via the API; confirm hn3 placement and that an `area` child is accepted under it.

Order of 3 vs 4/5 is free; any disagreement window is tolerated (test accounts).
