//! Schema-check logic — ported 1:1 from `services/hierarchy/lib/logic/schema_check.ml`.
//!
//! The single entry point `find_for` locates the HN2 (company) ancestor of a
//! node and returns its `(NodeId, Schema)`.  All repository access is injected
//! as an async closure.

use std::future::Future;

use crate::domain::ids::{Level, NodeId};
use crate::domain::node::{segment_at_level, Node};
use crate::domain::schema::Schema;
use crate::errors::RepositoryError;

// ---------------------------------------------------------------------------
// find_for
// ---------------------------------------------------------------------------

/// Locate the HN2 ancestor of `id` and return `(hn2_id, schema)`.
///
/// Algorithm (mirrors OCaml `Schema_check.find_for`):
/// 1. If `id` is root → `Err(SchemaMissing(id))`.
/// 2. Fetch the node; absent → `Err(NotFound(id))`.
/// 3. If the node *is* an HN2 → return its schema (or `SchemaMissing`).
/// 4. Otherwise extract the HN2 segment from the node's `path`, parse it,
///    fetch that HN2 node, and return its schema.
pub async fn find_for<FGN, FGNFut>(
    id: NodeId,
    get_node: FGN,
) -> Result<(NodeId, Schema), RepositoryError>
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
{
    if id.is_root() {
        return Err(RepositoryError::SchemaMissing(id));
    }

    let node = get_node(id.clone())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(id.clone()))?;

    if node.level() == Level::Hn2 {
        return match node.schema {
            Some(s) => Ok((node.id, s)),
            None => Err(RepositoryError::SchemaMissing(id)),
        };
    }

    // Walk the path to find the HN2 segment.
    match segment_at_level(&node.path, Level::Hn2) {
        None => Err(RepositoryError::SchemaMissing(id)),
        Some(seg) => {
            let hn2_id = NodeId::parse(&seg)
                .map_err(|_| RepositoryError::SchemaMissing(id.clone()))?;
            let hn2 = get_node(hn2_id.clone())
                .await?
                .ok_or_else(|| RepositoryError::NotFound(hn2_id.clone()))?;
            match hn2.schema {
                Some(s) => Ok((hn2.id, s)),
                None => Err(RepositoryError::SchemaMissing(hn2_id)),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — port of `services/hierarchy/test/test_logic_schema_check.ml`
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use chrono::{DateTime, Utc};

    use super::*;
    use crate::domain::ids::{Level, NodeId};
    use crate::domain::node;
    use crate::domain::schema::{EdgeSpec, Schema};
    use crate::errors::RepositoryError;
    use crate::repository::memory::Store;

    fn ts() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn sample_schema() -> Schema {
        Schema {
            version: 1,
            edges: vec![
                (
                    Level::Hn2,
                    vec![(
                        Level::Hn3,
                        vec![EdgeSpec::builder()
                            .label("property".to_string())
                            .build()],
                    )],
                ),
                (
                    Level::Hn3,
                    vec![(
                        Level::Hn4,
                        vec![EdgeSpec::builder()
                            .label("building".to_string())
                            .build()],
                    )],
                ),
            ],
            metadata: vec![],
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

    fn get_node_fn(
        s: Rc<Store>,
    ) -> impl Fn(NodeId) -> std::future::Ready<Result<Option<node::Node>, RepositoryError>>
    {
        move |nid| std::future::ready(Ok(s.get_node(&nid)))
    }

    // -----------------------------------------------------------------------
    // Test: find_schema_from_self  (OCaml: `find_schema_from_self`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn find_schema_from_self() {
        let store = Rc::new(Store::new());
        let c2 = NodeId::make(Level::Hn2, 10010);
        let n = node::make(
            10010,
            Level::Hn2,
            "Acme",
            NodeId::root(),
            &parent_path_for_hn2(),
            ts(),
            serde_json::json!({}),
            Some(sample_schema()),
        );
        store.put_node(&n);

        let result = find_for(c2.clone(), get_node_fn(store.clone()))
            .await
            .expect("find_for should succeed");

        assert_eq!(result.0, c2, "host should be the HN2 node itself");
        assert_eq!(result.1.version, 1, "schema version should be 1");
    }

    // -----------------------------------------------------------------------
    // Test: find_schema_by_walking_up  (OCaml: `find_schema_by_walking_up`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn find_schema_by_walking_up() {
        let store = Rc::new(Store::new());
        let c2 = NodeId::make(Level::Hn2, 10020);
        let n2 = node::make(
            10020,
            Level::Hn2,
            "Acme",
            NodeId::root(),
            &parent_path_for_hn2(),
            ts(),
            serde_json::json!({}),
            Some(sample_schema()),
        );
        store.put_node(&n2);

        let c3 = NodeId::make(Level::Hn3, 10021);
        let n3 = node::make(
            10021,
            Level::Hn3,
            "Ostergade",
            c2.clone(),
            &n2.path,
            ts(),
            serde_json::json!({}),
            None,
        );
        store.put_node(&n3);

        let result = find_for(c3, get_node_fn(store.clone()))
            .await
            .expect("find_for from hn3 should succeed");

        assert_eq!(result.0, c2, "host is the HN2 ancestor");
    }

    // -----------------------------------------------------------------------
    // Test: schema_missing_when_no_hn2  (OCaml: `schema_missing_when_no_hn2`)
    //
    // An HN3 node whose path has no HN2 ancestor → SchemaMissing.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn schema_missing_when_no_hn2() {
        let store = Rc::new(Store::new());
        let c3 = NodeId::make(Level::Hn3, 10030);
        let n3 = node::make(
            10030,
            Level::Hn3,
            "orphan",
            NodeId::root(),
            // path contains no HN2 segment
            &NodeId::root().to_string(),
            ts(),
            serde_json::json!({}),
            None,
        );
        store.put_node(&n3);

        let result = find_for(c3, get_node_fn(store.clone())).await;

        match result {
            Err(RepositoryError::SchemaMissing(_)) => {} // expected
            Ok(_) => panic!("expected SchemaMissing"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }
}
