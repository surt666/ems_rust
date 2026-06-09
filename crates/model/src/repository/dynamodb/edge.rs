//! DynamoDB operations for hierarchy edges.
//!
//! Mirrors `services/hierarchy/lib/repo/dynamo.ml` edge operations:
//! `put_edge_spec` and `delete_edge`.

use aws_sdk_dynamodb::{types::AttributeValue, Client};
use chrono::DateTime;
use chrono::Utc;

use crate::domain::values::EdgeKind;
use crate::errors::RepositoryError;
use crate::repository::dynamodb::codec::{self, AnchorEdgeParams, EdgeParams};

// ---------------------------------------------------------------------------
// Edge put — mirrors dynamo.ml `put_edge_spec`
// ---------------------------------------------------------------------------

/// Write an edge item.
///
/// When `self_path` is `Some`, uses `anchor_edge_to_item` (hierarchy-side
/// `has_<label>` / `has_sensor` edges).  When `None`, uses the generic
/// `edge_to_item` (user-side `administrates` / `blocked` edges).
///
/// Mirrors OCaml `put_edge_spec`.
#[allow(clippy::too_many_arguments)]
pub async fn put_edge(
    client: &Client,
    table: &str,
    from_: &str,
    to_: &str,
    kind: &EdgeKind,
    name: &str,
    created: &DateTime<Utc>,
    self_path: Option<&str>,
) -> Result<(), RepositoryError> {
    let item = if let Some(path) = self_path {
        codec::anchor_edge_to_item(AnchorEdgeParams {
            from_,
            to_,
            kind,
            name,
            created,
            self_path: path,
        })
    } else {
        codec::edge_to_item(EdgeParams {
            from_,
            to_,
            kind,
            name,
            created,
        })
    };

    client
        .put_item()
        .table_name(table)
        .set_item(Some(item))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Edge delete — mirrors dynamo.ml `delete_edge`
// ---------------------------------------------------------------------------

/// Delete a single edge item identified by `(from_, to_, kind)`.
///
/// sk = `"<sk_verb>#<to_>"`.  Mirrors OCaml `delete_edge`.
pub async fn delete_edge(
    client: &Client,
    table: &str,
    from_: &str,
    to_: &str,
    kind: &EdgeKind,
) -> Result<(), RepositoryError> {
    let sk = format!("{}#{}", kind.sk_verb(), to_);
    let mut key = std::collections::HashMap::new();
    key.insert("pk".to_string(), AttributeValue::S(from_.to_string()));
    key.insert("sk".to_string(), AttributeValue::S(sk));

    client
        .delete_item()
        .table_name(table)
        .set_key(Some(key))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Integration tests (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "it")]
#[cfg(test)]
mod it_tests {
    use super::*;
    use crate::domain::ids::{Level, NodeId};
    use chrono::Utc;

    async fn it_client() -> (Client, String) {
        let table = std::env::var("HIERARCHY_TABLE")
            .expect("HIERARCHY_TABLE must be set for integration tests");
        let config = aws_config::load_from_env().await;
        let client = Client::new(&config);
        (client, table)
    }

    #[tokio::test]
    #[ignore]
    async fn it_put_delete_edge_roundtrip() {
        let (client, table) = it_client().await;
        let from_ = NodeId::root().to_string();
        let to_ = NodeId::make(Level::Hn1, 999_998).to_string();
        let kind = EdgeKind::HasLabel("building".to_string());
        let created = Utc::now();
        let path = format!("HN0#root|{}", to_);

        put_edge(&client, &table, &from_, &to_, &kind, "test", &created, Some(&path))
            .await
            .unwrap();
        delete_edge(&client, &table, &from_, &to_, &kind)
            .await
            .unwrap();
    }
}
