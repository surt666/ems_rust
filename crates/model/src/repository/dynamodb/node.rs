//! DynamoDB operations for hierarchy nodes.
//!
//! Mirrors `services/hierarchy/lib/repo/dynamo.ml` node + subtree operations.
//! All item encoding/decoding goes through `codec`; never re-encode here.

use std::collections::HashMap;

use aws_sdk_dynamodb::{
    types::{AttributeValue, Put, TransactWriteItem, Update, WriteRequest},
    Client,
};
use chrono::{DateTime, Utc};

use crate::domain::ids::{Level, NodeId};
use crate::domain::node::Node;
use crate::domain::values::EdgeKind;
use crate::errors::RepositoryError;
use crate::repository::dynamodb::codec::{
    self, AnchorEdgeParams, Item,
};

// ---------------------------------------------------------------------------
// Constants — mirror dynamo.ml / codec.ml
// ---------------------------------------------------------------------------

const COUNTER_SK: &str = "count";
const ACTIVE_SK_PREFIX: &str = "active#";
const HAS_SENSOR_SK_PREFIX: &str = "has_sensor#";
const MAX_ALLOC_RETRIES: u32 = 5;
const COUNTER_INITIAL_N: i64 = 10_000;
const BATCH_DELETE_CHUNK: usize = 25;

// ---------------------------------------------------------------------------
// Counter key helpers — mirror codec.ml
// ---------------------------------------------------------------------------

pub(crate) fn counter_pk_node(level: Level) -> String {
    format!("count#HN{}", level.depth())
}

pub(crate) const COUNTER_PK_SENSOR: &str = "count#S";

fn counter_key(pk: &str) -> Item {
    let mut m = Item::new();
    m.insert("pk".to_string(), AttributeValue::S(pk.to_string()));
    m.insert("sk".to_string(), AttributeValue::S(COUNTER_SK.to_string()));
    m
}

fn pk_key(id: &str) -> Item {
    let mut m = Item::new();
    m.insert("pk".to_string(), AttributeValue::S(id.to_string()));
    m.insert("sk".to_string(), AttributeValue::S(id.to_string()));
    m
}

// ---------------------------------------------------------------------------
// Counter reads/writes — mirror dynamo.ml counter_ops
// ---------------------------------------------------------------------------

/// Read the counter at `pk`; returns `(n, live)` or `None` if missing.
async fn read_counter(client: &Client, table: &str, pk: &str) -> Option<(i64, i64)> {
    let resp = client
        .get_item()
        .table_name(table)
        .set_key(Some(counter_key(pk)))
        .send()
        .await
        .ok()?;
    let item = resp.item?;
    let read_n = |k: &str| -> Option<i64> {
        match item.get(k) {
            Some(AttributeValue::N(s)) => s.parse::<i64>().ok(),
            _ => None,
        }
    };
    let n = read_n("n")?;
    let live = read_n("live").unwrap_or(0);
    Some((n, live))
}

/// Idempotent counter seed: put with `attribute_not_exists(pk)` guard.
async fn seed_counter(client: &Client, table: &str, pk: &str, initial_n: i64) {
    let mut item = Item::new();
    item.insert("pk".to_string(), AttributeValue::S(pk.to_string()));
    item.insert("sk".to_string(), AttributeValue::S(COUNTER_SK.to_string()));
    item.insert("type".to_string(), AttributeValue::S("counter".to_string()));
    item.insert("n".to_string(), AttributeValue::N(initial_n.to_string()));
    item.insert("live".to_string(), AttributeValue::N("0".to_string()));
    let _ = client
        .put_item()
        .table_name(table)
        .set_item(Some(item))
        .condition_expression("attribute_not_exists(pk)")
        .send()
        .await;
}

/// Adjust `live` by `delta` (no condition — used at delete time).
pub(crate) async fn bump_live(client: &Client, table: &str, pk: &str, delta: i64) {
    let _ = client
        .update_item()
        .table_name(table)
        .set_key(Some(counter_key(pk)))
        .update_expression("ADD #live :d")
        .expression_attribute_names("#live", "live")
        .expression_attribute_values(":d", AttributeValue::N(delta.to_string()))
        .send()
        .await;
}

// ---------------------------------------------------------------------------
// Atomic allocate + put — mirror dynamo.ml try_transact_alloc
// ---------------------------------------------------------------------------

