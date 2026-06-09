//! Hierarchy logic — ported 1:1 from `services/hierarchy/lib/logic/hierarchy.ml`.
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
///
/// Mirrors OCaml `Hierarchy.get_node`.
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
///
/// Mirrors OCaml `Hierarchy.list_children`.
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
///
/// Mirrors OCaml `Hierarchy.list_child_refs`.
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
/// Mirrors OCaml `Hierarchy.add_node`.
///
/// Rules:
/// - `level = Hn0` → `BadRequest` (cannot create root).
/// - Parent not found → `NotFound`.
/// - Parent is root (Hn0): child must be Hn1 (partner); schema must be `None`.
///   Default label = "partner".
/// - Parent is Hn1: child must be Hn2 (company); `schema` is **required** and
///   validated.  Default label = "company".
/// - Otherwise: call `schema_check::find_for` on the parent, then validate the
///   proposed edge via the schema's `edges_between` list.  `schema` must be
///   `None` here.
///
/// After edge selection, metadata is validated against `schema.metadata_for(level)`.
/// Cardinality (`max`) is enforced by counting existing children of that label.
/// Finally the node is allocated (`add_node`) and the edge written atomically.
#[allow(clippy::too_many_arguments)]
pub async fn add_node<FGN, FGNFut, FLC, FLCFut, FAN, FANFut>(
    parent: NodeId,
    level: Option<Level>,
    label: Option<String>,
    name: String,
    metadata: serde_json::Value,
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

    // Resolve the target level.
    let resolved_level = match level {
        Some(lv) => lv,
        None => resolve_child_level(
            &parent,
            parent_level,
            label.as_deref(),
            &get_node_fn,
        )
        .await?,
    };

    // Depth check: child must be strictly deeper than parent.
    if parent_level.depth() >= resolved_level.depth() {
        return Err(bad("child depth must exceed parent depth"));
    }

    // Determine edge label and (optional) node schema.
    let (edge_label, node_schema) = match (parent_level, resolved_level) {
        (Level::Hn0, Level::Hn1) => {
            if schema.is_some() {
                return Err(RepositoryError::BadRequest(
                    "schema only allowed on hn2 nodes".to_string(),
                ));
            }
            let lbl = label.clone().unwrap_or_else(|| "partner".to_string());
            (lbl, None)
        }
        (Level::Hn0, _) => {
            return Err(bad("root can only contain hn1 (partner) nodes"));
        }
        (Level::Hn1, Level::Hn2) => {
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
            let lbl = label.clone().unwrap_or_else(|| "company".to_string());
            (lbl, Some(sch))
        }
        (Level::Hn1, _) => {
            return Err(bad("partner can only contain hn2 (company) nodes"));
        }
        _ => {
            if schema.is_some() {
                return Err(RepositoryError::BadRequest(
                    "schema only allowed on hn2 nodes".to_string(),
                ));
            }
            let lbl = add_under_schema(
                &parent,
                parent_level,
                resolved_level,
                label.as_deref(),
                &metadata,
                &get_node_fn,
                list_children_fn,
            )
            .await?;
            (lbl, None)
        }
    };

    // For non-schema-governed levels (Hn0→Hn1, Hn1→Hn2), we still need to
    // validate metadata.  For schema-governed levels that was done inside
    // add_under_schema.  For Hn0→Hn1 and Hn1→Hn2 the schema has no metadata
    // specs yet (node_schema is None or freshly created), so we skip here.
    // (OCaml only validates metadata for schema-governed children via add_under_schema.)

    // Allocate node and write edge atomically.
    // The closure is `Fn` (not `FnOnce`) so it can be invoked on every retry
    // of the counter's ConditionalCheckFailed loop — each call clones its
    // captured state independently.
    let parent_path = parent_node.path.clone();
    let parent_clone = parent.clone();

    add_node_fn(
        resolved_level,
        Box::new(move |raw_id| {
            let child = node::make(
                raw_id,
                resolved_level,
                &name.clone(),
                parent_clone.clone(),
                &parent_path,
                chrono::Utc::now(),
                metadata.clone(),
                node_schema.clone(),
            );
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
// resolve_child_level (internal)
// ---------------------------------------------------------------------------

/// Resolve the child level when it was not specified by the caller.
///
/// Mirrors OCaml `resolve_child_level`.
async fn resolve_child_level<FGN, FGNFut>(
    parent: &NodeId,
    parent_level: Level,
    label: Option<&str>,
    get_node_fn: &FGN,
) -> Result<Level, RepositoryError>
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
{
    match parent_level {
        Level::Hn0 => Ok(Level::Hn1),
        Level::Hn1 => Ok(Level::Hn2),
        _ => {
            let (_host, schema) =
                schema_check::find_for(parent.clone(), get_node_fn).await?;
            match label {
                Some(l) => {
                    let matches: Vec<Level> = schema
                        .allowed_children(parent_level)
                        .iter()
                        .filter_map(|(child_level, specs)| {
                            if specs.iter().any(|s| s.label == l) {
                                Some(*child_level)
                            } else {
                                None
                            }
                        })
                        .collect();
                    match matches.as_slice() {
                        [c] => Ok(*c),
                        [] => Err(bad(format!(
                            "no edge from {} with label {:?}",
                            parent_level, l
                        ))),
                        _ => Err(bad(format!(
                            "label {:?} matches multiple target levels; specify level",
                            l
                        ))),
                    }
                }
                None => Level::of_depth(parent_level.depth() + 1)
                    .ok_or_else(|| bad(format!("no default child level for {}", parent_level))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// add_under_schema (internal)
// ---------------------------------------------------------------------------

/// Validate and select the edge spec for a schema-governed child add.
///
/// Returns the resolved edge label on success.
/// Mirrors OCaml `add_under_schema`.
///
/// `list_children_fn` is consumed here (FnOnce) when a cardinality max needs
/// to be enforced; otherwise it is dropped unused.
async fn add_under_schema<FGN, FGNFut, FLC, FLCFut>(
    parent: &NodeId,
    parent_level: Level,
    level: Level,
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

    let candidates = schema.edges_between(parent_level, level);

    // Select the edge spec.
    let edge_spec = match (label, candidates.as_slice()) {
        (Some(l), _) => {
            candidates
                .iter()
                .find(|s| s.label == l)
                .cloned()
                .ok_or_else(|| {
                    bad(format!(
                        "edge {} -> {} ({}) not allowed by schema",
                        parent_level, level, l
                    ))
                })?
        }
        (None, [single]) => single.clone(),
        (None, []) => {
            return Err(bad(format!(
                "edge {} -> {} not allowed by schema",
                parent_level, level
            )))
        }
        (None, _many) => {
            return Err(bad(format!(
                "edge {} -> {} is ambiguous; specify label",
                parent_level, level
            )))
        }
    };

    // Validate metadata against the schema's specs for this level.
    let specs = schema.metadata_for(level);
    validate(specs, metadata)
        .map_err(RepositoryError::Validation)?;

    // Enforce cardinality max.
    if let Some(max) = edge_spec.max {
        let kind = EdgeKind::HasLabel(edge_spec.label.clone());
        // list_children_fn is FnOnce — consume it here for the cardinality check.
        let existing = list_children_fn(parent.clone(), Some(kind)).await
            .unwrap_or_default();
        if existing.len() >= max as usize {
            return Err(bad(format!(
                "max {} {} per parent already reached",
                max, edge_spec.label
            )));
        }
    }

    Ok(edge_spec.label)
}

// ---------------------------------------------------------------------------
// Tests — port of `services/hierarchy/test/test_logic_hierarchy.ml`
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use chrono::{DateTime, Utc};

    use super::*;
    use crate::domain::ids::{Level, NodeId};
    use crate::domain::node;
    use crate::domain::schema::{EdgeSpec as SchemaEdgeSpec, FieldSpec, Schema};
    use crate::domain::values::{EdgeKind, FieldType};
    use crate::errors::RepositoryError;
    use crate::repository::memory::Store;

    fn ts() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    // Sample schema mirroring OCaml `sample_schema` in test_logic_hierarchy.ml:
    //   Hn2→Hn3: label="property", max=2
    //   Hn3→Hn4: label="building", min=1
    //   Hn4: metadata lat (Number, -90..90, required)
    fn sample_schema() -> Schema {
        Schema {
            version: 1,
            edges: vec![
                (
                    Level::Hn2,
                    vec![(
                        Level::Hn3,
                        vec![SchemaEdgeSpec::builder()
                            .label("property".to_string())
                            .max(Some(2))
                            .build()],
                    )],
                ),
                (
                    Level::Hn3,
                    vec![(
                        Level::Hn4,
                        vec![SchemaEdgeSpec::builder()
                            .label("building".to_string())
                            .min(Some(1))
                            .build()],
                    )],
                ),
            ],
            metadata: vec![(
                Level::Hn4,
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
            sensors: vec![],
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
        let n2 = node::make(
            10002,
            Level::Hn2,
            "Acme",
            NodeId::root(),
            &parent_path_for_hn2(),
            ts(),
            serde_json::json!({}),
            Some(sample_schema()),
        );
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

    // -----------------------------------------------------------------------
    // Test: add property + building  (OCaml: `add_property_and_building`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn add_property_and_building() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let prop = do_add_node(
            store.clone(),
            c2,
            Some(Level::Hn3),
            None,
            "Ostergade",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("add property should succeed");

        let b1 = do_add_node(
            store.clone(),
            prop.id.clone(),
            Some(Level::Hn4),
            None,
            "B1",
            serde_json::json!({"lat": 55.0}),
            None,
        )
        .await
        .expect("add building should succeed");

        assert_eq!(
            b1.parent.as_ref().unwrap().to_string(),
            prop.id.to_string(),
            "parent wired correctly"
        );
    }

    // -----------------------------------------------------------------------
    // Test: rejects disallowed edge  (OCaml: `rejects_disallowed_edge`)
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
    // Test: rejects bad metadata  (OCaml: `rejects_bad_metadata`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_bad_metadata() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let prop = do_add_node(
            store.clone(),
            c2,
            Some(Level::Hn3),
            None,
            "P",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("add property");

        // lat = 200 is out of range [-90, 90]
        let result = do_add_node(
            store.clone(),
            prop.id,
            Some(Level::Hn4),
            None,
            "B",
            serde_json::json!({"lat": 200.0}),
            None,
        )
        .await;

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
    // Test: enforces cardinality max  (OCaml: `enforces_cardinality_max`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn enforces_cardinality_max() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        // Max is 2 for property (Hn3) under company (Hn2).
        do_add_node(
            store.clone(),
            c2.clone(),
            Some(Level::Hn3),
            None,
            "P1",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("add P1");

        do_add_node(
            store.clone(),
            c2.clone(),
            Some(Level::Hn3),
            None,
            "P2",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("add P2");

        let result = do_add_node(
            store.clone(),
            c2.clone(),
            Some(Level::Hn3),
            None,
            "P3",
            serde_json::json!({}),
            None,
        )
        .await;

        match result {
            Err(RepositoryError::Validation(_)) => {} // expected
            Ok(_) => panic!("expected Validation on max=2 exceeded"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: creates partner under root  (OCaml: `creates_partner_under_root`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn creates_partner_under_root() {
        let store = Rc::new(Store::new());
        // Seed root node.
        store.put_node(&node::make_root(ts()));

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
    // Test: rejects creating root  (OCaml: `rejects_root_creation`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_root_creation() {
        let store = Rc::new(Store::new());
        store.put_node(&node::make_root(ts()));

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
    // Test: creates company with schema  (OCaml: `creates_company_with_schema`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn creates_company_with_schema() {
        let store = Rc::new(Store::new());
        store.put_node(&node::make_root(ts()));

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
    // Test: company without schema fails  (OCaml: `company_without_schema_fails`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn company_without_schema_fails() {
        let store = Rc::new(Store::new());
        store.put_node(&node::make_root(ts()));

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
    // Test: infers level = parent+1 by default  (OCaml: `infers_level_default_parent_plus_one`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn infers_level_default_parent_plus_one() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let n = do_add_node(
            store.clone(),
            c2,
            None, // no explicit level
            None,
            "P",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("should succeed with inferred level");

        assert_eq!(n.level(), Level::Hn3, "resolved to Hn3");
    }

    // -----------------------------------------------------------------------
    // Test: infers level from label  (OCaml: `infers_level_from_label`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn infers_level_from_label() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        let n = do_add_node(
            store.clone(),
            c2,
            None, // no explicit level
            Some("property".to_string()),
            "P",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("should succeed");

        assert_eq!(n.level(), Level::Hn3, "resolved to Hn3 via label");
    }

    // -----------------------------------------------------------------------
    // Test: infers level from cross-level label  (OCaml: `infers_level_from_cross_level_label`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn infers_level_from_cross_level_label() {
        // Schema with two edge types from Hn2: floor→Hn3, zone→Hn4.
        let cross_schema = Schema {
            version: 1,
            edges: vec![(
                Level::Hn2,
                vec![
                    (
                        Level::Hn3,
                        vec![SchemaEdgeSpec::builder().label("floor".to_string()).build()],
                    ),
                    (
                        Level::Hn4,
                        vec![SchemaEdgeSpec::builder().label("zone".to_string()).build()],
                    ),
                ],
            )],
            metadata: vec![],
            sensors: vec![],
        };

        let store = Rc::new(Store::new());
        let c2 = NodeId::make(Level::Hn2, 10003);
        let n2 = node::make(
            10003,
            Level::Hn2,
            "X",
            NodeId::root(),
            &parent_path_for_hn2(),
            ts(),
            serde_json::json!({}),
            Some(cross_schema),
        );
        store.put_node(&n2);

        let n = do_add_node(
            store.clone(),
            c2,
            None, // no explicit level
            Some("zone".to_string()),
            "Z",
            serde_json::json!({}),
            None,
        )
        .await
        .expect("should succeed");

        assert_eq!(n.level(), Level::Hn4, "label=zone resolved to Hn4");
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

        do_add_node(
            store.clone(),
            c2.clone(),
            Some(Level::Hn3),
            None,
            "P",
            serde_json::json!({}),
            None,
        )
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

        do_add_node(
            store.clone(),
            c2.clone(),
            Some(Level::Hn3),
            None,
            "P",
            serde_json::json!({}),
            None,
        )
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
    // Test: add_node → get_node roundtrip  (port of test_logic_properties.ml)
    //
    // OCaml uses a quickcheck generator over non-empty printable strings.
    // Here we test a representative set of names instead.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn add_node_get_node_roundtrip() {
        for name in ["A", "Hello World", "café", "123!@#", "x".repeat(200).as_str()] {
            let store = Rc::new(Store::new());
            let c2 = seed_company(&store);

            let n = do_add_node(
                store.clone(),
                c2,
                Some(Level::Hn3),
                None,
                name,
                serde_json::json!({}),
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("add_node errored on name={:?}: {:?}", name, e));

            let fetched = get_node(n.id.clone(), get_node_fn(store.clone()))
                .await
                .unwrap_or_else(|e| panic!("get_node miss after put for name={:?}: {:?}", name, e));

            assert_eq!(fetched.name, name, "name roundtrip differed for {:?}", name);
        }
    }
}
