//! Hierarchy logic.
//!
//! All repository operations are injected as async closures; no traits are used.

use std::future::Future;

use crate::domain::ids::{Level, NodeId};
use crate::domain::node::{self, Node};
use crate::domain::schema::{validate, Schema};
use crate::domain::values::EdgeKind;
use crate::errors::RepositoryError;
use crate::logic::schema_check;
use crate::repository::EdgeSpec;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn bad(message: impl Into<String>) -> RepositoryError {
    RepositoryError::Validation(vec![crate::domain::schema::MetadataError {
        path: String::new(),
        message: message.into(),
    }])
}

// ---------------------------------------------------------------------------
// get_node
// ---------------------------------------------------------------------------

/// Fetch a node by id, returning `Err(NotFound)` if absent.
pub async fn get_node<FGN, FGNFut>(
    id: NodeId,
    get_node_fn: FGN,
) -> Result<Node, RepositoryError>
where
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
{
    let id_clone = id.clone();
    get_node_fn(id)
        .await?
        .ok_or(RepositoryError::NotFound(id_clone))
}

// ---------------------------------------------------------------------------
// list_children / list_child_refs
// ---------------------------------------------------------------------------

/// List children of `parent`, optionally filtered by `label`.
pub async fn list_children<FLC, FLCFut>(
    parent: NodeId,
    label: Option<String>,
    list_children_fn: FLC,
) -> Result<Vec<Node>, RepositoryError>
where
    FLC: FnOnce(NodeId, Option<EdgeKind>) -> FLCFut,
    FLCFut: Future<Output = Result<Vec<Node>, RepositoryError>>,
{
    let kind = label.map(EdgeKind::HasLabel);
    list_children_fn(parent, kind).await
}

/// List (child_id, edge_name) refs under `parent`, optionally filtered by `label`.
pub async fn list_child_refs<FLC, FLCFut>(
    parent: NodeId,
    label: Option<String>,
    list_child_refs_fn: FLC,
) -> Result<Vec<(NodeId, String)>, RepositoryError>
where
    FLC: FnOnce(NodeId, Option<EdgeKind>) -> FLCFut,
    FLCFut: Future<Output = Result<Vec<(NodeId, String)>, RepositoryError>>,
{
    let kind = label.map(EdgeKind::HasLabel);
    list_child_refs_fn(parent, kind).await
}

// ---------------------------------------------------------------------------
// add_node
// ---------------------------------------------------------------------------

/// Add a child node under `parent`.
///
/// Rules:
/// - `level = Hn0` → `BadRequest` (cannot create root).
/// - Parent not found → `NotFound`.
/// - The child level is always `parent + 1` (derived); an explicit `level`
///   must equal that or the call is rejected.
/// - Parent is root (Hn0): child is type "partner"; schema must be `None`; an
///   explicit label must be `None` or "partner".
/// - Parent is Hn1: child is type "company"; `schema` is **required** and
///   validated; an explicit label must be `None` or "company".
/// - Otherwise: call `schema_check::find_for`, resolve the child type from the
///   parent type's `allowed_children` (see `add_under_schema`); `schema` must
///   be `None` here.
///
/// For schema-governed children, metadata is validated against
/// `schema.metadata_for(child_type)` and cardinality (`max`) is enforced by
/// counting existing children of that type.  Finally the node is allocated
/// (`add_node`) and the edge written atomically.
#[allow(clippy::too_many_arguments)]
pub async fn add_node<FGN, FGNFut, FLC, FLCFut, FAN, FANFut>(
    parent: NodeId,
    level: Option<Level>,
    label: Option<String>,
    name: String,
    mut metadata: serde_json::Value,
    schema: Option<Schema>,
    get_node_fn: FGN,
    list_children_fn: FLC,
    add_node_fn: FAN,
) -> Result<Node, RepositoryError>
where
    // get_node must be callable multiple times (Fn, not FnOnce) because
    // schema_check::find_for may call it for both the child node AND its HN2.
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FLC: FnOnce(NodeId, Option<EdgeKind>) -> FLCFut,
    FLCFut: Future<Output = Result<Vec<Node>, RepositoryError>>,
    FAN: FnOnce(Level, Box<dyn Fn(u32) -> (Node, EdgeSpec) + Send>) -> FANFut,
    FANFut: Future<Output = Result<Node, RepositoryError>>,
{
    // Reject creating root via add_node.
    if level == Some(Level::Hn0) {
        return Err(RepositoryError::BadRequest(
            "cannot create root via add_node".to_string(),
        ));
    }

    // Fetch parent node.
    let parent_node = get_node_fn(parent.clone())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(parent.clone()))?;

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
    };

    // Allocate node and write edge atomically.
    // The closure is `Fn` (not `FnOnce`) so it can be invoked on every retry
    // of the counter's ConditionalCheckFailed loop — each call clones its
    // captured state independently.
    let parent_path = parent_node.path.clone();
    let parent_clone = parent.clone();

    add_node_fn(
        resolved_level,
        Box::new(move |raw_id| {
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
            let edge = EdgeSpec {
                from_: parent_clone.to_string(),
                to_: child.id.to_string(),
                kind: EdgeKind::HasLabel(edge_label.clone()),
                name: child.name.clone(),
            };
            (child, edge)
        }),
    )
    .await
}