/// Result of a single attempt; used internally by the allocation loop.
enum AllocOutcome {
    Ok,
    CounterRace,
    Conflict(String),
    Other(String),
}

async fn try_transact_alloc(
    client: &Client,
    table: &str,
    counter_pk: &str,
    current_n: i64,
    next_n: i64,
    node_item: Item,
    edge_item: Item,
) -> AllocOutcome {
    // 1. Counter update: SET n = :next ADD live :one WHERE n = :current
    let upd = Update::builder()
        .table_name(table)
        .set_key(Some(counter_key(counter_pk)))
        .update_expression("SET #n = :next ADD #live :one")
        .condition_expression("#n = :current")
        .expression_attribute_names("#n", "n")
        .expression_attribute_names("#live", "live")
        .expression_attribute_values(":next", AttributeValue::N(next_n.to_string()))
        .expression_attribute_values(":current", AttributeValue::N(current_n.to_string()))
        .expression_attribute_values(":one", AttributeValue::N("1".to_string()))
        .build()
        .expect("update builder");

    // 2. Node put: condition attribute_not_exists(pk)
    let put_node = Put::builder()
        .table_name(table)
        .set_item(Some(node_item))
        .condition_expression("attribute_not_exists(pk)")
        .build()
        .expect("put_node builder");

    // 3. Edge put: condition attribute_not_exists(sk)
    let put_edge = Put::builder()
        .table_name(table)
        .set_item(Some(edge_item))
        .condition_expression("attribute_not_exists(sk)")
        .build()
        .expect("put_edge builder");

    let items = vec![
        TransactWriteItem::builder().update(upd).build(),
        TransactWriteItem::builder().put(put_node).build(),
        TransactWriteItem::builder().put(put_edge).build(),
    ];

    match client
        .transact_write_items()
        .set_transact_items(Some(items))
        .send()
        .await
    {
        Ok(_) => AllocOutcome::Ok,
        Err(e) => {
            // Parse cancellation reasons (index 0=counter, 1=node, 2=edge)
            let svc = e.into_service_error();
            use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
            if let TransactWriteItemsError::TransactionCanceledException(ref tce) = svc {
                let reasons = tce.cancellation_reasons();
                let code_at = |i: usize| -> &str {
                    reasons.get(i).and_then(|r| r.code()).unwrap_or("")
                };
                let counter_code = code_at(0);
                let node_code = code_at(1);
                let edge_code = code_at(2);
                if counter_code == "ConditionalCheckFailed" {
                    return AllocOutcome::CounterRace;
                }
                if node_code == "ConditionalCheckFailed"
                    || edge_code == "ConditionalCheckFailed"
                {
                    return AllocOutcome::Conflict("id collision on put".to_string());
                }
                AllocOutcome::Other("transact canceled".to_string())
            } else {
                AllocOutcome::Other(svc.to_string())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Edge spec type (mirrors OCaml Effects.edge_spec, local to alloc fns)
// ---------------------------------------------------------------------------

pub struct AllocEdgeSpec {
    pub from_: String,
    pub to_: String,
    pub kind: EdgeKind,
    pub name: String,
    pub created: DateTime<Utc>,
    pub self_path: Option<String>,
}

// ---------------------------------------------------------------------------
// allocate_and_put_node — mirror dynamo.ml
// ---------------------------------------------------------------------------

/// Allocate a new node id for `level`, call `build(id)` → `(Node, AllocEdgeSpec)`,
/// write all three atomically, and return the `Node`.
///
/// Retries up to `MAX_ALLOC_RETRIES` on counter contention; returns
/// `RepositoryError::Conflict` on real id collision or contention exhaustion.
pub async fn allocate_and_put_node<F>(
    client: &Client,
    table: &str,
    level: Level,
    build: F,
) -> Result<Node, RepositoryError>
where
    F: Fn(u32) -> (Node, AllocEdgeSpec),
{
    let counter_pk = counter_pk_node(level);
    let mut remaining = MAX_ALLOC_RETRIES;

    loop {
        if remaining == 0 {
            return Err(RepositoryError::Conflict(
                "counter contention exceeded retries".to_string(),
            ));
        }
        remaining -= 1;

        let current_n = match read_counter(client, table, &counter_pk).await {
            Some((n, _)) => n,
            None => {
                seed_counter(client, table, &counter_pk, COUNTER_INITIAL_N).await;
                COUNTER_INITIAL_N
            }
        };

        let next_n = current_n + 1;
        let (node, edge) = build(next_n as u32);
        let node_item = codec::node_to_item(&node);
        let self_path = edge.self_path.unwrap_or_default();
        let edge_item = codec::anchor_edge_to_item(AnchorEdgeParams {
            from_: &edge.from_,
            to_: &edge.to_,
            kind: &edge.kind,
            name: &edge.name,
            created: &edge.created,
            self_path: &self_path,
        });

        match try_transact_alloc(
            client, table, &counter_pk, current_n, next_n, node_item, edge_item,
        )
        .await
        {
            AllocOutcome::Ok => return Ok(node),
            AllocOutcome::CounterRace => continue,
            AllocOutcome::Conflict(m) => return Err(RepositoryError::Conflict(m)),
            AllocOutcome::Other(m) => return Err(RepositoryError::Aws(m)),
        }
    }
}

// ---------------------------------------------------------------------------
// allocate_and_put_sensor — mirror dynamo.ml
// ---------------------------------------------------------------------------

/// Allocate a new sensor id, call `build(id)` → `(Sensor_item, AllocEdgeSpec)`,
/// write atomically.  Returns the raw item map; callers decode with the codec.
///
/// Uses the `count#S` counter, same retry/seed logic as `allocate_and_put_node`.
pub async fn allocate_and_put_sensor<F>(
    client: &Client,
    table: &str,
    build: F,
) -> Result<Item, RepositoryError>
where
    F: Fn(u32) -> (Item, AllocEdgeSpec),
{
    let counter_pk = COUNTER_PK_SENSOR;
    let mut remaining = MAX_ALLOC_RETRIES;

    loop {
        if remaining == 0 {
            return Err(RepositoryError::Conflict(
                "counter contention exceeded retries".to_string(),
            ));
        }
        remaining -= 1;

        let current_n = match read_counter(client, table, counter_pk).await {
            Some((n, _)) => n,
            None => {
                seed_counter(client, table, counter_pk, COUNTER_INITIAL_N).await;
                COUNTER_INITIAL_N
            }
        };

        let next_n = current_n + 1;
        let (sensor_item, edge) = build(next_n as u32);
        let self_path = edge.self_path.unwrap_or_default();
        let edge_item = codec::anchor_edge_to_item(AnchorEdgeParams {
            from_: &edge.from_,
            to_: &edge.to_,
            kind: &edge.kind,
            name: &edge.name,
            created: &edge.created,
            self_path: &self_path,
        });

        let sensor_item_clone = sensor_item.clone();
        match try_transact_alloc(
            client, table, counter_pk, current_n, next_n, sensor_item_clone, edge_item,
        )
        .await
        {
            AllocOutcome::Ok => return Ok(sensor_item),
            AllocOutcome::CounterRace => continue,
            AllocOutcome::Conflict(m) => return Err(RepositoryError::Conflict(m)),
            AllocOutcome::Other(m) => return Err(RepositoryError::Aws(m)),
        }
    }
}

// ---------------------------------------------------------------------------
// get_node — mirror dynamo.ml `get_item`
// ---------------------------------------------------------------------------

pub async fn get_node(
    client: &Client,
    table: &str,
    id: &NodeId,
) -> Result<Option<Node>, RepositoryError> {
    let resp = client
        .get_item()
        .table_name(table)
        .set_key(Some(pk_key(&id.to_string())))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    match resp.item {
        None => Ok(None),
        // A decode failure (e.g. an unmigrated v1 schema) is a Codec error,
        // not absence — only a missing item maps to Ok(None).
        Some(item) => codec::node_of_item(&item).map(Some),
    }
}

// ---------------------------------------------------------------------------
// put_node — mirror dynamo.ml `put_node`
// ---------------------------------------------------------------------------

pub async fn put_node(
    client: &Client,
    table: &str,
    node: &Node,
) -> Result<(), RepositoryError> {
    client
        .put_item()
        .table_name(table)
        .set_item(Some(codec::node_to_item(node)))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// delete_subtree — mirror dynamo.ml `delete_subtree`
// ---------------------------------------------------------------------------

/// List all levels at or below `starting_level` (inclusive).
fn levels_at_or_below(starting_level: Level) -> Vec<Level> {
    let start_depth = starting_level.depth();
    (start_depth..=9)
        .filter_map(Level::of_depth)
        .collect()
}

/// Page through the GSI partition `gsi1pk_v` for rows whose `gsi1sk` begins
/// with `path_prefix`.  Mirrors `query_gsi_partition`.
async fn query_gsi_partition(
    client: &Client,
    table: &str,
    gsi1pk_v: &str,
    path_prefix: &str,
) -> Vec<Item> {
    let mut acc: Vec<Item> = Vec::new();
    let mut start_key: Option<HashMap<String, AttributeValue>> = None;

    loop {
        let mut req = client
            .query()
            .table_name(table)
            .index_name("gsi1")
            .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
            .expression_attribute_names("#pk", "gsi1pk")
            .expression_attribute_names("#sk", "gsi1sk")
            .expression_attribute_values(":pk", AttributeValue::S(gsi1pk_v.to_string()))
            .expression_attribute_values(":sk", AttributeValue::S(path_prefix.to_string()));

        if let Some(ref k) = start_key {
            req = req.set_exclusive_start_key(Some(k.clone()));
        }

        match req.send().await {
            Err(_) => break,
            Ok(resp) => {
                if let Some(items) = resp.items {
                    acc.extend(items);
                }
                let lek = resp.last_evaluated_key;
                if lek.as_ref().is_none_or(|m| m.is_empty()) {
                    break;
                }
                start_key = lek;
            }
        }
    }
    acc
}

/// Delete a list of `(pk, sk)` pairs in 25-row `BatchWriteItem` chunks.
/// Mirrors `batch_delete`.
async fn batch_delete(client: &Client, table: &str, keys: Vec<(AttributeValue, AttributeValue)>) {
    for chunk in keys.chunks(BATCH_DELETE_CHUNK) {
        let requests: Vec<WriteRequest> = chunk
            .iter()
            .map(|(pk_v, sk_v)| {
                let mut key: HashMap<String, AttributeValue> = HashMap::new();
                key.insert("pk".to_string(), pk_v.clone());
                key.insert("sk".to_string(), sk_v.clone());
                WriteRequest::builder()
                    .delete_request(
                        aws_sdk_dynamodb::types::DeleteRequest::builder()
                            .set_key(Some(key))
                            .build()
                            .expect("delete_request"),
                    )
                    .build()
            })
            .collect();

        let _ = client
            .batch_write_item()
            .request_items(table, requests)
            .send()
            .await;
    }
}

/// Delete the node at `id` and everything beneath it.
///
/// Strategy mirrors `dynamo.ml delete_subtree`:
/// - Query each level partition in GSI with `gsi1sk begins_with self.path`.
/// - Query the `S` sensor partition the same way.
/// - Batch-delete all matched keys.
/// - Decrement live counters per level and for sensors.
pub async fn delete_subtree(
    client: &Client,
    table: &str,
    id: &NodeId,
) -> Result<(), RepositoryError> {
    let node = match get_node(client, table, id).await? {
        None => return Ok(()),
        Some(n) => n,
    };

    let path_prefix = node.path.clone();
    let starting_level = node.id.level();
    let levels = levels_at_or_below(starting_level);

    let mut all_keys: Vec<(AttributeValue, AttributeValue)> = Vec::new();
    let mut level_node_counts: HashMap<Level, i64> = HashMap::new();

    for lvl in &levels {
        let gsi1pk_v = format!("HN{}", lvl.depth());
        let rows = query_gsi_partition(client, table, &gsi1pk_v, &path_prefix).await;
        let mut node_count: i64 = 0;
        for item in &rows {
            if let (Some(pk_v), Some(sk_v)) = (item.get("pk"), item.get("sk")) {
                all_keys.push((pk_v.clone(), sk_v.clone()));
            }
            if matches!(item.get("type"), Some(AttributeValue::S(t)) if t == "node") {
                node_count += 1;
            }
        }
        if node_count > 0 {
            level_node_counts.insert(*lvl, node_count);
        }
    }

    // Sensor partition
    let sensor_rows =
        query_gsi_partition(client, table, "S", &path_prefix).await;
    let mut sensor_count: i64 = 0;
    for item in &sensor_rows {
        if let (Some(pk_v), Some(sk_v)) = (item.get("pk"), item.get("sk")) {
            all_keys.push((pk_v.clone(), sk_v.clone()));
        }
        if matches!(item.get("type"), Some(AttributeValue::S(t)) if t == "sensor")
            && matches!(item.get("sk"), Some(AttributeValue::S(sk)) if sk.starts_with(ACTIVE_SK_PREFIX))
        {
            sensor_count += 1;
        }
    }

    batch_delete(client, table, all_keys).await;

    for (lvl, count) in level_node_counts {
        bump_live(client, table, &counter_pk_node(lvl), -count).await;
    }
    if sensor_count > 0 {
        bump_live(client, table, COUNTER_PK_SENSOR, -sensor_count).await;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// list_child_refs / list_children — mirror dynamo.ml `query_child_refs` / `query_children`
// ---------------------------------------------------------------------------

/// Query child edge rows under `parent`, optionally filtered to a specific kind.
/// Mirrors `query_child_edges`.
async fn query_child_edges(
    client: &Client,
    table: &str,
    parent: &NodeId,
    kind_opt: Option<&EdgeKind>,
) -> Vec<Item> {
    let pk_val = AttributeValue::S(parent.to_string());
    let sk_prefix = match kind_opt {
        Some(k) => format!("{}#", k.sk_verb()),
        None => "has_".to_string(),
    };

    let resp = client
        .query()
        .table_name(table)
        .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
        .expression_attribute_names("#pk", "pk")
        .expression_attribute_names("#sk", "sk")
        .expression_attribute_values(":pk", pk_val)
        .expression_attribute_values(":sk", AttributeValue::S(sk_prefix))
        .send()
        .await;

    match resp {
        Err(_) => vec![],
        Ok(r) => r.items.unwrap_or_default(),
    }
}

/// Extract the child id from an edge sk of the form `"<verb>#<id>"`.
fn child_id_from_edge_sk(sk: &str) -> Option<&str> {
    sk.find('#').map(|i| &sk[i + 1..])
}

/// List `(child_id, edge_name)` pairs for edges from `parent`.
/// Mirrors `query_child_refs`.
pub async fn list_child_refs(
    client: &Client,
    table: &str,
    parent: &NodeId,
    kind_opt: Option<&EdgeKind>,
) -> Result<Vec<(NodeId, String)>, RepositoryError> {
    let rows = query_child_edges(client, table, parent, kind_opt).await;
    let mut result = Vec::new();
    for item in &rows {
        let sk = match item.get("sk") {
            Some(AttributeValue::S(s)) => s.as_str(),
            _ => continue,
        };
        let name = match item.get("name") {
            Some(AttributeValue::S(n)) => n.as_str(),
            _ => continue,
        };
        if let Some(child_s) = child_id_from_edge_sk(sk) {
            if let Ok(child_id) = NodeId::parse(child_s) {
                result.push((child_id, name.to_string()));
            }
        }
    }
    Ok(result)
}

/// List child *nodes* (not sensors) reachable from `parent`.
/// Mirrors `query_children` (skips `has_sensor#` edges).
pub async fn list_children(
    client: &Client,
    table: &str,
    parent: &NodeId,
    kind_opt: Option<&EdgeKind>,
) -> Result<Vec<Node>, RepositoryError> {
    let rows = query_child_edges(client, table, parent, kind_opt).await;
    let mut result = Vec::new();
    for item in &rows {
        let sk = match item.get("sk") {
            Some(AttributeValue::S(s)) => s.as_str(),
            _ => continue,
        };
        // Skip sensor edges
        if sk.starts_with(HAS_SENSOR_SK_PREFIX) {
            continue;
        }
        if let Some(child_s) = child_id_from_edge_sk(sk) {
            if let Ok(child_id) = NodeId::parse(child_s) {
                if let Some(node) = get_node(client, table, &child_id).await? {
                    result.push(node);
                }
            }
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Integration tests (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "it")]
#[cfg(test)]
mod it_tests {
    use super::*;
    use crate::domain::ids::Level;
    use crate::domain::node::make_root;

    async fn it_client() -> (Client, String) {
        let table = std::env::var("HIERARCHY_TABLE")
            .expect("HIERARCHY_TABLE must be set for integration tests");
        let config = aws_config::load_from_env().await;
        let client = Client::new(&config);
        (client, table)
    }

    #[tokio::test]
    #[ignore]
    async fn it_get_node_absent() {
        let (client, table) = it_client().await;
        let id = NodeId::make(Level::Hn5, 999_999_991);
        let result = get_node(&client, &table, &id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn it_put_get_delete_node() {
        let (client, table) = it_client().await;
        let root = make_root();
        put_node(&client, &table, &root).await.unwrap();
        let got = get_node(&client, &table, &root.id).await.unwrap();
        assert!(got.is_some());
        assert_eq!(got.unwrap().name, "root");
    }
}
