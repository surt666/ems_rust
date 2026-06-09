//! Access-control logic — ported 1:1 from `services/hierarchy/lib/logic/access.ml`.
//!
//! All repository operations are injected as async closures; no traits are used.
//! Pure helpers (e.g. `ancestors_of_path`) are plain sync functions.

use std::future::Future;

use crate::domain::ids::{NodeId, UserId};
use crate::domain::node::Node;
use crate::domain::user::User;
use crate::domain::values::{CognitoGroup, EdgeKind};
use crate::errors::RepositoryError;
use crate::repository::EdgeSpec;

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Return all ancestor node-ids (root-first, self excluded) by parsing the
/// pipe-separated `path` stored in the node.
///
/// Mirrors OCaml `ancestors_of`:
/// - Returns `[]` for root.
/// - Otherwise splits `node.path` on `'|'`, drops empty segments and the node's
///   own id, then parses each remaining segment.
pub fn ancestors_of_path(node_id: &NodeId, path: &str) -> Vec<NodeId> {
    if node_id.is_root() {
        return vec![];
    }
    let own = node_id.to_string();
    path.split('|')
        .filter(|s| !s.is_empty() && *s != own)
        .filter_map(|s| NodeId::parse(s).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// block / unblock
// ---------------------------------------------------------------------------

/// Create a `Blocked` edge from `user_id` → `node_id`.
///
/// Fails with `NotFoundUser` if the user does not exist, or `NotFound` if the
/// node does not exist.
///
/// Mirrors OCaml `Access.block`.
pub async fn block<FGU, FGUFut, FGN, FGNFut, FPE, FPEFut>(
    user_id: UserId,
    node_id: NodeId,
    get_user: FGU,
    get_node: FGN,
    put_edge: FPE,
) -> Result<(), RepositoryError>
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FPE: FnOnce(EdgeSpec) -> FPEFut,
    FPEFut: Future<Output = Result<(), RepositoryError>>,
{
    let uid_clone = user_id.clone();
    let nid_clone = node_id.clone();

    get_user(user_id.clone())
        .await?
        .ok_or(RepositoryError::NotFoundUser(uid_clone))?;

    get_node(node_id.clone())
        .await?
        .ok_or(RepositoryError::NotFound(nid_clone))?;

    put_edge(EdgeSpec {
        from_: user_id.to_string(),
        to_: node_id.to_string(),
        kind: EdgeKind::Blocked,
        name: String::new(),
    })
    .await?;

    Ok(())
}

/// Remove the `Blocked` edge from `user_id` → `node_id`.
///
/// No-op if the edge is absent (matches OCaml semantics).
///
/// Mirrors OCaml `Access.unblock`.
pub async fn unblock<FDE, FDEFut>(
    user_id: UserId,
    node_id: NodeId,
    delete_edge: FDE,
) -> Result<(), RepositoryError>
where
    FDE: FnOnce(String, String, EdgeKind) -> FDEFut,
    FDEFut: Future<Output = Result<(), RepositoryError>>,
{
    delete_edge(user_id.to_string(), node_id.to_string(), EdgeKind::Blocked).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// effective_permission
// ---------------------------------------------------------------------------

/// Return `Some(cognito_group)` if the user has access to `node_id` (and none
/// of `node_id`'s ancestors are blocked), or `None` if any node in the chain is
/// blocked.  Returns `Err(NotFoundUser)` if the user does not exist.
///
/// Mirrors OCaml `Access.effective_permission`.
pub async fn effective_permission<FGU, FGUFut, FGN, FGNFut, FLB, FLBFut>(
    user_id: UserId,
    node_id: NodeId,
    get_user: FGU,
    get_node: FGN,
    list_blocked_nodes: FLB,
) -> Result<Option<CognitoGroup>, RepositoryError>
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FLB: FnOnce(UserId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
{
    let uid_clone = user_id.clone();

    let user = get_user(user_id.clone())
        .await?
        .ok_or(RepositoryError::NotFoundUser(uid_clone))?;

    let blocked = list_blocked_nodes(user_id).await?;

    // Build the chain: [node_id] ++ ancestors_of_path(node_id, path)
    // We need the node's path to compute ancestors; if the node is absent we
    // fall back to an empty ancestor list (matches OCaml `ancestors_of` returning []).
    let ancestors = match get_node(node_id.clone()).await? {
        Some(n) => ancestors_of_path(&node_id, &n.path),
        None => vec![],
    };

    let mut chain = vec![node_id];
    chain.extend(ancestors);

    let is_blocked = chain.iter().any(|n| blocked.contains(n));

    if is_blocked {
        Ok(None)
    } else {
        Ok(Some(user.cognito_group))
    }
}

// ---------------------------------------------------------------------------
// list_blocked_nodes / list_blocked_users
// ---------------------------------------------------------------------------

/// List all node-ids that `user_id` has blocked.
///
/// Mirrors OCaml `Access.list_blocked_nodes`.
pub async fn list_blocked_nodes<FLB, FLBFut>(
    user_id: UserId,
    list_blocked: FLB,
) -> Result<Vec<NodeId>, RepositoryError>
where
    FLB: FnOnce(UserId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
{
    list_blocked(user_id).await
}

/// List all user-ids that have blocked `node_id`.
///
/// Mirrors OCaml `Access.list_blocked_users`.
pub async fn list_blocked_users<FLB, FLBFut>(
    node_id: NodeId,
    list_blocked: FLB,
) -> Result<Vec<UserId>, RepositoryError>
where
    FLB: FnOnce(NodeId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<UserId>, RepositoryError>>,
{
    list_blocked(node_id).await
}

// ---------------------------------------------------------------------------
// grant_administrates
// ---------------------------------------------------------------------------

/// Create an `Administrates` edge from `user_id` → `node_id`.
///
/// Fails with `NotFoundUser` if the user does not exist.  For non-root nodes,
/// also fails with `NotFound` if the node does not exist.  Root is always valid.
///
/// Mirrors OCaml `Access.grant_administrates`.
pub async fn grant_administrates<FGU, FGUFut, FGN, FGNFut, FPE, FPEFut>(
    user_id: UserId,
    node_id: NodeId,
    get_user: FGU,
    get_node: FGN,
    put_edge: FPE,
) -> Result<(), RepositoryError>
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FPE: FnOnce(EdgeSpec) -> FPEFut,
    FPEFut: Future<Output = Result<(), RepositoryError>>,
{
    let uid_clone = user_id.clone();
    let nid_clone = node_id.clone();

    get_user(user_id.clone())
        .await?
        .ok_or(RepositoryError::NotFoundUser(uid_clone))?;

    if !node_id.is_root() {
        get_node(node_id.clone())
            .await?
            .ok_or(RepositoryError::NotFound(nid_clone))?;
    }

    put_edge(EdgeSpec {
        from_: user_id.to_string(),
        to_: node_id.to_string(),
        kind: EdgeKind::Administrates,
        name: String::new(),
    })
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// list_administrated_nodes
// ---------------------------------------------------------------------------

/// List all node-ids that `user_id` administrates.
///
/// Mirrors OCaml `Access.list_administrated_nodes`.
pub async fn list_administrated_nodes<FLA, FLAFut>(
    user_id: UserId,
    list_administrated: FLA,
) -> Result<Vec<NodeId>, RepositoryError>
where
    FLA: FnOnce(UserId) -> FLAFut,
    FLAFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
{
    list_administrated(user_id).await
}

// ---------------------------------------------------------------------------
// has_admin_access
// ---------------------------------------------------------------------------

/// Return `true` if `user_id` has an `Administrates` edge to `node_id` or any
/// ancestor of `node_id` (including root).
///
/// Mirrors OCaml `Access.has_admin_access`.
pub async fn has_admin_access<FLA, FLAFut, FGN, FGNFut>(
    user_id: UserId,
    node_id: NodeId,
    list_administrated: FLA,
    get_node: FGN,
) -> Result<bool, RepositoryError>
where
    FLA: FnOnce(UserId) -> FLAFut,
    FLAFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
{
    let grants = list_administrated(user_id).await?;

    // Direct grant on the target node?
    if grants.contains(&node_id) {
        return Ok(true);
    }

    // Grant on any ancestor?
    let ancestors = match get_node(node_id.clone()).await? {
        Some(n) => ancestors_of_path(&node_id, &n.path),
        None => vec![],
    };

    Ok(ancestors.iter().any(|a| grants.contains(a)))
}

// ---------------------------------------------------------------------------
// start_nodes  (OCaml `start_refs` / start-nodes for a user's admin scope)
// ---------------------------------------------------------------------------

/// Return the set of "start nodes" for a user's administration scope.
///
/// Rules (mirrors the OCaml `top_level_shows_administrated_hn2` behaviour):
/// - If the user has a grant on **root**, expand to root's direct children
///   (the partners / HN1 nodes) — the user effectively administrates everything,
///   so we surface the top-level nodes.
/// - Otherwise return the administrated nodes as-is.
///
/// Mirrors OCaml `start_refs`.
pub async fn start_nodes<FLA, FLAFut, FLC, FLCFut>(
    user_id: UserId,
    list_administrated: FLA,
    list_child_refs: FLC,
) -> Result<Vec<NodeId>, RepositoryError>
where
    FLA: FnOnce(UserId) -> FLAFut,
    FLAFut: Future<Output = Result<Vec<NodeId>, RepositoryError>>,
    FLC: FnOnce(NodeId) -> FLCFut,
    FLCFut: Future<Output = Result<Vec<(NodeId, String)>, RepositoryError>>,
{
    let grants = list_administrated(user_id).await?;

    if grants.iter().any(|n| n.is_root()) {
        // Root grant → expand to root's direct children.
        let children = list_child_refs(NodeId::root()).await?;
        Ok(children.into_iter().map(|(id, _name)| id).collect())
    } else {
        Ok(grants)
    }
}

// ---------------------------------------------------------------------------
// Tests — port of `services/hierarchy/test/test_logic_access.ml`
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use chrono::{DateTime, Utc};

    use super::*;
    use crate::domain::ids::{Level, NodeId, UserId};
    use crate::domain::node::{self, Node};
    use crate::domain::user::User;
    use crate::domain::values::{CognitoGroup, EdgeKind};
    use crate::repository::memory::Store;
    use crate::repository::EdgeSpec;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn ts() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    /// Build an HN2 node with a correct two-segment path:
    /// `HN0#root|HN1#10001|HN2#<id>`.
    fn make_hn2(id: u32, name: &str) -> Node {
        let parent_path = format!("{}|HN1#10001", NodeId::root());
        node::make(
            id,
            Level::Hn2,
            name,
            NodeId::root(),
            &parent_path,
            ts(),
            serde_json::json!({}),
            None,
        )
    }

    /// Build an HN3 node whose parent is `parent_id` at path `parent_path`.
    fn make_hn3(id: u32, name: &str, parent_id: NodeId, parent_path: &str) -> Node {
        node::make(
            id,
            Level::Hn3,
            name,
            parent_id,
            parent_path,
            ts(),
            serde_json::json!({}),
            None,
        )
    }

    fn make_writer_user(email: &str) -> User {
        User::builder()
            .email(email.to_owned())
            .name("Alice".to_owned())
            .cognito_group(CognitoGroup::Writer)
            .created(ts())
            .build()
    }

    // -----------------------------------------------------------------------
    // Closure factories over a shared `Rc<Store>`
    // -----------------------------------------------------------------------

    fn get_user_fn(s: Rc<Store>) -> impl FnOnce(UserId) -> std::future::Ready<Result<Option<User>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.get_user(&uid)))
    }

    fn get_node_fn(s: Rc<Store>) -> impl FnOnce(NodeId) -> std::future::Ready<Result<Option<Node>, RepositoryError>> {
        move |nid| std::future::ready(Ok(s.get_node(&nid)))
    }

    fn put_edge_fn(s: Rc<Store>) -> impl FnOnce(EdgeSpec) -> std::future::Ready<Result<(), RepositoryError>> {
        move |spec| {
            s.put_edge(spec);
            std::future::ready(Ok(()))
        }
    }

    fn delete_edge_fn(s: Rc<Store>) -> impl FnOnce(String, String, EdgeKind) -> std::future::Ready<Result<(), RepositoryError>> {
        move |from_, to_, kind| {
            s.delete_edge(&from_, &to_, &kind);
            std::future::ready(Ok(()))
        }
    }

    fn list_blocked_nodes_fn(s: Rc<Store>) -> impl FnOnce(UserId) -> std::future::Ready<Result<Vec<NodeId>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_blocked_nodes(&uid)))
    }

    fn list_blocked_users_fn(s: Rc<Store>) -> impl FnOnce(NodeId) -> std::future::Ready<Result<Vec<UserId>, RepositoryError>> {
        move |nid| std::future::ready(Ok(s.list_blocked_users(&nid)))
    }

    fn list_administrated_fn(s: Rc<Store>) -> impl FnOnce(UserId) -> std::future::Ready<Result<Vec<NodeId>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_administrated_nodes(&uid)))
    }

    fn list_child_refs_fn(s: Rc<Store>) -> impl FnOnce(NodeId) -> std::future::Ready<Result<Vec<(NodeId, String)>, RepositoryError>> {
        move |nid| std::future::ready(Ok(s.list_child_refs(&nid, None)))
    }

    // -----------------------------------------------------------------------
    // Seed helper — mirrors OCaml `seed ()`
    //
    // Creates:
    //   - c2 = HN2#10002  "Acme"  (parent: root, path includes HN1#10001 as intermediate)
    //   - bldg = HN3 child of c2  named "B"
    //   - user alice@ex with CognitoGroup::Writer
    //
    // Returns (store, c2_id, bldg_id, user_id)
    // -----------------------------------------------------------------------

    fn seed() -> (Rc<Store>, NodeId, NodeId, UserId) {
        let store = Rc::new(Store::new());

        // c2 node
        let c2 = make_hn2(10002, "Acme");
        store.put_node(&c2);
        let c2_id = c2.id.clone();

        // bldg node — child of c2
        let bldg_id_raw = 10003u32;
        let bldg = make_hn3(bldg_id_raw, "B", c2.id.clone(), &c2.path);
        store.put_node(&bldg);

        // parent→child edge (mirrors `Hierarchy.add_node`)
        store.put_edge(EdgeSpec {
            from_: c2.id.to_string(),
            to_: bldg.id.to_string(),
            kind: EdgeKind::HasLabel("building".to_owned()),
            name: "B".to_owned(),
        });

        let bldg_id = bldg.id.clone();

        // user
        let user = make_writer_user("alice@ex");
        store.put_user(&user);
        let uid = user.id.clone();

        (store, c2_id, bldg_id, uid)
    }

    // -----------------------------------------------------------------------
    // Test: block then list  (OCaml: `block_then_list`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn block_then_list() {
        let (store, c2, _bldg, uid) = seed();

        block(
            uid.clone(),
            c2.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            put_edge_fn(store.clone()),
        )
        .await
        .expect("block should succeed");

        let blocked = list_blocked_nodes(uid.clone(), list_blocked_nodes_fn(store.clone()))
            .await
            .expect("list blocked nodes");

        assert_eq!(blocked.len(), 1, "expected one blocked node");
        assert_eq!(blocked[0], c2, "blocked node should be c2");
    }

    // -----------------------------------------------------------------------
    // Test: block unknown user fails  (OCaml: `block_unknown_user_fails`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn block_unknown_user_fails() {
        let store = Rc::new(Store::new());
        let c2 = make_hn2(10003, "Acme");
        store.put_node(&c2);

        let ghost_uid = UserId::of_email("ghost@ex");

        let result = block(
            ghost_uid.clone(),
            c2.id.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            put_edge_fn(store.clone()),
        )
        .await;

        match result {
            Err(RepositoryError::NotFoundUser(_)) => {} // expected
            Ok(_) => panic!("expected NotFoundUser error"),
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: effective_permission + inheritance  (OCaml: `effective_permission_flows`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn effective_permission_flows() {
        let (store, c2, bldg, uid) = seed();

        // Baseline: writer capability at bldg.
        let perm = effective_permission(
            uid.clone(),
            bldg.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            list_blocked_nodes_fn(store.clone()),
        )
        .await
        .expect("effective_permission baseline");

        match perm {
            Some(g) => assert_eq!(g.to_string(), "Writer", "expected Writer group"),
            None => panic!("expected Some(Writer), got None"),
        }

        // Block on ancestor c2.
        block(
            uid.clone(),
            c2.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            put_edge_fn(store.clone()),
        )
        .await
        .expect("block c2");

        // Now blocked at c2 itself.
        let perm_c2 = effective_permission(
            uid.clone(),
            c2.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            list_blocked_nodes_fn(store.clone()),
        )
        .await
        .expect("effective_permission c2 blocked");

        assert!(perm_c2.is_none(), "expected blocked at c2, got Some");

        // Inherited block at bldg (child of c2).
        let perm_bldg_blocked = effective_permission(
            uid.clone(),
            bldg.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            list_blocked_nodes_fn(store.clone()),
        )
        .await
        .expect("effective_permission bldg blocked");

        assert!(perm_bldg_blocked.is_none(), "expected inherited block at bldg");

        // Unblock c2, restores access.
        unblock(
            uid.clone(),
            c2.clone(),
            delete_edge_fn(store.clone()),
        )
        .await
        .expect("unblock c2");

        let perm_restored = effective_permission(
            uid.clone(),
            bldg.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            list_blocked_nodes_fn(store.clone()),
        )
        .await
        .expect("effective_permission restored");

        assert!(perm_restored.is_some(), "expected restored access after unblock");
    }

    // -----------------------------------------------------------------------
    // Test: list_blocked_users reverse  (OCaml: `list_blocked_users_reverse`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_blocked_users_reverse() {
        let (store, c2, _bldg, uid) = seed();

        block(
            uid.clone(),
            c2.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            put_edge_fn(store.clone()),
        )
        .await
        .expect("block");

        let users = list_blocked_users(c2.clone(), list_blocked_users_fn(store.clone()))
            .await
            .expect("list_blocked_users");

        assert_eq!(users.len(), 1, "expected one blocked user");
        assert_eq!(users[0], uid, "blocked user should be uid");
    }

    // -----------------------------------------------------------------------
    // Test: delete_node cascades blocks  (OCaml: `delete_node_cascades_blocks`)
    //
    // When a node is deleted from the store, all edges touching it are removed
    // (that is Store::delete_node behaviour, ported from memory.ml).
    // After deletion, list_blocked_nodes for the user should be empty.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_node_cascades_blocks() {
        let (store, c2, _bldg, uid) = seed();

        block(
            uid.clone(),
            c2.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            put_edge_fn(store.clone()),
        )
        .await
        .expect("block");

        // Simulate Hierarchy.delete_node — the memory store removes all edges.
        store.delete_node(&c2);

        let blocked = list_blocked_nodes(uid.clone(), list_blocked_nodes_fn(store.clone()))
            .await
            .expect("list blocked after node delete");

        assert_eq!(blocked.len(), 0, "expected zero blocked nodes after node delete");
    }

    // -----------------------------------------------------------------------
    // Test: grant_administrates then has_admin_access
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn grant_then_has_admin_access_direct() {
        let (store, c2, _bldg, uid) = seed();

        grant_administrates(
            uid.clone(),
            c2.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            put_edge_fn(store.clone()),
        )
        .await
        .expect("grant");

        let result = has_admin_access(
            uid.clone(),
            c2.clone(),
            list_administrated_fn(store.clone()),
            get_node_fn(store.clone()),
        )
        .await
        .expect("has_admin_access");

        assert!(result, "user should have admin access to c2 after grant");
    }

    #[tokio::test]
    async fn has_admin_access_via_ancestor() {
        let (store, c2, bldg, uid) = seed();

        // Grant on c2 (ancestor of bldg).
        grant_administrates(
            uid.clone(),
            c2.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            put_edge_fn(store.clone()),
        )
        .await
        .expect("grant on ancestor");

        let result = has_admin_access(
            uid.clone(),
            bldg.clone(),
            list_administrated_fn(store.clone()),
            get_node_fn(store.clone()),
        )
        .await
        .expect("has_admin_access via ancestor");

        assert!(result, "user should have admin access to bldg via ancestor grant on c2");
    }

    #[tokio::test]
    async fn has_admin_access_no_grant() {
        let (store, _c2, bldg, uid) = seed();

        let result = has_admin_access(
            uid.clone(),
            bldg.clone(),
            list_administrated_fn(store.clone()),
            get_node_fn(store.clone()),
        )
        .await
        .expect("has_admin_access no grant");

        assert!(!result, "user should not have admin access without any grant");
    }

    // -----------------------------------------------------------------------
    // Test: list_administrated_nodes (round-trip via grant)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_administrated_nodes_test() {
        let (store, c2, _bldg, uid) = seed();

        grant_administrates(
            uid.clone(),
            c2.clone(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            put_edge_fn(store.clone()),
        )
        .await
        .expect("grant");

        let nodes = list_administrated_nodes(uid.clone(), list_administrated_fn(store.clone()))
            .await
            .expect("list");

        assert_eq!(nodes.len(), 1);
        assert!(nodes.contains(&c2));
    }

    // -----------------------------------------------------------------------
    // Test: start_nodes — root grant expands to children
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn start_nodes_root_grant_expands_to_children() {
        let store = Rc::new(Store::new());

        // Seed: two HN1 children of root.
        let hn1a = node::make(
            10001, Level::Hn1, "Partner A",
            NodeId::root(), &NodeId::root().to_string(), ts(),
            serde_json::json!({}), None,
        );
        let hn1b = node::make(
            10002, Level::Hn1, "Partner B",
            NodeId::root(), &NodeId::root().to_string(), ts(),
            serde_json::json!({}), None,
        );
        store.put_node(&hn1a);
        store.put_node(&hn1b);
        store.put_edge(EdgeSpec {
            from_: NodeId::root().to_string(),
            to_: hn1a.id.to_string(),
            kind: EdgeKind::HasLabel("partner".to_owned()),
            name: "A".to_owned(),
        });
        store.put_edge(EdgeSpec {
            from_: NodeId::root().to_string(),
            to_: hn1b.id.to_string(),
            kind: EdgeKind::HasLabel("partner".to_owned()),
            name: "B".to_owned(),
        });

        // User with grant on root.
        let user = make_writer_user("sysadmin@ex");
        store.put_user(&user);
        let uid = user.id.clone();

        // Grant on root
        store.put_edge(EdgeSpec {
            from_: uid.to_string(),
            to_: NodeId::root().to_string(),
            kind: EdgeKind::Administrates,
            name: String::new(),
        });

        let nodes = start_nodes(
            uid.clone(),
            list_administrated_fn(store.clone()),
            list_child_refs_fn(store.clone()),
        )
        .await
        .expect("start_nodes root grant");

        assert_eq!(nodes.len(), 2, "root grant should expand to 2 children");
        assert!(nodes.contains(&hn1a.id));
        assert!(nodes.contains(&hn1b.id));
    }

    #[tokio::test]
    async fn start_nodes_non_root_grant_returns_as_is() {
        let (store, c2, _bldg, uid) = seed();

        // Grant on c2 (not root).
        store.put_edge(EdgeSpec {
            from_: uid.to_string(),
            to_: c2.to_string(),
            kind: EdgeKind::Administrates,
            name: String::new(),
        });

        let nodes = start_nodes(
            uid.clone(),
            list_administrated_fn(store.clone()),
            list_child_refs_fn(store.clone()),
        )
        .await
        .expect("start_nodes non-root");

        assert_eq!(nodes.len(), 1, "non-root grant returns administrated nodes as-is");
        assert!(nodes.contains(&c2));
    }

    // -----------------------------------------------------------------------
    // Test: grant_administrates on root succeeds (no node lookup for root)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn grant_administrates_root_succeeds() {
        let store = Rc::new(Store::new());
        let user = make_writer_user("admin@ex");
        store.put_user(&user);

        // Root is not in the node store — grant_administrates should still succeed.
        let result = grant_administrates(
            user.id.clone(),
            NodeId::root(),
            get_user_fn(store.clone()),
            get_node_fn(store.clone()),
            put_edge_fn(store.clone()),
        )
        .await;

        assert!(result.is_ok(), "grant on root should succeed even without root node in store");

        let nodes = store.list_administrated_nodes(&user.id);
        assert_eq!(nodes.len(), 1);
        assert!(nodes.contains(&NodeId::root()));
    }
}
