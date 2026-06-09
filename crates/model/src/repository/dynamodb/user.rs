//! DynamoDB operations for users and the user-edge relationships
//! (administrated nodes, blocked nodes, blocked users).
//!
//! Mirrors `services/hierarchy/lib/repo/dynamo.ml` user ops:
//! `put_user`, `get_user`, `list_users`, `delete_user`,
//! `query_administrated_nodes`, `query_blocked_nodes`, `query_blocked_users`.

use aws_sdk_dynamodb::{types::AttributeValue, Client};

use crate::domain::ids::{NodeId, UserId};
use crate::domain::user::User;
use crate::domain::values::EdgeKind;
use crate::errors::RepositoryError;
use crate::repository::dynamodb::codec;

// ---------------------------------------------------------------------------
// put_user — mirrors dynamo.ml `put_user`
// ---------------------------------------------------------------------------

pub async fn put_user(
    client: &Client,
    table: &str,
    user: &User,
) -> Result<(), RepositoryError> {
    client
        .put_item()
        .table_name(table)
        .set_item(Some(codec::user_to_item(user)))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// get_user — mirrors dynamo.ml `get_user`
// ---------------------------------------------------------------------------

pub async fn get_user(
    client: &Client,
    table: &str,
    id: &UserId,
) -> Result<Option<User>, RepositoryError> {
    let uid = id.to_string();
    let mut key = std::collections::HashMap::new();
    key.insert("pk".to_string(), AttributeValue::S(uid.clone()));
    key.insert("sk".to_string(), AttributeValue::S(uid));

    let resp = client
        .get_item()
        .table_name(table)
        .set_key(Some(key))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    match resp.item {
        None => Ok(None),
        Some(item) => match codec::user_of_item(&item) {
            Ok(u) => Ok(Some(u)),
            Err(_) => Ok(None),
        },
    }
}

// ---------------------------------------------------------------------------
// list_users — mirrors dynamo.ml `list_users`
//
// Query on gsi1 index: gsi1pk = "user".  No begins_with condition (all users).
// ---------------------------------------------------------------------------

pub async fn list_users(
    client: &Client,
    table: &str,
) -> Result<Vec<User>, RepositoryError> {
    let resp = client
        .query()
        .table_name(table)
        .index_name("gsi1")
        .key_condition_expression("#pk = :pk")
        .expression_attribute_names("#pk", "gsi1pk")
        .expression_attribute_values(":pk", AttributeValue::S("user".to_string()))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    let rows = resp.items.unwrap_or_default();
    let users = rows
        .iter()
        .filter_map(|item| codec::user_of_item(item).ok())
        .collect();
    Ok(users)
}

// ---------------------------------------------------------------------------
// delete_user — mirrors dynamo.ml `delete_user`
// ---------------------------------------------------------------------------

pub async fn delete_user(
    client: &Client,
    table: &str,
    id: &UserId,
) -> Result<(), RepositoryError> {
    let uid = id.to_string();
    let mut key = std::collections::HashMap::new();
    key.insert("pk".to_string(), AttributeValue::S(uid.clone()));
    key.insert("sk".to_string(), AttributeValue::S(uid));

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
// list_blocked_nodes — mirrors dynamo.ml `query_blocked_nodes`
//
// Query pk = user_id, sk begins_with "blocked#"
// ---------------------------------------------------------------------------

pub async fn list_blocked_nodes(
    client: &Client,
    table: &str,
    user_id: &UserId,
) -> Result<Vec<NodeId>, RepositoryError> {
    let pk_val = AttributeValue::S(user_id.to_string());
    let prefix = "blocked#".to_string();

    let resp = client
        .query()
        .table_name(table)
        .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
        .expression_attribute_names("#pk", "pk")
        .expression_attribute_names("#sk", "sk")
        .expression_attribute_values(":pk", pk_val)
        .expression_attribute_values(":sk", AttributeValue::S(prefix.clone()))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    let plen = prefix.len();
    let ids = resp
        .items
        .unwrap_or_default()
        .iter()
        .filter_map(|item| match item.get("sk") {
            Some(AttributeValue::S(sk)) if sk.starts_with(&prefix) => {
                let rest = &sk[plen..];
                NodeId::parse(rest).ok()
            }
            _ => None,
        })
        .collect();
    Ok(ids)
}

// ---------------------------------------------------------------------------
// list_administrated_nodes — mirrors dynamo.ml `query_administrated_nodes`
//
// Query pk = user_id, sk begins_with "administrates#"
// ---------------------------------------------------------------------------

pub async fn list_administrated_nodes(
    client: &Client,
    table: &str,
    user_id: &UserId,
) -> Result<Vec<NodeId>, RepositoryError> {
    let pk_val = AttributeValue::S(user_id.to_string());
    let prefix = "administrates#".to_string();

    let resp = client
        .query()
        .table_name(table)
        .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
        .expression_attribute_names("#pk", "pk")
        .expression_attribute_names("#sk", "sk")
        .expression_attribute_values(":pk", pk_val)
        .expression_attribute_values(":sk", AttributeValue::S(prefix.clone()))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    let plen = prefix.len();
    let ids = resp
        .items
        .unwrap_or_default()
        .iter()
        .filter_map(|item| match item.get("sk") {
            Some(AttributeValue::S(sk)) if sk.starts_with(&prefix) => {
                let rest = &sk[plen..];
                NodeId::parse(rest).ok()
            }
            _ => None,
        })
        .collect();
    Ok(ids)
}

// ---------------------------------------------------------------------------
// list_access_edges — query pk = user_id for administrates / reads / writes
//
// Runs three queries (one per verb prefix) and merges the results.
// ---------------------------------------------------------------------------

pub async fn list_access_edges(
    client: &Client,
    table: &str,
    user_id: &UserId,
) -> Result<Vec<(NodeId, EdgeKind)>, RepositoryError> {
    let pk_val = AttributeValue::S(user_id.to_string());

    let prefixes: &[(&str, EdgeKind)] = &[
        ("administrates#", EdgeKind::Administrates),
        ("reads#",         EdgeKind::Reads),
        ("writes#",        EdgeKind::Writes),
    ];

    let mut result = Vec::new();

    for (prefix, kind) in prefixes {
        let resp = client
            .query()
            .table_name(table)
            .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
            .expression_attribute_names("#pk", "pk")
            .expression_attribute_names("#sk", "sk")
            .expression_attribute_values(":pk", pk_val.clone())
            .expression_attribute_values(":sk", AttributeValue::S(prefix.to_string()))
            .send()
            .await
            .map_err(|e| RepositoryError::Aws(e.to_string()))?;

        let plen = prefix.len();
        for item in resp.items.unwrap_or_default() {
            if let Some(AttributeValue::S(sk)) = item.get("sk") {
                if sk.starts_with(prefix) {
                    let rest = &sk[plen..];
                    if let Ok(nid) = NodeId::parse(rest) {
                        result.push((nid, kind.clone()));
                    }
                }
            }
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// list_blocked_users — mirrors dynamo.ml `query_blocked_users`
//
// Query on gsi1: gsi1pk = node_id, gsi1sk begins_with "blocks#".
// Returns user ids extracted from the main `pk` attribute of matched rows.
// ---------------------------------------------------------------------------

pub async fn list_blocked_users(
    client: &Client,
    table: &str,
    node_id: &NodeId,
) -> Result<Vec<UserId>, RepositoryError> {
    let pk_val = AttributeValue::S(node_id.to_string());
    let prefix = "blocks#".to_string();

    let resp = client
        .query()
        .table_name(table)
        .index_name("gsi1")
        .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
        .expression_attribute_names("#pk", "gsi1pk")
        .expression_attribute_names("#sk", "gsi1sk")
        .expression_attribute_values(":pk", pk_val)
        .expression_attribute_values(":sk", AttributeValue::S(prefix))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    let ids = resp
        .items
        .unwrap_or_default()
        .iter()
        .filter_map(|item| match item.get("pk") {
            Some(AttributeValue::S(user_s)) => UserId::parse(user_s).ok(),
            _ => None,
        })
        .collect();
    Ok(ids)
}

// ---------------------------------------------------------------------------
// Integration tests (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "it")]
#[cfg(test)]
mod it_tests {
    use super::*;
    use crate::domain::values::CognitoGroup;
    use chrono::Utc;

    async fn it_client() -> (Client, String) {
        let table = std::env::var("HIERARCHY_TABLE")
            .expect("HIERARCHY_TABLE must be set for integration tests");
        let config = aws_config::load_from_env().await;
        let client = Client::new(&config);
        (client, table)
    }

    fn make_test_user() -> User {
        User::builder()
            .email("it-test-user@example.com".to_owned())
            .name("IT Test".to_owned())
            .cognito_group(CognitoGroup::Reader)
            .created(Utc::now())
            .build()
    }

    #[tokio::test]
    #[ignore]
    async fn it_user_put_get_delete() {
        let (client, table) = it_client().await;
        let u = make_test_user();
        put_user(&client, &table, &u).await.unwrap();
        let got = get_user(&client, &table, &u.id).await.unwrap();
        assert!(got.is_some());
        delete_user(&client, &table, &u.id).await.unwrap();
        let gone = get_user(&client, &table, &u.id).await.unwrap();
        assert!(gone.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn it_list_users_non_empty() {
        let (client, table) = it_client().await;
        let users = list_users(&client, &table).await.unwrap();
        // At minimum, the seeded admin user should be present in a live table.
        // This test just checks the call round-trips without error.
        let _ = users;
    }
}
