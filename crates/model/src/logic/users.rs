//! Users logic — ported 1:1 from `services/hierarchy/lib/logic/users.ml`.
//!
//! All repository operations are injected as async closures; no traits are used.

use std::future::Future;

use crate::domain::ids::UserId;
use crate::domain::user::User;
use crate::domain::values::{CognitoGroup, Currency, Language};
use crate::errors::RepositoryError;

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

/// Create a new user.
///
/// Rejects with `Conflict` if a user with the same id already exists.
/// Otherwise persists via `put_user` and returns the newly created user.
///
/// Mirrors OCaml `Users.create`.
pub async fn create<FGU, FGUFut, FPU, FPUFut>(
    email: String,
    name: String,
    cognito_group: CognitoGroup,
    language: Option<Language>,
    currency: Option<Currency>,
    get_user: FGU,
    put_user: FPU,
) -> Result<User, RepositoryError>
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FPU: FnOnce(User) -> FPUFut,
    FPUFut: Future<Output = Result<(), RepositoryError>>,
{
    let id = UserId::of_email(&email);

    match get_user(id).await? {
        Some(_) => Err(RepositoryError::Conflict(format!(
            "user {} already exists",
            email
        ))),
        None => {
            let user = User::builder()
                .email(email)
                .name(name)
                .cognito_group(cognito_group)
                .language(language.unwrap_or_default())
                .currency(currency.unwrap_or_default())
                .build();
            put_user(user.clone()).await?;
            Ok(user)
        }
    }
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

/// Update an existing user's mutable fields.
///
/// Fails with `NotFoundUser` if the user does not exist.
/// Returns the updated user.
///
/// Mirrors OCaml `Users.update`.
pub async fn update<FGU, FGUFut, FPU, FPUFut>(
    id: UserId,
    name: Option<String>,
    cognito_group: Option<CognitoGroup>,
    language: Option<Language>,
    currency: Option<Currency>,
    get_user: FGU,
    put_user: FPU,
) -> Result<User, RepositoryError>
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FPU: FnOnce(User) -> FPUFut,
    FPUFut: Future<Output = Result<(), RepositoryError>>,
{
    let id_clone = id.clone();
    let u = get_user(id)
        .await?
        .ok_or(RepositoryError::NotFoundUser(id_clone))?;

    let updated = User::builder()
        .email(u.email.clone())
        .id(u.id.clone())
        .name(name.unwrap_or(u.name))
        .cognito_group(cognito_group.unwrap_or(u.cognito_group))
        .language(language.unwrap_or(u.language))
        .currency(currency.unwrap_or(u.currency))
        .created(u.created)
        .build();

    put_user(updated.clone()).await?;
    Ok(updated)
}

// ---------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------

/// Delete a user, cascading ALL access edges (Administrates, Reads, Writes)
/// AND Blocked edges, then the user node itself.
///
/// Fails with `NotFoundUser` if the user does not exist.
///
/// Mirrors OCaml `Users.delete`.
pub async fn delete<FGU, FGUFut, FLA, FLAFut, FLB, FLBFut, FDE, FDEFut, FDU, FDUFut>(
    id: UserId,
    get_user: FGU,
    list_access_edges: FLA,
    list_blocked: FLB,
    delete_edge: FDE,
    delete_user: FDU,
) -> Result<UserId, RepositoryError>
where
    FGU: FnOnce(UserId) -> FGUFut,
    FGUFut: Future<Output = Result<Option<User>, RepositoryError>>,
    FLA: FnOnce(UserId) -> FLAFut,
    FLAFut: Future<Output = Result<Vec<(crate::domain::ids::NodeId, crate::domain::values::EdgeKind)>, RepositoryError>>,
    FLB: FnOnce(UserId) -> FLBFut,
    FLBFut: Future<Output = Result<Vec<crate::domain::ids::NodeId>, RepositoryError>>,
    FDE: Fn(String, String, crate::domain::values::EdgeKind) -> FDEFut,
    FDEFut: Future<Output = Result<(), RepositoryError>>,
    FDU: FnOnce(UserId) -> FDUFut,
    FDUFut: Future<Output = Result<(), RepositoryError>>,
{
    let id_clone = id.clone();
    match get_user(id.clone()).await? {
        None => Err(RepositoryError::NotFoundUser(id_clone)),
        Some(_) => {
            let id_s = id.to_string();

            // Cascade: delete all Administrates / Reads / Writes edges.
            let access = list_access_edges(id.clone()).await?;
            for (node_id, kind) in access {
                delete_edge(id_s.clone(), node_id.to_string(), kind).await?;
            }

            // Cascade: delete all Blocked edges.
            let blocked_nodes = list_blocked(id.clone()).await?;
            for node_id in blocked_nodes {
                delete_edge(
                    id_s.clone(),
                    node_id.to_string(),
                    crate::domain::values::EdgeKind::Blocked,
                )
                .await?;
            }

            // Delete the user.
            delete_user(id.clone()).await?;
            Ok(id)
        }
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// List all users.
///
/// Mirrors OCaml `Users.list`.
pub async fn list<FLU, FLUFut>(list_users: FLU) -> Result<Vec<User>, RepositoryError>
where
    FLU: FnOnce() -> FLUFut,
    FLUFut: Future<Output = Result<Vec<User>, RepositoryError>>,
{
    list_users().await
}

// ---------------------------------------------------------------------------
// Tests — port of `services/hierarchy/test/test_logic_users.ml`
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use chrono::{DateTime, Utc};

    use super::*;
    use crate::domain::ids::{Level, NodeId, UserId};
    use crate::domain::node;
    use crate::domain::user::User;
    use crate::domain::values::{CognitoGroup, EdgeKind};
    use crate::repository::memory::Store;
    use crate::repository::EdgeSpec;

    // -----------------------------------------------------------------------
    // Helpers / closure factories
    // -----------------------------------------------------------------------

    fn ts() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn get_user_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<Option<User>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.get_user(&uid)))
    }

    fn put_user_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(User) -> std::future::Ready<Result<(), RepositoryError>> {
        move |u| {
            s.put_user(&u);
            std::future::ready(Ok(()))
        }
    }

    fn delete_user_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<(), RepositoryError>> {
        move |uid| {
            s.delete_user(&uid);
            std::future::ready(Ok(()))
        }
    }

    fn list_users_fn(
        s: Rc<Store>,
    ) -> impl FnOnce() -> std::future::Ready<Result<Vec<User>, RepositoryError>> {
        move || std::future::ready(Ok(s.list_users()))
    }

    fn list_access_edges_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<Vec<(NodeId, EdgeKind)>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_access_edges(&uid)))
    }

    fn list_blocked_nodes_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(UserId) -> std::future::Ready<Result<Vec<NodeId>, RepositoryError>> {
        move |uid| std::future::ready(Ok(s.list_blocked_nodes(&uid)))
    }

    /// delete_edge closure — must be `Fn` (called in a loop), so we clone the store.
    fn delete_edge_fn(
        s: Rc<Store>,
    ) -> impl Fn(String, String, EdgeKind) -> std::future::Ready<Result<(), RepositoryError>> {
        move |from_, to_, kind| {
            s.delete_edge(&from_, &to_, &kind);
            std::future::ready(Ok(()))
        }
    }

    /// Make an HN2 node for cascade tests (mirrors OCaml `sample_schema` / `parent_path_for_hn2`).
    fn make_hn2(id: u32, name: &str) -> crate::domain::node::Node {
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

    // -----------------------------------------------------------------------
    // Test: create happy path  (OCaml: `create_happy`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_happy() {
        let store = Rc::new(Store::new());

        let u = create(
            "a@x".to_owned(),
            "A".to_owned(),
            CognitoGroup::Reader,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await
        .expect("create should succeed");

        assert_eq!(u.name, "A");
        assert_eq!(u.id.email(), "a@x");
    }

    // -----------------------------------------------------------------------
    // Test: duplicate rejected  (OCaml: `create_duplicate_conflicts`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_duplicate_conflicts() {
        let store = Rc::new(Store::new());

        // First create succeeds.
        create(
            "a@x".to_owned(),
            "A".to_owned(),
            CognitoGroup::Reader,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await
        .expect("first create should succeed");

        // Second create with same email must fail with Conflict.
        let result = create(
            "a@x".to_owned(),
            "A again".to_owned(),
            CognitoGroup::Reader,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await;

        match result {
            Err(RepositoryError::Conflict(_)) => {} // expected
            Ok(_) => panic!("expected Conflict error"),
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: get unknown → NotFoundUser  (OCaml: `get_unknown_is_not_found`)
    //
    // The OCaml `Users.get` is a pure effect; here we inline the equivalent
    // logic: get_user returning None → NotFoundUser.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_unknown_is_not_found() {
        let store = Rc::new(Store::new());
        let ghost_id = UserId::of_email("ghost@x");

        // update with a non-existent user exercises the same get→not_found path.
        let result = update(
            ghost_id,
            None,
            None,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await;

        match result {
            Err(RepositoryError::NotFoundUser(_)) => {} // expected
            Ok(_) => panic!("expected NotFoundUser"),
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: update changes name  (OCaml: `update_changes_name`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn update_changes_name() {
        let store = Rc::new(Store::new());

        let u = create(
            "a@x".to_owned(),
            "A".to_owned(),
            CognitoGroup::Reader,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await
        .expect("create");

        let updated = update(
            u.id.clone(),
            Some("Alice".to_owned()),
            None,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await
        .expect("update");

        assert_eq!(updated.name, "Alice");
    }

    // -----------------------------------------------------------------------
    // Test: list and delete via logic  (OCaml: `list_and_delete_via_logic`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_and_delete_via_logic() {
        let store = Rc::new(Store::new());

        let u1 = create(
            "a@x".to_owned(),
            "A".to_owned(),
            CognitoGroup::Reader,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await
        .expect("create u1");

        create(
            "b@x".to_owned(),
            "B".to_owned(),
            CognitoGroup::Writer,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await
        .expect("create u2");

        let all = list(list_users_fn(store.clone()))
            .await
            .expect("list");
        assert_eq!(all.len(), 2, "expected two users");

        delete(
            u1.id.clone(),
            get_user_fn(store.clone()),
            list_access_edges_fn(store.clone()),
            list_blocked_nodes_fn(store.clone()),
            delete_edge_fn(store.clone()),
            delete_user_fn(store.clone()),
        )
        .await
        .expect("delete u1");

        let remaining = list(list_users_fn(store.clone()))
            .await
            .expect("list after delete");
        assert_eq!(remaining.len(), 1, "expected one user left");
    }

    // -----------------------------------------------------------------------
    // Test: delete unknown → NotFoundUser  (OCaml: `delete_unknown_errors`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_unknown_errors() {
        let store = Rc::new(Store::new());
        let ghost_id = UserId::of_email("ghost@x");

        let result = delete(
            ghost_id,
            get_user_fn(store.clone()),
            list_access_edges_fn(store.clone()),
            list_blocked_nodes_fn(store.clone()),
            delete_edge_fn(store.clone()),
            delete_user_fn(store.clone()),
        )
        .await;

        match result {
            Err(RepositoryError::NotFoundUser(_)) => {} // expected
            Ok(_) => panic!("expected NotFoundUser"),
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: delete cascades Blocked edges  (OCaml: `delete_cascades_blocked_edges`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_cascades_blocked_edges() {
        let store = Rc::new(Store::new());
        let node = make_hn2(10042, "Acme");
        let node_id = node.id.clone();
        store.put_node(&node);

        // Create user.
        let uid = create(
            "a@x".to_owned(),
            "A".to_owned(),
            CognitoGroup::Reader,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await
        .expect("create user")
        .id;

        // Create Blocked edge directly via access::block (using store ops).
        store.put_edge(EdgeSpec {
            from_: uid.to_string(),
            to_: node_id.to_string(),
            kind: EdgeKind::Blocked,
            name: String::new(),
        });

        // Verify one blocked node before delete.
        assert_eq!(
            store.list_blocked_nodes(&uid).len(),
            1,
            "one blocked before delete"
        );

        // Delete user — must cascade.
        delete(
            uid.clone(),
            get_user_fn(store.clone()),
            list_access_edges_fn(store.clone()),
            list_blocked_nodes_fn(store.clone()),
            delete_edge_fn(store.clone()),
            delete_user_fn(store.clone()),
        )
        .await
        .expect("delete user");

        // Edges gone, user gone.
        assert_eq!(
            store.list_blocked_nodes(&uid).len(),
            0,
            "zero blocked after delete"
        );
        assert_eq!(
            store.list_blocked_users(&node_id).len(),
            0,
            "no users block node after delete"
        );
        assert!(store.get_user(&uid).is_none(), "user should be gone");
    }

    // -----------------------------------------------------------------------
    // Test: delete cascades Administrates edges  (OCaml: `delete_cascades_administrates_edges`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_cascades_administrates_edges() {
        let store = Rc::new(Store::new());
        let node = make_hn2(10043, "Beta");
        let node_id = node.id.clone();
        store.put_node(&node);

        // Create user.
        let uid = create(
            "c@x".to_owned(),
            "C".to_owned(),
            CognitoGroup::Admin,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await
        .expect("create user")
        .id;

        // Grant Administrates edge directly.
        store.put_edge(EdgeSpec {
            from_: uid.to_string(),
            to_: node_id.to_string(),
            kind: EdgeKind::Administrates,
            name: String::new(),
        });

        // Verify one admin node before delete.
        assert_eq!(
            store.list_administrated_nodes(&uid).len(),
            1,
            "one admin before delete"
        );

        // Delete user — must cascade.
        delete(
            uid.clone(),
            get_user_fn(store.clone()),
            list_access_edges_fn(store.clone()),
            list_blocked_nodes_fn(store.clone()),
            delete_edge_fn(store.clone()),
            delete_user_fn(store.clone()),
        )
        .await
        .expect("delete user");

        // Edges gone.
        assert_eq!(
            store.list_administrated_nodes(&uid).len(),
            0,
            "zero admin after delete"
        );
        assert!(store.get_user(&uid).is_none(), "user should be gone");
    }

    // -----------------------------------------------------------------------
    // Test: delete cascades Writes and Reads edges
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_cascades_writes_and_reads_edges() {
        let store = Rc::new(Store::new());
        let node_w = make_hn2(10044, "WriteTarget");
        let node_r = make_hn2(10045, "ReadTarget");
        let node_w_id = node_w.id.clone();
        let node_r_id = node_r.id.clone();
        store.put_node(&node_w);
        store.put_node(&node_r);

        // Create a Writer user.
        let uid = create(
            "writer@x".to_owned(),
            "Writer".to_owned(),
            CognitoGroup::Writer,
            None,
            None,
            get_user_fn(store.clone()),
            put_user_fn(store.clone()),
        )
        .await
        .expect("create user")
        .id;

        // Grant Writes edge to one node, Reads edge to another.
        store.put_edge(EdgeSpec {
            from_: uid.to_string(),
            to_: node_w_id.to_string(),
            kind: EdgeKind::Writes,
            name: String::new(),
        });
        store.put_edge(EdgeSpec {
            from_: uid.to_string(),
            to_: node_r_id.to_string(),
            kind: EdgeKind::Reads,
            name: String::new(),
        });

        // Verify two access edges before delete.
        assert_eq!(
            store.list_access_edges(&uid).len(),
            2,
            "two access edges before delete"
        );

        // Delete user — must cascade both edges.
        delete(
            uid.clone(),
            get_user_fn(store.clone()),
            list_access_edges_fn(store.clone()),
            list_blocked_nodes_fn(store.clone()),
            delete_edge_fn(store.clone()),
            delete_user_fn(store.clone()),
        )
        .await
        .expect("delete user");

        // All access edges gone, user gone.
        assert_eq!(
            store.list_access_edges(&uid).len(),
            0,
            "zero access edges after delete"
        );
        assert!(store.get_user(&uid).is_none(), "user should be gone");
    }
}
