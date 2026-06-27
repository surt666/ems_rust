//! Named repository-closure factories for the prod dispatch/query entry points.
//!
//! `dispatch::run` and `query::run_query` need the same
//! `move |x| async move { ddb_*::call(ddb, &table, &x).await }` adapter for ~12
//! distinct repository calls, at both `Fn` and `FnOnce` call sites. Spelling
//! each one inline made every arm rebuild the closure (and inconsistently single-
//! vs double-clone the table name). These factories build them once: each returns
//! an `Fn + Clone` closure (so it satisfies both `Fn` and `FnOnce` bounds) whose
//! future is boxed (`RepoFut`) so the otherwise-unnameable async-block type can
//! appear in the `impl Fn(..) -> RepoFut<T>` return position.

use std::future::Future;
use std::pin::Pin;

use aws_sdk_dynamodb::Client as DynamoClient;

use model::domain::ids::{NodeId, UserId};
use model::domain::node::Node;
use model::domain::user::User;
use model::domain::values::EdgeKind;
use model::errors::RepositoryError;
use model::repository::dynamodb::{edge, node as ddb_node, user};
use model::repository::EdgeSpec;

/// A boxed, `Send` repository future. Names the async-block type so the factories
/// below can return `impl Fn(..) -> RepoFut<T>`.
pub type RepoFut<T> = Pin<Box<dyn Future<Output = Result<T, RepositoryError>> + Send>>;

pub fn get_node_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(NodeId) -> RepoFut<Option<Node>> + Clone {
    move |id| {
        let t = table.clone();
        Box::pin(async move { ddb_node::get_node(ddb, &t, &id).await })
    }
}

pub fn get_user_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(UserId) -> RepoFut<Option<User>> + Clone {
    move |id| {
        let t = table.clone();
        Box::pin(async move { user::get_user(ddb, &t, &id).await })
    }
}

pub fn put_user_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(User) -> RepoFut<()> + Clone {
    move |u| {
        let t = table.clone();
        Box::pin(async move { user::put_user(ddb, &t, &u).await })
    }
}

pub fn put_node_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(Node) -> RepoFut<()> + Clone {
    move |n| {
        let t = table.clone();
        Box::pin(async move { ddb_node::put_node(ddb, &t, &n).await })
    }
}

pub fn put_edge_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(EdgeSpec) -> RepoFut<()> + Clone {
    move |spec: EdgeSpec| {
        let t = table.clone();
        Box::pin(async move {
            edge::put_edge(
                ddb,
                &t,
                &spec.from_,
                &spec.to_,
                &spec.kind,
                &spec.name,
                &chrono::Utc::now(),
                None,
            )
            .await
        })
    }
}

pub fn delete_edge_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(String, String, EdgeKind) -> RepoFut<()> + Clone {
    move |from_, to_, kind| {
        let t = table.clone();
        Box::pin(async move { edge::delete_edge(ddb, &t, &from_, &to_, &kind).await })
    }
}

pub fn delete_user_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(UserId) -> RepoFut<()> + Clone {
    move |id| {
        let t = table.clone();
        Box::pin(async move { user::delete_user(ddb, &t, &id).await })
    }
}

pub fn list_access_edges_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(UserId) -> RepoFut<Vec<(NodeId, EdgeKind)>> + Clone {
    move |id| {
        let t = table.clone();
        Box::pin(async move { user::list_access_edges(ddb, &t, &id).await })
    }
}

pub fn list_blocked_nodes_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(UserId) -> RepoFut<Vec<NodeId>> + Clone {
    move |id| {
        let t = table.clone();
        Box::pin(async move { user::list_blocked_nodes(ddb, &t, &id).await })
    }
}

pub fn list_child_refs_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(NodeId, Option<EdgeKind>) -> RepoFut<Vec<(NodeId, String)>> + Clone {
    move |pid, kind: Option<EdgeKind>| {
        let t = table.clone();
        Box::pin(async move { ddb_node::list_child_refs(ddb, &t, &pid, kind.as_ref()).await })
    }
}

pub fn list_children_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(NodeId, Option<EdgeKind>) -> RepoFut<Vec<Node>> + Clone {
    move |pid, kind: Option<EdgeKind>| {
        let t = table.clone();
        Box::pin(async move { ddb_node::list_children(ddb, &t, &pid, kind.as_ref()).await })
    }
}

pub fn list_users_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn() -> RepoFut<Vec<User>> + Clone {
    move || {
        let t = table.clone();
        Box::pin(async move { user::list_users(ddb, &t).await })
    }
}

// ---------------------------------------------------------------------------
// Error mapping (shared by the JSON + HTML repo-error formatters)
// ---------------------------------------------------------------------------

/// Map a `RepositoryError` to `(http_status, error_code, message)`.
///
/// Shared by the JSON formatters (`dispatch::repo_error_response`,
/// `query::repo_error`) and the HTML formatter (`query::html_repo_error`). The two
/// JSON formatters special-case `Validation` (to emit the per-error `details`
/// array) *before* calling this, so the `Validation` arm here only ever serves the
/// HTML formatter — which uses the first error's message and ignores the code.
pub fn repo_parts(e: &RepositoryError) -> (u16, &'static str, String) {
    match e {
        RepositoryError::NotFound(id) => (404, "Not_found", format!("{} not found", id)),
        RepositoryError::NotFoundUser(id) => (404, "Not_found", format!("{} not found", id)),
        RepositoryError::Conflict(m) => (409, "Conflict", m.clone()),
        RepositoryError::BadRequest(m) => (400, "Bad_request", m.clone()),
        RepositoryError::Validation(errs) => (
            400,
            "Validation",
            errs.first()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "validation failed".to_string()),
        ),
        RepositoryError::SchemaMissing(id) => {
            (400, "Schema_missing", format!("no hn2 schema found above {}", id))
        }
        RepositoryError::Codec(m) => (500, "Internal", m.clone()),
        RepositoryError::Aws(m) => (500, "Internal", m.clone()),
    }
}
