# Editable Node Metadata Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an Edit/Save control to the node-detail metadata section (Admin/Writer only) that makes the schema-typed metadata inputs writable and persists changes to the node's DynamoDB item via a new `update_node` command.

**Architecture:** New pure `schema::coerce_metadata` (string→schema-type coercion) + `hierarchy::update_node_metadata` logic (validate against the node type's schema, persist via the existing `put_node`). New `Command::UpdateNode` + `dispatch::handle_update_node`. The node-detail fragment (`render_node`) renders an editable `<form>` of typed inputs gated on `capability`; Edit (hyperscript) makes them writable, Save posts the command and re-renders the `#node-data-panel`.

**Tech Stack:** Rust (workspace: `crates/model`, `crates/services/hierarchy`), maud HTML, serde_json, chrono, HTMX + hyperscript, Astro frontend.

**Spec:** `docs/superpowers/specs/2026-06-12-node-metadata-edit-design.md`

**Conventions (read before starting):**
- Repository ops are injected as `async` closures; **no traits** (see `crates/model/src/logic/hierarchy.rs`).
- Run model tests with `cargo test -p model`, service tests with `cargo test -p hierarchy`, all with `cargo test`. Keep `cargo clippy` warning-free.
- HTML is server-rendered maud; HTMX composes it; **never** JSON to the browser for this UI.
- CSS uses grid, never flex.

---

### Task 1: `schema::coerce_metadata` (pure coercion)

**Files:**
- Modify: `crates/model/src/domain/schema.rs` (add `coerce_metadata` + a private `normalize_timestamp`, plus tests in the existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/model/src/domain/schema.rs`:

```rust
#[test]
fn coerce_number_and_integer_from_string() {
    let specs: Vec<(String, FieldSpec)> = vec![
        ("lat".into(), FieldSpec { typ: FieldType::Number { min: None, max: None }, required: true }),
        ("n".into(),   FieldSpec { typ: FieldType::Integer { min: None, max: None }, required: false }),
    ];
    let mut v = json!({ "lat": "55.5", "n": "7" });
    coerce_metadata(&specs, &mut v);
    assert_eq!(v["lat"], json!(55.5));
    assert_eq!(v["n"], json!(7));
}

#[test]
fn coerce_boolean_from_string() {
    let specs: Vec<(String, FieldSpec)> = vec![
        ("b".into(), FieldSpec { typ: FieldType::Boolean, required: false }),
    ];
    let mut v = json!({ "b": "true" });
    coerce_metadata(&specs, &mut v);
    assert_eq!(v["b"], json!(true));
}

#[test]
fn coerce_date_to_rfc3339() {
    let specs: Vec<(String, FieldSpec)> = vec![
        ("ts".into(), FieldSpec { typ: FieldType::Timestamp, required: false }),
    ];
    let mut v = json!({ "ts": "2026-06-12" });
    coerce_metadata(&specs, &mut v);
    assert_eq!(v["ts"], json!("2026-06-12T00:00:00Z"));
    // a value already in RFC3339 is left intact
    let mut v2 = json!({ "ts": "2026-06-12T08:30:00Z" });
    coerce_metadata(&specs, &mut v2);
    assert_eq!(v2["ts"], json!("2026-06-12T08:30:00Z"));
}

#[test]
fn coerce_empty_string_removes_key() {
    let specs: Vec<(String, FieldSpec)> = vec![
        ("lat".into(), FieldSpec { typ: FieldType::Number { min: None, max: None }, required: false }),
    ];
    let mut v = json!({ "lat": "  " });
    coerce_metadata(&specs, &mut v);
    assert!(v.as_object().unwrap().get("lat").is_none());
}

#[test]
fn coerce_leaves_typed_and_undeclared_values() {
    let specs: Vec<(String, FieldSpec)> = vec![
        ("lat".into(), FieldSpec { typ: FieldType::Number { min: None, max: None }, required: false }),
    ];
    // already a number → untouched; undeclared key → untouched
    let mut v = json!({ "lat": 1.0, "other": "x" });
    coerce_metadata(&specs, &mut v);
    assert_eq!(v["lat"], json!(1.0));
    assert_eq!(v["other"], json!("x"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model coerce_ 2>&1 | tail -20`
Expected: compile error — `coerce_metadata` not found.

- [ ] **Step 3: Implement `coerce_metadata` + `normalize_timestamp`**

Add to `crates/model/src/domain/schema.rs` (after the `validate` free function, before the `tests` module). `FieldType` is already imported via `use crate::domain::values::FieldType;` at the top of the file:

```rust
/// Normalize a timestamp string to RFC3339, or return `None` if it is neither
/// RFC3339 nor a bare `YYYY-MM-DD` date.
fn normalize_timestamp(s: &str) -> Option<String> {
    let t = s.trim();
    if chrono::DateTime::parse_from_rfc3339(t).is_ok() {
        return Some(t.to_string());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Some(format!("{}T00:00:00Z", d.format("%Y-%m-%d")));
    }
    None
}

/// Coerce string-encoded form values to their declared schema types, in place.
///
/// Form bodies encode every value as a string; `validate` expects JSON of the
/// declared type. For each declared field present in the object:
/// - empty/whitespace string → the key is removed (blank optional field);
/// - `number`/`integer` string that parses → JSON number;
/// - `boolean` `"true"`/`"false"` → JSON bool;
/// - `timestamp` date-only (`YYYY-MM-DD`) → RFC3339 midnight UTC; RFC3339 kept;
/// - `string`/`enum` and already-typed values are left unchanged.
///
/// Keys are never added; undeclared keys are left untouched.
pub fn coerce_metadata(specs: &[(String, FieldSpec)], v: &mut serde_json::Value) {
    use serde_json::Value;
    let obj = match v.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    for (name, spec) in specs {
        let cur = match obj.get(name) {
            Some(x) => x.clone(),
            None => continue,
        };
        if let Value::String(s) = &cur {
            if s.trim().is_empty() {
                obj.remove(name);
                continue;
            }
        }
        let coerced: Option<Value> = match (&spec.typ, &cur) {
            (FieldType::Number { .. }, Value::String(s)) => s
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number),
            (FieldType::Integer { .. }, Value::String(s)) => {
                s.trim().parse::<i64>().ok().map(|i| Value::Number(i.into()))
            }
            (FieldType::Boolean, Value::String(s)) => match s.trim() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            (FieldType::Timestamp, Value::String(s)) => normalize_timestamp(s).map(Value::String),
            _ => None,
        };
        if let Some(c) = coerced {
            obj.insert(name.clone(), c);
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p model coerce_ 2>&1 | tail -20`
Expected: all 5 `coerce_*` tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/model/src/domain/schema.rs
git commit -m "feat(model): schema::coerce_metadata for form-string metadata coercion"
```

---

### Task 2: `hierarchy::update_node_metadata` (logic) + coerce in add-child path

**Files:**
- Modify: `crates/model/src/logic/hierarchy.rs` (add `update_node_metadata`; call `coerce_metadata` in `add_under_schema`; add tests)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/model/src/logic/hierarchy.rs`. The module already imports `Store`, `node`, `Level`, `NodeId`, `RepositoryError`, and has `get_node_fn`/`seed_company`. Add a `put_node_fn` factory and the tests:

```rust
fn put_node_fn(
    s: Rc<Store>,
) -> impl FnOnce(node::Node) -> std::future::Ready<Result<(), RepositoryError>> {
    move |n| {
        s.put_node(&n);
        std::future::ready(Ok(()))
    }
}

#[tokio::test]
async fn update_metadata_persists_and_coerces() {
    let store = Rc::new(Store::new());
    let c2 = seed_company(&store);
    // building under company (hn3); building metadata requires lat: Number.
    let b = add_node_helper(&store, c2, Some("building"), "B", serde_json::json!({"lat": 1.0}))
        .await
        .expect("add building");

    // Update with a STRING lat (as a form would send) — coercion must make it validate.
    let updated = update_node_metadata(
        b.id.clone(),
        serde_json::json!({ "lat": "55.5" }),
        get_node_fn(store.clone()),
        put_node_fn(store.clone()),
    )
    .await
    .expect("update should succeed");
    assert_eq!(updated.metadata["lat"], serde_json::json!(55.5));

    // Persisted to the store.
    let fetched = store.get_node(&b.id).unwrap();
    assert_eq!(fetched.metadata["lat"], serde_json::json!(55.5));
}

#[tokio::test]
async fn update_metadata_rejects_invalid() {
    let store = Rc::new(Store::new());
    let c2 = seed_company(&store);
    let b = add_node_helper(&store, c2, Some("building"), "B", serde_json::json!({"lat": 1.0}))
        .await
        .expect("add building");

    // lat = 200 is out of range [-90, 90].
    let err = update_node_metadata(
        b.id,
        serde_json::json!({ "lat": "200" }),
        get_node_fn(store.clone()),
        put_node_fn(store.clone()),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RepositoryError::Validation(_)));
}

#[tokio::test]
async fn update_metadata_keeps_only_declared_fields() {
    let store = Rc::new(Store::new());
    let c2 = seed_company(&store);
    let b = add_node_helper(&store, c2, Some("building"), "B", serde_json::json!({"lat": 1.0}))
        .await
        .expect("add building");

    let updated = update_node_metadata(
        b.id,
        serde_json::json!({ "lat": "10", "bogus": "x" }),
        get_node_fn(store.clone()),
        put_node_fn(store.clone()),
    )
    .await
    .expect("update");
    assert_eq!(updated.metadata["lat"], serde_json::json!(10.0));
    assert!(updated.metadata.as_object().unwrap().get("bogus").is_none());
}

#[tokio::test]
async fn update_metadata_missing_node_is_not_found() {
    let store = Rc::new(Store::new());
    let ghost = NodeId::make(Level::Hn3, 99999);
    let err = update_node_metadata(
        ghost,
        serde_json::json!({}),
        get_node_fn(store.clone()),
        put_node_fn(store.clone()),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RepositoryError::NotFound(_)));
}

#[tokio::test]
async fn update_metadata_rejects_hn1() {
    let store = Rc::new(Store::new());
    store.put_node(&node::make_root());
    // hn1 partner node has no schema-governed metadata.
    let p = node::make(
        10001, Level::Hn1, "P", NodeId::root(),
        &NodeId::root().to_string(), serde_json::json!({}), None,
    );
    store.put_node(&p);
    let err = update_node_metadata(
        p.id,
        serde_json::json!({}),
        get_node_fn(store.clone()),
        put_node_fn(store.clone()),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RepositoryError::BadRequest(_)));
}

/// add-child via the form path sends string metadata; coercion must let it validate.
#[tokio::test]
async fn add_under_schema_coerces_string_metadata() {
    let store = Rc::new(Store::new());
    let c2 = seed_company(&store);
    // building requires lat: Number; pass it as a STRING (form encoding).
    let b = add_node_helper(&store, c2, Some("building"), "B", serde_json::json!({"lat": "12.5"}))
        .await
        .expect("string lat should coerce + validate");
    assert_eq!(b.metadata["lat"], serde_json::json!(12.5));
}
```

Note: `seed_company` in this module's tests sets `building` metadata `lat` (Number, -90..90, required). Confirm by reading the existing `sample_schema()`/`seed_company` in the file; if `building` has no `lat` metadata in the logic-test fixture, add a `lat` Number(-90..90, required) field to that fixture's `metadata` before writing these tests.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p model update_metadata 2>&1 | tail -20`
Expected: compile error — `update_node_metadata` not found.

- [ ] **Step 3: Implement `update_node_metadata` and add coercion to `add_under_schema`**

In `crates/model/src/logic/hierarchy.rs`, add the function (after `add_under_schema`):

```rust
// ---------------------------------------------------------------------------
// update_node_metadata
// ---------------------------------------------------------------------------

/// Replace a node's metadata, validated against its type's schema, persisted
/// via `put_node_fn`. Only schema-declared fields are kept.
///
/// - Node missing → `NotFound`.
/// - hn0/hn1 (no schema-governed metadata) → `BadRequest`.
/// - Metadata is coerced (form strings → declared types) then validated.
pub async fn update_node_metadata<FGN, FGNFut, FPN, FPNFut>(
    id: NodeId,
    mut metadata: serde_json::Value,
    get_node_fn: FGN,
    put_node_fn: FPN,
) -> Result<Node, RepositoryError>
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FPN: FnOnce(Node) -> FPNFut,
    FPNFut: Future<Output = Result<(), RepositoryError>>,
{
    let mut node = get_node_fn(id.clone())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(id.clone()))?;

    if node.level().depth() < 2 {
        return Err(RepositoryError::BadRequest(
            "node type has no editable metadata".to_string(),
        ));
    }

    let (_host, schema) = schema_check::find_for(id.clone(), &get_node_fn).await?;
    let specs = schema.metadata_for(&node.label);

    crate::domain::schema::coerce_metadata(specs, &mut metadata);
    validate(specs, &metadata).map_err(RepositoryError::Validation)?;

    // Keep only declared fields.
    let mut out = serde_json::Map::new();
    if let Some(obj) = metadata.as_object() {
        for (name, _spec) in specs {
            if let Some(val) = obj.get(name) {
                out.insert(name.clone(), val.clone());
            }
        }
    }
    node.metadata = serde_json::Value::Object(out);

    put_node_fn(node.clone()).await?;
    Ok(node)
}
```

In `add_under_schema`, add coercion immediately before the existing `validate(specs, metadata)` call. The current code is:

```rust
    // Validate metadata against the child type's specs.
    let specs = schema.metadata_for(&child_type);
    validate(specs, metadata).map_err(RepositoryError::Validation)?;
```

`metadata` there is `&serde_json::Value` (borrowed). Coercion needs `&mut`, so clone-coerce-validate:

```rust
    // Validate metadata against the child type's specs (coercing form strings first).
    let specs = schema.metadata_for(&child_type);
    let mut coerced = metadata.clone();
    crate::domain::schema::coerce_metadata(specs, &mut coerced);
    validate(specs, &coerced).map_err(RepositoryError::Validation)?;
```

Note: `add_under_schema` only resolves/validates the child type; the actual node metadata is written later by the `add_node_fn` builder from the original `metadata`. To persist the *coerced* values on create too, `add_node` must build the node from coerced metadata. Locate the `node::make(..., metadata.clone(), ...)` call inside the `add_node_fn` builder closure and coerce there as well: before constructing the builder closure, compute the coerced metadata once for the schema-governed branch.

Concretely, change the `_ =>` arm of the `match parent_level` in `add_node` so `add_under_schema` returns the coerced metadata alongside the label, OR (simpler) re-coerce in the builder. Implement the simpler approach: have `add_under_schema` return `(String, serde_json::Value)` = (child_type, coerced_metadata), and in `add_node` use that coerced value for `node_schema`/metadata. Update the `_ =>` arm:

```rust
        _ => {
            if schema.is_some() {
                return Err(RepositoryError::BadRequest(
                    "schema only allowed on hn2 nodes".to_string(),
                ));
            }
            let (lbl, coerced_meta) = add_under_schema(
                &parent,
                &parent_node.label,
                label.as_deref(),
                &metadata,
                &get_node_fn,
                list_children_fn,
            )
            .await?;
            metadata = coerced_meta; // persist coerced values
            (lbl, None)
        }
```

For this, make `metadata` a `mut` binding in `add_node`'s signature handling (it is a `serde_json::Value` parameter — change `metadata: serde_json::Value` usage so it can be reassigned: add `let mut metadata = metadata;` at the top of `add_node`, or mark the param `mut`). Change `add_under_schema`'s signature to return `Result<(String, serde_json::Value), RepositoryError>` and have it `Ok((child_type, coerced))`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p model 2>&1 | tail -25`
Expected: the new `update_metadata_*` and `add_under_schema_coerces_string_metadata` tests pass; all pre-existing `add_node` tests still pass (they pass real JSON numbers → coercion is a no-op).

- [ ] **Step 5: Commit**

```bash
git add crates/model/src/logic/hierarchy.rs
git commit -m "feat(model): update_node_metadata logic + coerce metadata on add-child"
```

---

### Task 3: `Command::UpdateNode` (parse)

**Files:**
- Modify: `crates/services/hierarchy/src/command.rs` (add variant + parse test)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/services/hierarchy/src/command.rs`:

```rust
/// `update_node` parses id + metadata (JSON).
#[test]
fn parse_update_node_json() {
    let json = json!({
        "action": "update_node",
        "id": "HN4#10044",
        "metadata": { "lat": "55.5" }
    });
    let cmd: Command = serde_json::from_value(json).unwrap();
    assert!(matches!(&cmd, Command::UpdateNode { id, metadata: Some(_) } if id == "HN4#10044"));
}

/// `update_node` from a form body nests data.metadata.* into a metadata object.
#[test]
fn parse_update_node_form() {
    let body = "action=update_node&data.id=HN4%2310044&data.metadata.lat=55.5";
    let cmd = parse_command(Some("application/x-www-form-urlencoded"), body).unwrap();
    match cmd {
        Command::UpdateNode { id, metadata } => {
            assert_eq!(id, "HN4#10044");
            assert_eq!(metadata.unwrap()["lat"], json!("55.5"));
        }
        other => panic!("wrong variant: {:?}", other),
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p hierarchy parse_update_node 2>&1 | tail -20`
Expected: compile error — no `UpdateNode` variant.

- [ ] **Step 3: Add the variant**

In `crates/services/hierarchy/src/command.rs`, add to the `Command` enum (after `DeleteNode`):

```rust
    /// `update_node` — replace a node's metadata.
    UpdateNode {
        id: String,
        #[serde(default)]
        metadata: Option<Value>,
    },
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p hierarchy parse_update_node 2>&1 | tail -20`
Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/services/hierarchy/src/command.rs
git commit -m "feat(hierarchy): Command::UpdateNode"
```

---

### Task 4: `handle_update_node` + dispatch wiring

**Files:**
- Modify: `crates/services/hierarchy/src/dispatch.rs` (add handler, `run` arm, test)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/services/hierarchy/src/dispatch.rs`. It already has `Store`, `seed_company`, `make_get_node`, `status`, `body`, `handle_add_node`. Add a `put_node` factory and tests:

```rust
fn make_put_node(
    s: Rc<Store>,
) -> impl FnOnce(node::Node) -> std::future::Ready<Result<(), RepositoryError>> {
    move |n| {
        s.put_node(&n);
        std::future::ready(Ok(()))
    }
}

#[tokio::test]
async fn update_node_happy() {
    let store = Rc::new(Store::new());
    let c2 = seed_company(&store);
    // building child with lat metadata.
    let add = handle_add_node(
        c2.to_string(), "B".to_string(), Some("hn3".to_string()),
        Some("building".to_string()), Some(json!({"lat": 1.0})), None,
        make_get_node(store.clone()), make_list_children(store.clone()), make_add_node(store.clone()),
    ).await;
    assert_eq!(status(&add), 200, "add building: {add:?}");
    let id_s = body(&add)["id"].as_str().unwrap().to_string();

    let resp = handle_update_node(
        id_s.clone(),
        Some(json!({ "lat": "42.0" })),
        make_get_node(store.clone()),
        make_put_node(store.clone()),
    ).await;
    assert_eq!(status(&resp), 200, "update: {resp:?}");
    assert_eq!(body(&resp)["metadata"]["lat"].as_f64(), Some(42.0));
}

#[tokio::test]
async fn update_node_validation_400() {
    let store = Rc::new(Store::new());
    let c2 = seed_company(&store);
    let add = handle_add_node(
        c2.to_string(), "B".to_string(), Some("hn3".to_string()),
        Some("building".to_string()), Some(json!({"lat": 1.0})), None,
        make_get_node(store.clone()), make_list_children(store.clone()), make_add_node(store.clone()),
    ).await;
    let id_s = body(&add)["id"].as_str().unwrap().to_string();

    let resp = handle_update_node(
        id_s,
        Some(json!({ "lat": "200" })), // out of range
        make_get_node(store.clone()),
        make_put_node(store.clone()),
    ).await;
    assert_eq!(status(&resp), 400, "expected validation 400: {resp:?}");
}
```

Note: `seed_company` in `dispatch.rs` tests uses `company_schema()` which currently has `metadata: vec![]`. Add a `building` → `lat` Number(-90..90, required) entry to that `company_schema()` fixture's `metadata` so building has an editable/validated field. (Mirror the `sample_schema` in `logic/hierarchy.rs`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p hierarchy update_node_happy update_node_validation 2>&1 | tail -20`
Expected: compile error — `handle_update_node` not found.

- [ ] **Step 3: Implement handler + run arm**

In `crates/services/hierarchy/src/dispatch.rs`, add after `handle_delete_node`:

```rust
// ---------------------------------------------------------------------------
// update_node handler
// ---------------------------------------------------------------------------

pub async fn handle_update_node<FGN, FGNFut, FPN, FPNFut>(
    id: String,
    metadata: Option<Value>,
    get_node: FGN,
    put_node: FPN,
) -> Value
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FPN: FnOnce(Node) -> FPNFut,
    FPNFut: Future<Output = Result<(), RepositoryError>>,
{
    let nid = match NodeId::parse(&id) {
        Ok(i) => i,
        Err(e) => return bad_request(&format!("bad id: {}", e)),
    };
    let meta = metadata.unwrap_or_else(|| json!({}));
    match hierarchy::update_node_metadata(nid, meta, get_node, put_node).await {
        Ok(n) => ok(node_to_json(&n)),
        Err(e) => repo_error_response(e),
    }
}
```

In `dispatch::run`, add a match arm after `Command::DeleteNode { .. } => { ... }`:

```rust
        Command::UpdateNode { id, metadata } => {
            handle_update_node(
                id,
                metadata,
                {
                    let t = table.clone();
                    move |nid| {
                        let t = t.clone();
                        async move { ddb_node::get_node(ddb, &t, &nid).await }
                    }
                },
                {
                    let t = table.clone();
                    move |node| {
                        let t = t.clone();
                        async move { ddb_node::put_node(ddb, &t, &node).await }
                    }
                },
            )
            .await
        }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p hierarchy update_node 2>&1 | tail -20`
Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/services/hierarchy/src/dispatch.rs
git commit -m "feat(hierarchy): handle_update_node + dispatch wiring"
```

---

### Task 5: Shared `forms::metadata_inputs` builder (prefill + disabled)

**Files:**
- Modify: `crates/services/hierarchy/src/html/forms.rs` (add `metadata_inputs`)
- Modify: `crates/services/hierarchy/src/query.rs` (delete local `build_metadata_inputs`, call `forms::metadata_inputs`)

- [ ] **Step 1: Add `forms::metadata_inputs`**

In `crates/services/hierarchy/src/html/forms.rs`, add (it can reuse imports already present; add `use model::domain::schema::FieldSpec;` and `use model::domain::values::FieldType;` if not present):

```rust
/// Render schema-typed metadata form inputs.
///
/// - `prefill`: current values keyed by field name (for the edit form); `None`
///   leaves inputs empty (add-child form).
/// - `disabled`: when true, inputs/selects start disabled and carry the
///   `md-input` class (used by the node-edit form's Edit toggle); add-child
///   passes `false` for byte-identical output to the previous builder.
pub fn metadata_inputs(
    fields: &[(String, FieldSpec)],
    prefill: Option<&serde_json::Map<String, serde_json::Value>>,
    disabled: bool,
) -> Markup {
    use maud::html;

    // Scalar → display string.
    fn sval(v: &serde_json::Value) -> String {
        match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => String::new(),
        }
    }
    let cur = |name: &str| -> Option<String> {
        prefill.and_then(|m| m.get(name)).map(sval)
    };
    // For <input type=date>, prefill wants YYYY-MM-DD.
    let date_part = |s: &str| s.get(0..10).unwrap_or(s).to_string();

    let input_class = if disabled { "form-input md-input" } else { "form-input" };
    let select_class = if disabled { "form-select md-input" } else { "form-select" };

    html! {
        @for (fname, spec) in fields {
            @let req_attr = spec.required;
            @let value = cur(fname);
            div class="form-row" {
                label class="form-label" { (fname) }
                @match &spec.typ {
                    FieldType::String { .. } => {
                        input type="text"
                            name=(format!("data.metadata.{}", fname))
                            class=(input_class)
                            value=[value.clone()]
                            disabled[disabled]
                            required[req_attr];
                    }
                    FieldType::Number { .. } => {
                        input type="number" step="any"
                            name=(format!("data.metadata.{}", fname))
                            class=(input_class)
                            value=[value.clone()]
                            disabled[disabled]
                            required[req_attr];
                    }
                    FieldType::Integer { .. } => {
                        input type="number" step="1"
                            name=(format!("data.metadata.{}", fname))
                            class=(input_class)
                            value=[value.clone()]
                            disabled[disabled]
                            required[req_attr];
                    }
                    FieldType::Boolean => {
                        @let v = value.clone().unwrap_or_default();
                        select
                            name=(format!("data.metadata.{}", fname))
                            class=(select_class)
                            disabled[disabled]
                            required[req_attr]
                        {
                            option value="true" selected[v == "true"] { "true" }
                            option value="false" selected[v == "false"] { "false" }
                        }
                    }
                    FieldType::Timestamp => {
                        input type="date"
                            name=(format!("data.metadata.{}", fname))
                            class=(input_class)
                            value=[value.clone().map(|s| date_part(&s))]
                            disabled[disabled]
                            required[req_attr];
                    }
                    FieldType::Enum { one_of } => {
                        @let v = value.clone().unwrap_or_default();
                        select
                            name=(format!("data.metadata.{}", fname))
                            class=(select_class)
                            disabled[disabled]
                            required[req_attr]
                        {
                            @for opt in one_of {
                                option value=(opt) selected[*opt == v] { (opt) }
                            }
                        }
                    }
                }
                @if spec.required {
                    span class="required" { "*" }
                }
            }
        }
    }
}
```

Note on identical output for add-child: with `prefill = None` and `disabled = false`, `value=[None]` and `disabled[false]` render **nothing**, and `class` is `"form-input"`/`"form-select"` — matching the old `build_metadata_inputs`. The existing `add_child_form_*` tests (which check `option value="..."` for type selectors, not metadata classes) remain green.

- [ ] **Step 2: Replace the local builder in query.rs**

In `crates/services/hierarchy/src/query.rs`, delete the local `fn build_metadata_inputs(...)` (lines ~734-798) and change the `handle_add_child_form` call site from:

```rust
    let metadata_inputs = build_metadata_inputs(&metadata_fields);
```

to:

```rust
    let metadata_inputs = forms::metadata_inputs(&metadata_fields, None, false);
```

(`forms` is already imported: `use crate::html::{forms, node as html_node, tree};`.)

- [ ] **Step 3: Run the add-child tests**

Run: `cargo test -p hierarchy add_child_form 2>&1 | tail -20`
Expected: all `add_child_form_*` tests still pass (identical metadata-input output).

- [ ] **Step 4: Commit**

```bash
git add crates/services/hierarchy/src/html/forms.rs crates/services/hierarchy/src/query.rs
git commit -m "refactor(hierarchy): shared forms::metadata_inputs (prefill + disabled)"
```

---

### Task 6: Editable metadata form in `render_node` + `handle_node` wiring

**Files:**
- Modify: `crates/services/hierarchy/src/html/node.rs` (`render_node` new param + edit form + tests)
- Modify: `crates/services/hierarchy/src/query.rs` (`handle_node` passes `metadata_fields`)
- Modify: `crates/services/hierarchy/tests/node_forms_html.rs` (update `render_node` call sites)

- [ ] **Step 1: Write the failing render_node gating tests**

In `crates/services/hierarchy/src/html/node.rs` `tests` module, add a fields fixture + tests:

```rust
fn lat_fields() -> Vec<(String, model::domain::schema::FieldSpec)> {
    use model::domain::values::FieldType;
    vec![(
        "lat".to_string(),
        model::domain::schema::FieldSpec {
            typ: FieldType::Number { min: Some(-90.0), max: Some(90.0) },
            required: true,
        },
    )]
}

#[test]
fn render_node_writer_with_fields_shows_edit_form() {
    let mut node = make_hn2_node();
    node.metadata = serde_json::json!({ "lat": 55 }); // integer renders as "55"
    let fields = lat_fields();
    let html = render_node(&node, false, true, Some(CognitoGroup::Writer), &fields).into_string();
    assert!(html.contains("id=\"metadata-form\""), "edit form missing");
    assert!(html.contains("update_node"), "update_node action missing");
    assert!(html.contains("data-i18n=\"node.edit\""), "Edit button missing");
    assert!(html.contains("data-i18n=\"node.save\""), "Save button missing");
    assert!(html.contains("value=\"55\""), "lat prefill missing");
}

#[test]
fn render_node_admin_with_fields_shows_edit_form() {
    let mut node = make_hn2_node();
    node.metadata = serde_json::json!({ "lat": 10.0 });
    let html = render_node(&node, false, true, Some(CognitoGroup::Admin), &lat_fields()).into_string();
    assert!(html.contains("id=\"metadata-form\""), "edit form missing for admin");
}

#[test]
fn render_node_reader_with_fields_no_edit_form() {
    let mut node = make_hn2_node();
    node.metadata = serde_json::json!({ "lat": 55.0 });
    let html = render_node(&node, false, true, Some(CognitoGroup::Reader), &lat_fields()).into_string();
    assert!(!html.contains("id=\"metadata-form\""), "reader must not get edit form");
    assert!(!html.contains("update_node"), "reader must not get update_node");
}

#[test]
fn render_node_writer_no_fields_no_edit_form() {
    let node = make_hn2_node(); // empty metadata, no fields
    let html = render_node(&node, false, true, Some(CognitoGroup::Writer), &[]).into_string();
    assert!(!html.contains("id=\"metadata-form\""), "no fields → no edit form");
    assert!(html.contains("No metadata available"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p hierarchy render_node_writer_with_fields 2>&1 | tail -20`
Expected: compile error — `render_node` takes 4 args, tests pass 5.

- [ ] **Step 3: Add the param + edit form to `render_node`**

In `crates/services/hierarchy/src/html/node.rs`:

1. Add the import at the top: `use model::domain::schema::FieldSpec;`
2. Change the signature:

```rust
pub fn render_node(
    node: &Node,
    show_sensors: bool,
    allow_children: bool,
    capability: Option<CognitoGroup>,
    metadata_fields: &[(String, FieldSpec)],
) -> Markup {
```

3. Replace the `let metadata_section = match &node.metadata { ... };` block with:

```rust
    let metadata_section = if can_write && !metadata_fields.is_empty() {
        metadata_edit_form(&nid_str, metadata_fields, &node.metadata)
    } else {
        match &node.metadata {
            serde_json::Value::Object(m) if m.is_empty() => html! {
                p style="color: var(--text-muted);" data-i18n="node.no_metadata" {
                    "No metadata available"
                }
            },
            serde_json::Value::Null => html! {
                p style="color: var(--text-muted);" data-i18n="node.no_metadata" {
                    "No metadata available"
                }
            },
            other => html! { div class="form" { (metadata_rows(other)) } },
        }
    };
```

4. Add the edit-form helper (after `metadata_rows`/`json_scalar_str`, before `add_child_dialog`). The Save button posts the form to `/hierarchy/command`; on success it reloads `#node-data-panel` (defined in `frontend/src/pages/node.astro`), re-rendering the card read-only with persisted values; on error it shows the response text. Edit (hyperscript) removes `@disabled` from the `.md-input` controls and swaps Edit→Save:

```rust
const METADATA_AFTER_REQUEST_JS: &str = "if(event.detail.successful){ \
    htmx.trigger('#node-data-panel','load'); } else { \
    var ed=document.getElementById('metadata-error'); \
    ed.textContent=event.detail.xhr.responseText; ed.style.display='block'; }";

fn metadata_edit_form(
    nid_str: &str,
    fields: &[FieldSpec_pair],
    metadata: &serde_json::Value,
) -> Markup {
    let empty = serde_json::Map::new();
    let prefill = metadata.as_object().unwrap_or(&empty);
    html! {
        form id="metadata-form" class="form"
            data-hx-post="/hierarchy/command"
            data-hx-swap="none"
            data-hx-request=(r#"{"noHeaders": true}"#)
            hx-on--after-request=(METADATA_AFTER_REQUEST_JS)
        {
            input type="hidden" name="action" value="update_node";
            input type="hidden" name="data.id" value=(nid_str);
            (crate::html::forms::metadata_inputs(fields, Some(prefill), true))
            div id="metadata-error" class="login-error" style="display:none; margin-top: 0.5rem;" {}
            div style="display: grid; grid-auto-flow: column; justify-content: end; gap: 0.5rem; margin-top: 1rem;" {
                button type="button" id="metadata-edit-btn" class="btn-secondary"
                    _="on click remove @disabled from <#metadata-form .md-input/> then add @hidden to me then remove @hidden from #metadata-save-btn"
                    data-i18n="node.edit"
                { "Edit" }
                button type="submit" id="metadata-save-btn" class="btn-warning" hidden
                    data-i18n="node.save"
                { "Save" }
            }
        }
    }
}
```

Replace the parameter type `&[FieldSpec_pair]` with the real type `&[(String, FieldSpec)]` (it reads as `FieldSpec_pair` here only to avoid a markdown table-pipe clash). `FieldSpec` is the import added in sub-step 1.

- [ ] **Step 4: Update `handle_node` to pass `metadata_fields`**

In `crates/services/hierarchy/src/query.rs`, inside `handle_node`'s `Ok(n)` arm, the current final line is:

```rust
            html_ok(html_node::render_node(&n, show_sensors, allow_children, capability))
```

Replace it with the metadata-fields computation + updated call:

```rust
            let metadata_fields: Vec<(String, model::domain::schema::FieldSpec)> = schema
                .as_ref()
                .map(|s| s.metadata_for(&n.label).to_vec())
                .unwrap_or_default();
            html_ok(html_node::render_node(
                &n, show_sensors, allow_children, capability, &metadata_fields,
            ))
```

- [ ] **Step 5: Update golden/test call sites**

In `crates/services/hierarchy/tests/node_forms_html.rs`, every `render_node(&node, false, true, Some(CognitoGroup::Admin))` call gains a trailing `&[]` argument, e.g.:

```rust
let rendered = render_node(&node, false, true, Some(CognitoGroup::Admin), &[]).into_string();
```

Update all `render_node(...)` calls in that file the same way. Also update the existing `render_node(...)` calls in `crates/services/hierarchy/src/html/node.rs` `tests` module (the capability-gating tests) to pass a trailing `&[]` (they test the no-fields path).

- [ ] **Step 6: Run the node tests + golden**

Run: `cargo test -p hierarchy 2>&1 | tail -30`
Expected: new `render_node_*` tests pass; `node_detail_matches_golden` still passes (golden node has empty metadata + `&[]` fields → "No metadata available", unchanged). If the golden fails, inspect the diff — it should be unchanged; do **not** edit the golden unless the diff is purely the unchanged no-metadata branch.

- [ ] **Step 7: Commit**

```bash
git add crates/services/hierarchy/src/html/node.rs crates/services/hierarchy/src/query.rs crates/services/hierarchy/tests/node_forms_html.rs
git commit -m "feat(hierarchy): editable metadata form in node view (Admin/Writer)"
```

---

### Task 7: i18n keys

**Files:**
- Modify: `translations/da.json`, `translations/en.json`

- [ ] **Step 1: Add the keys**

In both files, in the `"node"` object, add (after `"delete"`):

`translations/en.json`:
```json
    "edit": "Edit",
    "save": "Save",
```

`translations/da.json`:
```json
    "edit": "Rediger",
    "save": "Gem",
```

- [ ] **Step 2: Verify JSON validity**

Run: `python3 -c "import json; json.load(open('translations/da.json')); json.load(open('translations/en.json')); print('ok')"`
Expected: `ok`

- [ ] **Step 3: Commit**

```bash
git add translations/da.json translations/en.json
git commit -m "i18n: node.edit / node.save"
```

---

### Task 8: Docs — list the new command

**Files:**
- Modify: `docs/architecture.md` (command list in §7), `docs/api.md` (add `update_node`)

- [ ] **Step 1: Update architecture.md command list**

In `docs/architecture.md` §7, change the commands line:

```
add_node · delete_node · attach_sensor · replace_sensor_device
create_user · update_user · delete_user · block_user · unblock_user · grant_administrates
```

to include `update_node` after `delete_node`:

```
add_node · update_node · delete_node · attach_sensor · replace_sensor_device
create_user · update_user · delete_user · block_user · unblock_user · grant_administrates
```

- [ ] **Step 2: Add an `update_node` section to api.md**

In `docs/api.md`, after the `delete_node` section, add:

```markdown
### update_node

Replace a node's metadata. Validated against the node type's schema (with form
strings coerced to their declared types); only schema-declared fields are kept.
hn0/hn1 nodes have no editable metadata.

```json
{
  "action": "update_node",
  "id": "HN4#10044",
  "metadata": { "lat": 55.68, "lng": 12.57 }
}
```

| field    | required | notes                                   |
|----------|----------|-----------------------------------------|
| id       | yes      | `HN<n>#<int>` (hn2 and below)           |
| metadata | no       | default `{}`; validated against the type's schema |

Response — the updated node (same shape as `get_node`).
```

- [ ] **Step 3: Commit**

```bash
git add docs/architecture.md docs/api.md
git commit -m "docs: document update_node command"
```

---

### Task 9: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Full test + lint**

Run:
```bash
cargo test 2>&1 | tail -30
cargo clippy --all-targets 2>&1 | tail -20
```
Expected: all tests pass; clippy warning-free.

- [ ] **Step 2: Frontend build (no code change, but verify nothing broke)**

Run: `cd frontend && npm run build 2>&1 | tail -15`
Expected: build succeeds. (The metadata form is backend-rendered; `node.astro`'s `#node-data-panel` already exists.)

- [ ] **Step 3: Manual smoke (note for the human)**

After deploy (per `CLAUDE.md`), on the live `/node` page as an Admin/Writer: the metadata section shows typed inputs + an **Edit** button; clicking Edit makes them writable and reveals **Save**; Save persists and the panel reloads showing the new values read-only. As a Reader, no Edit button appears.

---

## Notes / decisions baked in

- **Permissions** are UI-gated via `capability` in `render_node`; `update_node` itself does not re-check the caller's group (store-and-report model, consistent with `delete_node`).
- **Coercion** is shared by the edit path and the add-child path, so creating and editing the same field behave identically.
- **Only schema-declared fields** are persisted (Task 2 rebuild), so a direct API caller can't inject undeclared keys.
- **No Astro changes** are required; the form is server-rendered maud and reuses the existing `#node-data-panel`.