// ---------------------------------------------------------------------------
// add_under_schema (internal)
// ---------------------------------------------------------------------------

/// Validate and select the child type for a schema-governed add.
/// Returns the resolved child type (= edge label) on success.
///
/// The child type is resolved from the parent type's `allowed_children`:
/// an explicit label must be allowed; an omitted label is accepted only when
/// exactly one child type exists. Metadata is validated against the child
/// type's specs and the edge's `max` cardinality is enforced.
async fn add_under_schema<FGN, FGNFut, FLC, FLCFut>(
    parent: &NodeId,
    parent_type: &str,
    label: Option<&str>,
    metadata: &serde_json::Value,
    get_node_fn: &FGN,
    list_children_fn: FLC,
) -> Result<(String, serde_json::Value), RepositoryError>
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

    // Validate metadata against the child type's specs (coercing form strings first).
    let specs = schema.metadata_for(&child_type);
    let mut coerced = metadata.clone();
    crate::domain::schema::coerce_metadata(specs, &mut coerced);
    validate(specs, &coerced).map_err(RepositoryError::Validation)?;

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

    Ok((child_type, coerced))
}

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::domain::ids::{Level, NodeId};
    use crate::domain::node;
    use crate::domain::schema::{EdgeSpec as SchemaEdgeSpec, FieldSpec, Schema};
    use crate::domain::values::{EdgeKind, FieldType};
    use crate::errors::RepositoryError;
    use crate::repository::memory::Store;

    // Canonical v2 type-graph fixture.
    //   company → {group, property, building (max=2)}
    //   group   → building
    //   property→ building
    //   building→ area
    //   building metadata: lat (Number, -90..90, required)
    fn sample_schema() -> Schema {
        Schema {
            version: 2,
            edges: vec![
                ("company".to_string(), vec![
                    ("group".to_string(), SchemaEdgeSpec::builder().build()),
                    ("property".to_string(), SchemaEdgeSpec::builder().max(Some(2)).build()),
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
            metadata: vec![(
                "building".to_string(),
                vec![(
                    "lat".to_string(),
                    FieldSpec {
                        typ: FieldType::Number {
                            min: Some(-90.0),
                            max: Some(90.0),
                        },
                        required: true,
                    },
                )],
            )],
            sensors: vec!["building".to_string(), "area".to_string()],
        }
    }

    fn parent_path_for_hn2() -> String {
        format!(
            "{}|{}",
            NodeId::root(),
            NodeId::make(Level::Hn1, 10001)
        )
    }

    /// Seed an HN2 company node with the sample schema into the store.
    fn seed_company(store: &Rc<Store>) -> NodeId {
        let c2 = NodeId::make(Level::Hn2, 10002);
        let mut n2 = node::make(
            10002,
            Level::Hn2,
            "Acme",
            NodeId::root(),
            &parent_path_for_hn2(),
            serde_json::json!({}),
            Some(sample_schema()),
        );
        n2.label = "company".to_string();
        store.put_node(&n2);
        c2
    }

    // -----------------------------------------------------------------------
    // Closure factories
    // -----------------------------------------------------------------------

    fn get_node_fn(
        s: Rc<Store>,
    ) -> impl Fn(NodeId) -> std::future::Ready<Result<Option<node::Node>, RepositoryError>>
    {
        move |nid| std::future::ready(Ok(s.get_node(&nid)))
    }

    fn list_children_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(NodeId, Option<EdgeKind>) -> std::future::Ready<Result<Vec<node::Node>, RepositoryError>>
    {
        move |nid, kind| std::future::ready(Ok(s.list_children(&nid, kind.as_ref())))
    }

    fn add_node_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(
        Level,
        Box<dyn Fn(u32) -> (node::Node, EdgeSpec) + Send>,
    ) -> std::future::Ready<Result<node::Node, RepositoryError>>
    {
        move |level, build| {
            let n = s.add_node(level, build);
            std::future::ready(Ok(n))
        }
    }

    // -----------------------------------------------------------------------
    // Helper: add_node wrapper with all closures wired to store.
    // -----------------------------------------------------------------------

    async fn do_add_node(
        store: Rc<Store>,
        parent: NodeId,
        level: Option<Level>,
        label: Option<String>,
        name: &str,
        metadata: serde_json::Value,
        schema: Option<Schema>,
    ) -> Result<node::Node, RepositoryError> {
        add_node(
            parent,
            level,
            label,
            name.to_string(),
            metadata,
            schema,
            get_node_fn(store.clone()),
            list_children_fn(store.clone()),
            add_node_fn(store.clone()),
        )
        .await
    }

    /// Add a schema-governed child by type label (no explicit level).
    async fn add_node_helper(
        store: &Rc<Store>,
        parent: NodeId,
        label: Option<&str>,
        name: &str,
        metadata: serde_json::Value,
    ) -> Result<node::Node, RepositoryError> {
        do_add_node(
            store.clone(),
            parent,
            None,
            label.map(str::to_string),
            name,
            metadata,
            None,
        )
        .await
    }

    /// Add a child with an explicit level (to exercise the level-derivation check).
    async fn add_node_level_helper(
        store: &Rc<Store>,
        parent: NodeId,
        level: Option<Level>,
        label: Option<&str>,
        name: &str,
        metadata: serde_json::Value,
    ) -> Result<node::Node, RepositoryError> {
        do_add_node(
            store.clone(),
            parent,
            level,
            label.map(str::to_string),
            name,
            metadata,
            None,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Test: add property + building
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn add_property_and_building() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let prop = add_node_helper(&store, c2, Some("property"), "Ostergade", serde_json::json!({}))
            .await
            .expect("add property should succeed");
        assert_eq!(prop.level(), Level::Hn3);
        assert_eq!(prop.label, "property");

        // property → building (only one child type, label may be omitted).
        let b1 = add_node_helper(
            &store,
            prop.id.clone(),
            None,
            "B1",
            serde_json::json!({"lat": 55.0}),
        )
        .await
        .expect("add building should succeed");

        assert_eq!(
            b1.parent.as_ref().unwrap().to_string(),
            prop.id.to_string(),
            "parent wired correctly"
        );
        assert_eq!(b1.label, "building");
    }

    // -----------------------------------------------------------------------
    // Test: rejects disallowed edge
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_disallowed_edge() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let result = do_add_node(
            store.clone(),
            c2,
            Some(Level::Hn5),
            None,
            "bad",
            serde_json::json!({}),
            None,
        )
        .await;

        match result {
            Err(RepositoryError::Validation(_)) => {} // expected
            Ok(_) => panic!("expected Validation error"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: rejects bad metadata
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_bad_metadata() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let prop = add_node_helper(&store, c2, Some("property"), "P", serde_json::json!({}))
            .await
            .expect("add property");

        // lat = 200 is out of range [-90, 90]
        let result =
            add_node_helper(&store, prop.id, None, "B", serde_json::json!({"lat": 200.0})).await;

        match result {
            Err(RepositoryError::Validation(errs)) => {
                assert!(
                    errs.iter().any(|e| e.path == "lat"),
                    "expected a lat validation failure, got {:?}",
                    errs
                );
            }
            Ok(_) => panic!("expected Validation error"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: enforces cardinality max
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn enforces_cardinality_max() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        // Max is 2 for property under company.
        add_node_helper(&store, c2.clone(), Some("property"), "P1", serde_json::json!({}))
            .await
            .expect("add P1");

        add_node_helper(&store, c2.clone(), Some("property"), "P2", serde_json::json!({}))
            .await
            .expect("add P2");

        let result =
            add_node_helper(&store, c2.clone(), Some("property"), "P3", serde_json::json!({})).await;

        match result {
            Err(RepositoryError::Validation(_)) => {} // expected
            Ok(_) => panic!("expected Validation on max=2 exceeded"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: creates partner under root
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn creates_partner_under_root() {
        let store = Rc::new(Store::new());
        // Seed root node.
        store.put_node(&node::make_root());

        let n = do_add_node(
            store.clone(),
            NodeId::root(),
            Some(Level::Hn1),
            None,
            "Acme Group",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("partner should be created");

        assert_eq!(n.level(), Level::Hn1, "node is Hn1");
        assert!(n.schema.is_none(), "partner should have no schema");
    }

    // -----------------------------------------------------------------------
    // Test: rejects creating root
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_root_creation() {
        let store = Rc::new(Store::new());
        store.put_node(&node::make_root());

        let result = do_add_node(
            store.clone(),
            NodeId::root(),
            Some(Level::Hn0),
            None,
            "root2",
            serde_json::json!({}),
            None,
        )
        .await;

        match result {
            Err(RepositoryError::BadRequest(_)) => {} // expected
            Ok(_) => panic!("expected BadRequest"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: creates company with schema
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn creates_company_with_schema() {
        let store = Rc::new(Store::new());
        store.put_node(&node::make_root());

        let partner = do_add_node(
            store.clone(),
            NodeId::root(),
            Some(Level::Hn1),
            None,
            "Group",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("partner");

        let company = do_add_node(
            store.clone(),
            partner.id,
            Some(Level::Hn2),
            None,
            "Acme",
            serde_json::json!({}),
            Some(sample_schema()),
        )
        .await
        .expect("company");

        assert!(company.schema.is_some(), "company should have a schema");
    }

    // -----------------------------------------------------------------------
    // Test: company without schema fails
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn company_without_schema_fails() {
        let store = Rc::new(Store::new());
        store.put_node(&node::make_root());

        let partner = do_add_node(
            store.clone(),
            NodeId::root(),
            Some(Level::Hn1),
            None,
            "Group",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("partner");

        let result = do_add_node(
            store.clone(),
            partner.id,
            Some(Level::Hn2),
            None,
            "Acme",
            serde_json::json!({}),
            None, // no schema
        )
        .await;

        match result {
            Err(RepositoryError::BadRequest(_)) => {} // expected
            Ok(_) => panic!("expected BadRequest"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: infers level = parent+1 by default
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn infers_level_default_parent_plus_one() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        // property has exactly one child type (building) → label may be omitted.
        let prop = add_node_helper(&store, c2, Some("property"), "P", serde_json::json!({}))
            .await
            .expect("add property");

        let n = add_node_helper(&store, prop.id, None, "B", serde_json::json!({"lat": 1.0}))
            .await
            .expect("should succeed with inferred type");

        assert_eq!(n.level(), Level::Hn4, "resolved to Hn4 (parent + 1)");
        assert_eq!(n.label, "building");
    }

    // -----------------------------------------------------------------------
    // Test: infers level from label
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn infers_level_from_label() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let n = add_node_helper(&store, c2, Some("property"), "P", serde_json::json!({}))
            .await
            .expect("should succeed");

        assert_eq!(n.level(), Level::Hn3, "resolved to Hn3 via label");
        assert_eq!(n.label, "property");
    }

    // -----------------------------------------------------------------------
    // THE driving scenario: building under company (hn3) AND under group (hn4),
    // children validating as "area" at both depths.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn building_at_variable_depth() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        // building directly under company → hn3
        let b1 = add_node_helper(&store, c2.clone(), Some("building"), "B1",
            serde_json::json!({"lat": 55.0})).await.unwrap();
        assert_eq!(b1.level(), Level::Hn3);
        assert_eq!(b1.label, "building");

        // group under company → hn3; building under group → hn4
        let g = add_node_helper(&store, c2.clone(), Some("group"), "G",
            serde_json::json!({})).await.unwrap();
        let b2 = add_node_helper(&store, g.id.clone(), Some("building"), "B2",
            serde_json::json!({"lat": 55.0})).await.unwrap();
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
        let c2 = seed_company(&store);
        let err = add_node_helper(&store, c2.clone(), None, "X",
            serde_json::json!({})).await.unwrap_err();
        assert!(format!("{:?}", err).contains("specify label"));
    }

    /// Explicit level param must equal parent + 1.
    #[tokio::test]
    async fn level_param_must_match_derived() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        // company is hn2 → only hn3 children; requesting hn4 is rejected
        let err = add_node_level_helper(&store, c2, Some(Level::Hn4), Some("building"), "B",
            serde_json::json!({"lat": 1.0})).await.unwrap_err();
        assert!(format!("{:?}", err).contains("parent level + 1"));
    }

    /// Metadata required fields enforced per type at any depth.
    #[tokio::test]
    async fn metadata_enforced_per_type() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let err = add_node_helper(&store, c2, Some("building"), "B",
            serde_json::json!({})).await.unwrap_err(); // missing lat
        assert!(matches!(err, RepositoryError::Validation(_)));
    }

    // -----------------------------------------------------------------------
    // Test: get_node missing → NotFound
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_node_missing_is_not_found() {
        let store = Rc::new(Store::new());
        let ghost = NodeId::make(Level::Hn3, 99999);

        let result = get_node(ghost, get_node_fn(store.clone())).await;

        match result {
            Err(RepositoryError::NotFound(_)) => {}
            Ok(_) => panic!("expected NotFound"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: list_children / list_child_refs
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_children_works() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        add_node_helper(&store, c2.clone(), Some("property"), "P", serde_json::json!({}))
            .await
            .expect("add child");

        let kids = list_children(
            c2.clone(),
            None,
            |nid, kind| {
                let s = store.clone();
                std::future::ready(Ok(s.list_children(&nid, kind.as_ref())))
            },
        )
        .await
        .expect("list children");

        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].level(), Level::Hn3);
    }

    #[tokio::test]
    async fn list_child_refs_works() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        add_node_helper(&store, c2.clone(), Some("property"), "P", serde_json::json!({}))
            .await
            .expect("add child");

        let refs = list_child_refs(
            c2.clone(),
            None,
            |nid, kind| {
                let s = store.clone();
                std::future::ready(Ok(s.list_child_refs(&nid, kind.as_ref())))
            },
        )
        .await
        .expect("list child refs");

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].1, "P"); // name
    }

    // -----------------------------------------------------------------------
    // Test: add_node → get_node roundtrip
    //
    // We test a representative set of names.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn add_node_get_node_roundtrip() {
        for name in ["A", "Hello World", "café", "123!@#", "x".repeat(200).as_str()] {
            let store = Rc::new(Store::new());
            let c2 = seed_company(&store);

            let n = add_node_helper(&store, c2, Some("property"), name, serde_json::json!({}))
                .await
                .unwrap_or_else(|e| panic!("add_node errored on name={:?}: {:?}", name, e));

            let fetched = get_node(n.id.clone(), get_node_fn(store.clone()))
                .await
                .unwrap_or_else(|e| panic!("get_node miss after put for name={:?}: {:?}", name, e));

            assert_eq!(fetched.name, name, "name roundtrip differed for {:?}", name);
        }
    }

    // -----------------------------------------------------------------------
    // update_node_metadata
    // -----------------------------------------------------------------------

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
        let b = add_node_helper(&store, c2, Some("building"), "B", serde_json::json!({"lat": "12.5"}))
            .await
            .expect("string lat should coerce + validate");
        assert_eq!(b.metadata["lat"], serde_json::json!(12.5));
    }
}
