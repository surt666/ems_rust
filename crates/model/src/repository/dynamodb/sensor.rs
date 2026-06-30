//! DynamoDB operations for sensors.
//!
//! Sensor id allocation lives in `node.rs` (`allocate_and_put_sensor`).

use std::collections::HashMap;

use aws_sdk_dynamodb::{
    types::{AttributeValue, Delete, Put, TransactWriteItem},
    Client,
};
use chrono::{DateTime, Utc};

use crate::domain::ids::{NodeId, SensorId};
use crate::domain::sensor::Sensor;
use crate::domain::sensor_sk::SensorSk;
use crate::errors::RepositoryError;
use crate::repository::dynamodb::codec;
use crate::repository::dynamodb::node::{
    bump_live, query_gsi_partition, ACTIVE_SK_PREFIX, COUNTER_PK_SENSOR, HAS_SENSOR_SK_PREFIX,
};

// ---------------------------------------------------------------------------
// get_active_sensor
//
// Query pk = sensor_id, sk begins_with "active#", limit 1, descending.
// Returns the most recent active row decoded as a Sensor.
// ---------------------------------------------------------------------------

pub async fn get_active_sensor(
    client: &Client,
    table: &str,
    id: &SensorId,
) -> Result<Option<Sensor>, RepositoryError> {
    let resp = client
        .query()
        .table_name(table)
        .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
        .expression_attribute_names("#pk", "pk")
        .expression_attribute_names("#sk", "sk")
        .expression_attribute_values(":pk", AttributeValue::S(id.to_string()))
        .expression_attribute_values(":sk", AttributeValue::S(ACTIVE_SK_PREFIX.to_string()))
        .limit(1)
        .scan_index_forward(false)
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    let items = resp.items.unwrap_or_default();
    match items.into_iter().next() {
        None => Ok(None),
        Some(item) => match codec::sensor_of_item(&item) {
            Ok(s) => Ok(Some(s)),
            Err(_) => Ok(None),
        },
    }
}

/// The active logical sensor a daq currently belongs to, if any.
///
/// Reads the daq lock partition — `Query pk="DAQ#<daq>"`, `Limit(1)` (the
/// uniqueness invariant guarantees ≤ 1 row) — then loads the owning sensor so
/// the caller can report where it's attached. A direct partition hit, never a
/// scan.
pub async fn find_active_by_daq(
    client: &Client,
    table: &str,
    daq_id: &str,
) -> Result<Option<Sensor>, RepositoryError> {
    let resp = client
        .query()
        .table_name(table)
        .key_condition_expression("#pk = :pk")
        .expression_attribute_names("#pk", "pk")
        .expression_attribute_values(":pk", AttributeValue::S(codec::daq_lock_pk(daq_id)))
        .limit(1)
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    match resp.items.unwrap_or_default().into_iter().next() {
        None => Ok(None),
        Some(lock) => match codec::daq_lock_owner(&lock) {
            None => Ok(None),
            Some((owner_id, _path)) => get_active_sensor(client, table, &owner_id).await,
        },
    }
}

// ---------------------------------------------------------------------------
// list_sensor_ids
//
// Query pk = parent_id, sk begins_with "has_sensor#".
// Strips the prefix to recover the SensorId.
// ---------------------------------------------------------------------------

pub async fn list_sensor_ids(
    client: &Client,
    table: &str,
    parent: &NodeId,
) -> Result<Vec<SensorId>, RepositoryError> {
    let resp = client
        .query()
        .table_name(table)
        .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
        .expression_attribute_names("#pk", "pk")
        .expression_attribute_names("#sk", "sk")
        .expression_attribute_values(":pk", AttributeValue::S(parent.to_string()))
        .expression_attribute_values(
            ":sk",
            AttributeValue::S(HAS_SENSOR_SK_PREFIX.to_string()),
        )
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    let plen = HAS_SENSOR_SK_PREFIX.len();
    let ids = resp
        .items
        .unwrap_or_default()
        .iter()
        .filter_map(|item| match item.get("sk") {
            Some(AttributeValue::S(sk)) if sk.starts_with(HAS_SENSOR_SK_PREFIX) => {
                let rest = &sk[plen..];
                SensorId::parse(rest).ok()
            }
            _ => None,
        })
        .collect();
    Ok(ids)
}

// ---------------------------------------------------------------------------
// list_sensors_under_path
//
// Uses the GSI: gsi1pk = "S", gsi1sk begins_with path_prefix.
// Returns only active rows (sk starts with "active#").
// ---------------------------------------------------------------------------

