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

use model::domain::ids::{Level, NodeId, UserId};
use model::domain::node::Node;
use model::domain::node_formula::NodeFormula;
use model::domain::sensor::Sensor;
use model::domain::user::User;
use model::domain::values::{EdgeKind, EnergyType, Purpose};
use model::errors::RepositoryError;
use model::logic::formulas::MatrixRow;
use model::repository::dynamodb::{
    edge, node as ddb_node, node_formula as ddb_formula, sensor as ddb_sensor, user,
    weight as ddb_weight,
};
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

// ---------------------------------------------------------------------------
// Node formulas + the materialised coefficient matrix
// ---------------------------------------------------------------------------

pub fn put_node_formula_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(NodeFormula, String, String) -> RepoFut<()> + Clone {
    move |f, node_path, company_path| {
        let t = table.clone();
        Box::pin(async move {
            ddb_formula::put_node_formula(ddb, &t, &f, &node_path, &company_path).await
        })
    }
}

pub fn delete_node_formula_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(NodeId, EnergyType, Purpose) -> RepoFut<()> + Clone {
    move |node, energy_type, purpose| {
        let t = table.clone();
        Box::pin(async move {
            ddb_formula::delete_node_formula(ddb, &t, &node, energy_type, purpose).await
        })
    }
}

pub fn list_company_formulas_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(String) -> RepoFut<Vec<NodeFormula>> + Clone {
    move |company_path| {
        let t = table.clone();
        Box::pin(async move { ddb_formula::list_company_formulas(ddb, &t, &company_path).await })
    }
}

/// Every active sensor in a company. The company path arrives WITHOUT a trailing
/// separator; the GSI prefix query needs one, or `HN2#1` would match `HN2#10`.
pub fn list_company_sensors_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(String) -> RepoFut<Vec<Sensor>> + Clone {
    move |company_path| {
        let t = table.clone();
        Box::pin(async move {
            let prefix = format!("{company_path}{}", model::domain::node::PATH_SEP);
            ddb_sensor::list_sensors_under_path(ddb, &t, &prefix).await
        })
    }
}

/// Every node in a company, the company itself included — the roots of the
/// recursion. Nodes are indexed per LEVEL (`gsi1pk = "HN<d>"`), so this fans out
/// over HN2..HN9 and filters by path.
pub fn list_company_nodes_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(String) -> RepoFut<Vec<Node>> + Clone {
    move |company_path| {
        let t = table.clone();
        Box::pin(async move {
            let mut out = Vec::new();
            for level in [
                Level::Hn2, Level::Hn3, Level::Hn4, Level::Hn5,
                Level::Hn6, Level::Hn7, Level::Hn8, Level::Hn9,
            ] {
                let found =
                    ddb_node::list_by_level_under_path(ddb, &t, level, &company_path).await?;
                out.extend(found);
            }
            Ok(out)
        })
    }
}

pub fn replace_matrix_fn(
    ddb: &'static DynamoClient,
    table: String,
) -> impl Fn(String, Vec<MatrixRow>) -> RepoFut<()> + Clone {
    move |company_path, matrix| {
        let t = table.clone();
        Box::pin(async move {
            ddb_weight::replace_company_matrix(ddb, &t, &company_path, &matrix).await
        })
    }
}