pub async fn list_sensors_under_path(
    client: &Client,
    table: &str,
    path_prefix: &str,
) -> Result<Vec<Sensor>, RepositoryError> {
    // Sensors shard per company (`gsi1pk = "S#HN2#<id>"`); the prefix always
    // carries the company segment, so derive the partition from it and keep the
    // `begins_with(prefix)` to narrow within the company (building/area).
    let partition = codec::sensor_gsi1pk(path_prefix);
    let rows = query_gsi_partition(client, table, &partition, path_prefix).await;
    let sensors = rows
        .iter()
        .filter(|item| {
            matches!(item.get("sk"), Some(AttributeValue::S(sk)) if sk.starts_with(ACTIVE_SK_PREFIX))
        })
        .filter_map(|item| codec::sensor_of_item(item).ok())
        .collect();
    Ok(sensors)
}

// ---------------------------------------------------------------------------
// transact_replace
//
// Atomic swap: delete old active → put old as history → put new active.
// ---------------------------------------------------------------------------

pub async fn transact_replace(
    client: &Client,
    table: &str,
    old_created: DateTime<Utc>,
    new_sensor: &Sensor,
) -> Result<(), RepositoryError> {
    // Build the old active item to derive its pk/sk.
    // Create a throwaway sensor with old `created` and new fields, encoding it
    // with `active:true` to get the sk, then with `active:false` for the
    // history item.
    let old_sensor = Sensor::builder()
        .id(new_sensor.id)
        .created(old_created)
        .daq_id(new_sensor.daq_id.clone())
        .path(new_sensor.path.clone())
        .purpose(new_sensor.purpose)
        .meter_type(new_sensor.meter_type)
        .unit(new_sensor.unit.clone())
        .formula(new_sensor.formula.clone())
        .resample_minutes(new_sensor.resample_minutes)
        .build();

    // Active sk: "active#<rfc3339Z>"
    let old_active_sk = SensorSk::Active(old_created).to_string();
    let pk_str = old_sensor.id.to_string();

    // Read the current active row to learn the *old* daq, so we can move its lock
    // if the device changed (replace-device). For a same-daq replace (set_formula)
    // this matches the new daq and we skip the swap.
    let old_daq: Option<String> = {
        let mut k: HashMap<String, AttributeValue> = HashMap::new();
        k.insert("pk".to_string(), AttributeValue::S(pk_str.clone()));
        k.insert("sk".to_string(), AttributeValue::S(old_active_sk.clone()));
        client
            .get_item()
            .table_name(table)
            .set_key(Some(k))
            .send()
            .await
            .ok()
            .and_then(|r| r.item)
            .and_then(|it| match it.get("daq_id") {
                Some(AttributeValue::S(d)) => Some(d.clone()),
                _ => None,
            })
    };

    // History item: same sensor, sk = bare rfc3339Z (no "active#" prefix)
    let mut history_item = codec::sensor_to_item(&old_sensor);
    // Override the sk to the history form
    history_item.insert(
        "sk".to_string(),
        AttributeValue::S(SensorSk::History(old_created).to_string()),
    );

    // New active item
    let new_active_item = codec::sensor_to_item(new_sensor);

    // 1. Delete old active row
    let mut old_key: HashMap<String, AttributeValue> = HashMap::new();
    old_key.insert("pk".to_string(), AttributeValue::S(pk_str.clone()));
    old_key.insert("sk".to_string(), AttributeValue::S(old_active_sk));
    let del = Delete::builder()
        .table_name(table)
        .set_key(Some(old_key))
        .build()
        .expect("delete builder");

    // 2. Put history item
    let put_hist = Put::builder()
        .table_name(table)
        .set_item(Some(history_item))
        .build()
        .expect("put_hist builder");

    // 3. Put new active item
    let put_new = Put::builder()
        .table_name(table)
        .set_item(Some(new_active_item))
        .build()
        .expect("put_new builder");

    let mut items = vec![
        TransactWriteItem::builder().delete(del).build(),
        TransactWriteItem::builder().put(put_hist).build(),
        TransactWriteItem::builder().put(put_new).build(),
    ];

    // Move the daq lock atomically with the swap, but only when the device
    // actually changed (replace-device). A same-daq replace (set_formula) leaves
    // the lock alone.
    if let Some(od) = old_daq {
        if od != new_sensor.daq_id {
            let del_lock = Delete::builder()
                .table_name(table)
                .set_key(Some(codec::daq_lock_key(&od, new_sensor.id)))
                .build()
                .expect("del_lock builder");
            let put_lock = Put::builder()
                .table_name(table)
                .set_item(Some(codec::daq_lock_item(
                    &new_sensor.daq_id,
                    new_sensor.id,
                    &new_sensor.path,
                )))
                .build()
                .expect("put_lock builder");
            items.push(TransactWriteItem::builder().delete(del_lock).build());
            items.push(TransactWriteItem::builder().put(put_lock).build());
        }
    }

    client
        .transact_write_items()
        .set_transact_items(Some(items))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// delete_sensor
//
// 1. Query all rows under the sensor's pk partition (active + history).
// 2. Delete each row individually.
// 3. Delete the parent edge row (has_sensor#<sensor_id>).
// 4. Decrement the sensor counter by 1.
// ---------------------------------------------------------------------------

pub async fn delete_sensor(
    client: &Client,
    table: &str,
    sensor_id: &SensorId,
    parent: &NodeId,
) -> Result<(), RepositoryError> {
    let id_s = sensor_id.to_string();

    // Query entire pk partition for the sensor (active + all history rows)
    let resp = client
        .query()
        .table_name(table)
        .key_condition_expression("#pk = :pk")
        .expression_attribute_names("#pk", "pk")
        .expression_attribute_values(":pk", AttributeValue::S(id_s.clone()))
        .send()
        .await
        .map_err(|e| RepositoryError::Aws(e.to_string()))?;

    let items = resp.items.unwrap_or_default();

    // The active row's daq owns a lock row to free (history rows kept old daqs
    // but never held a lock — only the active daq does).
    let active_daq: Option<String> = items.iter().find_map(|it| match (it.get("sk"), it.get("daq_id")) {
        (Some(AttributeValue::S(sk)), Some(AttributeValue::S(d))) if sk.starts_with(ACTIVE_SK_PREFIX) => {
            Some(d.clone())
        }
        _ => None,
    });

    // Delete each sensor row
    for item in &items {
        if let (Some(pk_v), Some(sk_v)) = (item.get("pk"), item.get("sk")) {
            let mut key: HashMap<String, AttributeValue> = HashMap::new();
            key.insert("pk".to_string(), pk_v.clone());
            key.insert("sk".to_string(), sk_v.clone());
            let _ = client
                .delete_item()
                .table_name(table)
                .set_key(Some(key))
                .send()
                .await;
        }
    }

    // Free the daq lock so the daq can be re-attached elsewhere.
    if let Some(daq) = active_daq {
        let _ = client
            .delete_item()
            .table_name(table)
            .set_key(Some(codec::daq_lock_key(&daq, *sensor_id)))
            .send()
            .await;
    }

    // Delete the parent edge row: pk = parent, sk = "has_sensor#<sensor_id>"
    let edge_sk = format!("{}{}", HAS_SENSOR_SK_PREFIX, id_s);
    let mut edge_key: HashMap<String, AttributeValue> = HashMap::new();
    edge_key.insert(
        "pk".to_string(),
        AttributeValue::S(parent.to_string()),
    );
    edge_key.insert("sk".to_string(), AttributeValue::S(edge_sk));
    let _ = client
        .delete_item()
        .table_name(table)
        .set_key(Some(edge_key))
        .send()
        .await;

    // Decrement live counter
    bump_live(client, table, COUNTER_PK_SENSOR, -1).await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Integration tests (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "it")]
#[cfg(test)]
mod it_tests {
    use super::*;
    use crate::domain::ids::Level;
    use crate::domain::values::{MeterType, Resource};

    async fn it_client() -> (Client, String) {
        let table = std::env::var("HIERARCHY_TABLE")
            .expect("HIERARCHY_TABLE must be set for integration tests");
        let config = aws_config::load_from_env().await;
        let client = Client::new(&config);
        (client, table)
    }

    fn make_test_sensor(id: u32, parent_path: &str) -> Sensor {
        let path = format!("{}|S#{}", parent_path, id);
        Sensor::builder()
            .id(SensorId::make(id))
            .created(Utc::now())
            .daq_id(format!("it-test:{}", id))
            .path(path)
            .purpose(Resource::Electricity)
            .meter_type(MeterType::Counter)
            .build()
    }

    #[tokio::test]
    #[ignore]
    async fn it_get_active_sensor_absent() {
        let (client, table) = it_client().await;
        let result = get_active_sensor(&client, &table, &SensorId::make(999_999_990))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn it_list_sensors_under_path_empty() {
        let (client, table) = it_client().await;
        let result = list_sensors_under_path(&client, &table, "HN0#root|HN1#999999888")
            .await
            .unwrap();
        assert!(result.is_empty());
    }
}
