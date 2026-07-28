//! Sensors logic.
//!
//! All repository operations are injected as async closures; no traits are used.

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::domain::ids::{NodeId, SensorId};
use crate::domain::node::{child_path, Node};
use crate::domain::sensor::Sensor;
use crate::domain::values::{EdgeKind, ReadingKind, EnergyType};
use crate::errors::RepositoryError;
use crate::logic::schema_check;
use crate::repository::EdgeSpec as RepoEdgeSpec;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn validation_err(msg: impl Into<String>) -> RepositoryError {
    RepositoryError::Validation(vec![crate::domain::schema::MetadataError {
        path: String::new(),
        message: msg.into(),
    }])
}

/// Construct a synthetic `NotFound` for a sensor.
const fn sensor_not_found(id: SensorId) -> RepositoryError {
    RepositoryError::NotFound(NodeId::make(crate::domain::ids::Level::Hn9, id.id()))
}

// ---------------------------------------------------------------------------
// attach
// ---------------------------------------------------------------------------

/// Attach a new sensor to `parent`.
#[allow(clippy::too_many_arguments)]
pub async fn attach<FGN, FGNFut, FAS, FASFut, FFD>(
    parent: NodeId,
    daq_id: String,
    energy_type: EnergyType,
    reading_kind: ReadingKind,
    unit: Option<String>,
    resample_minutes: Option<i32>,
    get_node: FGN,
    add_sensor: FAS,
    find_active_by_daq: FFD,
) -> Result<Sensor, RepositoryError>
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FAS: FnOnce(Box<dyn Fn(u32) -> (Sensor, RepoEdgeSpec) + Send>) -> FASFut,
    FASFut: Future<Output = Result<Sensor, RepositoryError>>,
    FFD: Fn(&str) -> Option<Sensor>,
{
    // Validate parent exists.
    let parent_node = get_node(parent.clone())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(parent.clone()))?;

    // Business rule: a physical device (`daq_id`) feeds at most ONE active logical
    // meter. Attaching a daq_id that's already active on another logical is a
    // conflict — the device must be moved (its old attachment replaced/removed)
    // first. (A daq_id is movable; a logical_id is fixed to its node.)
    if let Some(existing) = find_active_by_daq(&daq_id) {
        return Err(daq_already_attached(&daq_id, &existing));
    }

    // Validate resample_minutes if set.
    if let Some(b) = resample_minutes {
        if b <= 0 {
            return Err(validation_err("resample_minutes must be > 0 minutes"));
        }
    }

    // Check schema allows sensors on this node type.
    let (_host, schema) = schema_check::find_for(parent.clone(), &get_node).await?;
    if !schema.allows_sensors(&parent_node.label) {
        return Err(validation_err(format!(
            "sensors not allowed on {:?} nodes",
            parent_node.label
        )));
    }

    // Capture state for the build closure.
    // The closure is `Fn` (not `FnOnce`) so it can be invoked on every retry
    // of the counter's ConditionalCheckFailed loop — each call clones its
    // captured state independently.
    let parent_path = parent_node.path.clone();
    let parent_clone = parent.clone();
    // Allocate sensor and write edge atomically.
    let s = add_sensor(Box::new(move |raw_id| {
        let sid = SensorId::make(raw_id);
        let path = child_path(&parent_path, &sid.to_string());
        let sensor = Sensor::builder()
            .id(sid)
            .created(chrono::Utc::now())
            .daq_id(daq_id.clone())
            .path(path)
            .energy_type(energy_type)
            .reading_kind(reading_kind)
            .unit(unit.clone())
            .resample_minutes(resample_minutes)
            .build();
        let edge = RepoEdgeSpec {
            from_: parent_clone.to_string(),
            to_: sensor.id.to_string(),
            kind: EdgeKind::HasSensor,
            name: String::new(),
        };
        (sensor, edge)
    }))
    .await?;

    Ok(s)
}

// ---------------------------------------------------------------------------
// list_active
// ---------------------------------------------------------------------------

/// List all active sensors attached to `parent`.
pub async fn list_active<FLS, FLSFut, FGA>(
    parent: NodeId,
    list_sensor_ids: FLS,
    get_active_sensor: FGA,
) -> Result<Vec<Sensor>, RepositoryError>
where
    FLS: FnOnce(NodeId) -> FLSFut,
    FLSFut: Future<Output = Result<Vec<SensorId>, RepositoryError>>,
    FGA: Fn(SensorId) -> Option<Sensor>,
{
    let ids = list_sensor_ids(parent).await?;
    let sensors = ids
        .into_iter()
        .filter_map(get_active_sensor)
        .collect();
    Ok(sensors)
}

// ---------------------------------------------------------------------------
// get_active
// ---------------------------------------------------------------------------

/// Fetch the active sensor for `id`, or error if absent.
pub fn get_active<FGA>(id: SensorId, get_active_sensor: FGA) -> Result<Sensor, RepositoryError>
where
    FGA: FnOnce(SensorId) -> Option<Sensor>,
{
    get_active_sensor(id)
        .ok_or_else(|| sensor_not_found(id))
}

// ---------------------------------------------------------------------------
// Revision helper
// ---------------------------------------------------------------------------

/// A fresh copy of `old` with `created` advanced to now; the caller overwrites
/// the single field it is changing via struct-update syntax.
fn revised(old: &Sensor) -> Sensor {
    Sensor { created: chrono::Utc::now(), ..old.clone() }
}

// ---------------------------------------------------------------------------
// replace_device
// ---------------------------------------------------------------------------

/// Replace the daq device on a sensor (bumps `created`).
pub async fn replace_device<FGA, FFD, FRD, FRDFut>(
    sensor_id: SensorId,
    new_daq_id: String,
    get_active_sensor: FGA,
    find_active_by_daq: FFD,
    replace_sensor_device: FRD,
) -> Result<Sensor, RepositoryError>
where
    FGA: FnOnce(SensorId) -> Option<Sensor>,
    FFD: FnOnce(&str) -> Option<Sensor>,
    FRD: FnOnce(DateTime<Utc>, Sensor) -> FRDFut,
    FRDFut: Future<Output = Result<(), RepositoryError>>,
{
    let old = get_active_sensor(sensor_id).ok_or_else(|| sensor_not_found(sensor_id))?;

    // Same one-active-logical-per-daq rule as `attach`: the *new* device must not
    // already feed a different logical meter. (Re-applying this logical's own daq
    // is a no-op and allowed.)
    if let Some(existing) = find_active_by_daq(&new_daq_id) {
        if existing.id != sensor_id {
            return Err(daq_already_attached(&new_daq_id, &existing));
        }
    }

    let old_created = old.created;
    let new_sensor = Sensor { daq_id: new_daq_id, ..revised(&old) };
    replace_sensor_device(old_created, new_sensor.clone()).await?;
    Ok(new_sensor)
}

/// The conflict raised when a `daq_id` is already attached to another active
/// logical meter — shared by `attach` and `replace_device`.
fn daq_already_attached(daq_id: &str, existing: &Sensor) -> RepositoryError {
    RepositoryError::Conflict(format!(
        "daq_id {:?} is already attached to logical meter {} at {}; move it first",
        daq_id, existing.id, existing.path
    ))
}

// ---------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------

/// Delete a sensor and everything that hangs off it.
///
/// The domain rules — mirroring `attach`/`replace_device`, so the orchestration
/// and the must-exist guard live here rather than in the service:
///   * the sensor must currently be active, else it's a `NotFound` (→ 404);
///   * the `has_sensor` edge to remove hangs off the sensor's **parent** node,
///     which is resolved from the active row's path (`Sensor::parent_id`).
///
/// `delete_sensor` (injected) performs the cross-row removal — active + history
/// rows, the daq lock, the parent edge, and the node's sensor-counter decrement.
/// Returns the deleted sensor's id on success.
pub async fn delete<FGA, FDS, FDSFut>(
    sensor_id: SensorId,
    get_active_sensor: FGA,
    delete_sensor: FDS,
) -> Result<SensorId, RepositoryError>
where
    FGA: FnOnce(SensorId) -> Option<Sensor>,
    FDS: FnOnce(SensorId, NodeId) -> FDSFut,
    FDSFut: Future<Output = Result<(), RepositoryError>>,
{
    let sensor = get_active_sensor(sensor_id).ok_or_else(|| sensor_not_found(sensor_id))?;
    delete_sensor(sensor_id, sensor.parent_id()).await?;
    Ok(sensor_id)
}

// ---------------------------------------------------------------------------
// list_under_company
// ---------------------------------------------------------------------------

/// List all active sensors in the same HN2 company subtree as `parent`.
pub async fn list_under_company<FGN, FGNFut, FLSP, FLSPFut>(
    parent: NodeId,
    get_node: FGN,
    list_sensors_under_path: FLSP,
) -> Result<Vec<Sensor>, RepositoryError>
where
    FGN: FnOnce(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FLSP: FnOnce(String) -> FLSPFut,
    FLSPFut: Future<Output = Result<Vec<Sensor>, RepositoryError>>,
{
    let node = get_node(parent.clone())
        .await?
        .ok_or(RepositoryError::NotFound(parent))?;

    match company_prefix_of_path(&node.path) {
        None => Ok(vec![]),
        Some(prefix) => list_sensors_under_path(prefix).await,
    }
}

// ---------------------------------------------------------------------------
// company_prefix_of_path (internal)
// ---------------------------------------------------------------------------

/// The company prefix **with** a trailing `|` — the `begins_with` form, which
/// keeps `HN2#1` from matching `HN2#10`.
fn company_prefix_of_path(path: &str) -> Option<String> {
    crate::domain::node::company_prefix(path).map(|p| format!("{p}{}", crate::domain::node::PATH_SEP))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use chrono::{DateTime, Utc};

    use super::*;
    use crate::domain::ids::{Level, NodeId, SensorId};
    use crate::domain::node;
    use crate::domain::schema::{EdgeSpec as SchemaEdgeSpec, Schema};
    use crate::repository::EdgeSpec as TestRepoEdgeSpec;
    use crate::domain::values::{EdgeKind, ReadingKind, EnergyType};
    use crate::errors::RepositoryError;
    use crate::logic::hierarchy;
    use crate::repository::memory::Store;

    fn ts() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn sample_schema() -> Schema {
        Schema {
            version: 2,
            edges: vec![
                ("company".to_string(), vec![
                    ("group".to_string(), SchemaEdgeSpec::builder().build()),
                    ("building".to_string(), SchemaEdgeSpec::builder().build()),
                ]),
                ("group".to_string(), vec![
                    ("building".to_string(), SchemaEdgeSpec::builder().build()),
                ]),
                ("building".to_string(), vec![
                    ("area".to_string(), SchemaEdgeSpec::builder().build()),
                ]),
            ],
            metadata: vec![],
            sensors: vec!["building".to_string()],
        }
    }

    fn parent_path_for_hn2() -> String {
        format!(
            "{}|{}",
            NodeId::root(),
            NodeId::make(Level::Hn1, 10001)
        )
    }

    fn seed_company(store: &Rc<Store>) -> NodeId {
        let c2 = NodeId::make(Level::Hn2, 10002);
        let mut n2 = node::make(
            10002,
            Level::Hn2,
            "Acme",
            NodeId::root(),
            &parent_path_for_hn2(),
            serde_json::json!({}),
            Some(sample_schema()),
        );
        n2.label = "company".to_string();
        store.put_node(&n2);
        c2
    }

    async fn seed_building(store: &Rc<Store>, c2: NodeId) -> NodeId {
        let n = hierarchy::add_node(
            c2,
            Some(Level::Hn3),
            Some("building".to_string()),
            "B".to_string(),
            serde_json::json!({}),
            None,
            {
                let s = store.clone();
                move |nid| std::future::ready(Ok(s.get_node(&nid)))
            },
            {
                let s = store.clone();
                move |nid, kind| std::future::ready(Ok(s.list_children(&nid, kind.as_ref())))
            },
            {
                let s = store.clone();
                move |level, build: Box<dyn Fn(u32) -> (_, _) + Send>| {
                    let n = s.add_node(level, build);
                    std::future::ready(Ok(n))
                }
            },
        )
        .await
        .expect("seed building");
        n.id
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

    fn get_active_fn(s: Rc<Store>) -> impl Fn(SensorId) -> Option<Sensor> + Clone {
        move |sid| s.get_active_sensor(&sid)
    }

    fn add_sensor_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(Box<dyn Fn(u32) -> (Sensor, TestRepoEdgeSpec) + Send>) -> std::future::Ready<Result<Sensor, RepositoryError>>
    {
        move |build| {
            let sensor = s.add_sensor(build);
            std::future::ready(Ok(sensor))
        }
    }

    fn delete_sensor_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(SensorId, NodeId) -> std::future::Ready<Result<(), RepositoryError>>
    {
        move |sid, parent| {
            s.delete_sensor(&sid, &parent);
            std::future::ready(Ok(()))
        }
    }

    fn replace_sensor_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(DateTime<Utc>, Sensor) -> std::future::Ready<Result<(), RepositoryError>>
    {
        move |old_created, new_sensor| {
            s.replace_sensor_device(old_created, &new_sensor);
            std::future::ready(Ok(()))
        }
    }

    fn list_sensor_ids_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(NodeId) -> std::future::Ready<Result<Vec<SensorId>, RepositoryError>>
    {
        move |nid| std::future::ready(Ok(s.list_sensor_ids(&nid)))
    }

    fn list_sensors_under_path_fn(
        s: Rc<Store>,
    ) -> impl FnOnce(String) -> std::future::Ready<Result<Vec<Sensor>, RepositoryError>>
    {
        move |prefix| std::future::ready(Ok(s.list_sensors_under_path(&prefix)))
    }

    /// Convenience: attach a sensor with standard defaults.
    async fn do_attach(
        store: Rc<Store>,
        parent: NodeId,
        daq_id: &str,
        resample_minutes: Option<i32>,
    ) -> Result<Sensor, RepositoryError> {
        attach(
            parent,
            daq_id.to_string(),
            EnergyType::Electricity,
            ReadingKind::Counter,
            Some("kWh".to_string()),
            resample_minutes,
            get_node_fn(store.clone()),
            add_sensor_fn(store.clone()),
            {
                let s = store.clone();
                move |daq: &str| s.find_active_by_daq(daq)
            },
        )
        .await
    }

    /// A daq_id already active on one logical can't be attached to a second.
    #[tokio::test]
    async fn rejects_daq_already_active_elsewhere() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let b1 = seed_building(&store, c2.clone()).await;
        let b2 = seed_building(&store, c2).await;

        do_attach(store.clone(), b1, "daq:dup", Some(15))
            .await
            .expect("first attach ok");

        let err = do_attach(store.clone(), b2, "daq:dup", Some(15))
            .await
            .expect_err("second attach of the same daq must conflict");
        assert!(matches!(err, RepositoryError::Conflict(_)), "got {err:?}");
    }

    /// `delete` removes the active sensor and returns its id.
    #[tokio::test]
    async fn delete_removes_active_sensor() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let b1 = seed_building(&store, c2).await;
        let s = do_attach(store.clone(), b1, "daq:del", Some(15))
            .await
            .expect("attach ok");

        let deleted = delete(s.id, get_active_fn(store.clone()), delete_sensor_fn(store.clone()))
            .await
            .expect("delete ok");
        assert_eq!(deleted, s.id);
        assert!(store.get_active_sensor(&s.id).is_none(), "sensor must be gone");
    }

    /// `delete` on a sensor that isn't active is a `NotFound` (→ 404), so the
    /// must-exist guard lives in the domain rather than the service.
    #[tokio::test]
    async fn delete_unknown_is_not_found() {
        let store = Rc::new(Store::new());
        let err = delete(
            SensorId::make(9_999_999),
            get_active_fn(store.clone()),
            delete_sensor_fn(store.clone()),
        )
        .await
        .expect_err("deleting a missing sensor must fail");
        assert!(matches!(err, RepositoryError::NotFound(_)), "got {err:?}");
    }

    // -----------------------------------------------------------------------
    // Test: replace_device_rejects_daq_owned_elsewhere
    //
    // Same one-active-logical-per-daq rule on the *replace* path: moving a
    // sensor's device onto a daq that already feeds another logical conflicts.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn replace_device_rejects_daq_owned_elsewhere() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let b1 = seed_building(&store, c2.clone()).await;
        let b2 = seed_building(&store, c2).await;

        let s1 = do_attach(store.clone(), b1, "daq:A", Some(15))
            .await
            .expect("attach s1");
        do_attach(store.clone(), b2, "daq:B", Some(15))
            .await
            .expect("attach s2");

        // Re-point s1 onto daq:B (owned by s2) → conflict.
        let err = replace_device(
            s1.id,
            "daq:B".to_string(),
            {
                let st = store.clone();
                move |sid| st.get_active_sensor(&sid)
            },
            {
                let st = store.clone();
                move |daq: &str| st.find_active_by_daq(daq)
            },
            replace_sensor_fn(store.clone()),
        )
        .await
        .expect_err("replace onto an owned daq must conflict");
        assert!(matches!(err, RepositoryError::Conflict(_)), "got {err:?}");
    }

    // -----------------------------------------------------------------------
    // Test: attach_happy
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn attach_happy() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s = do_attach(store.clone(), bldg.clone(), "daq:1", Some(15))
            .await
            .expect("attach should succeed");

        assert_eq!(s.daq_id, "daq:1");
        assert_eq!(s.parent_id().to_string(), bldg.to_string());
    }

    // -----------------------------------------------------------------------
    // Test: rejects_level_not_allowed
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_level_not_allowed() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        // Sensors allowed at Hn3 but not Hn2 in sample_schema.
        let result = do_attach(store.clone(), c2, "daq:2", Some(15)).await;

        match result {
            Err(RepositoryError::Validation(_)) => {} // expected
            Ok(_) => panic!("expected Validation at disallowed level"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: sensors are allowed by node TYPE, regardless of depth.
    // -----------------------------------------------------------------------

    /// Helper: add a schema-governed child node, returning the created Node.
    async fn add_child(
        store: &Rc<Store>,
        parent: NodeId,
        label: &str,
        name: &str,
    ) -> node::Node {
        hierarchy::add_node(
            parent,
            None,
            Some(label.to_string()),
            name.to_string(),
            serde_json::json!({}),
            None,
            {
                let s = store.clone();
                move |nid| std::future::ready(Ok(s.get_node(&nid)))
            },
            {
                let s = store.clone();
                move |nid, kind| std::future::ready(Ok(s.list_children(&nid, kind.as_ref())))
            },
            {
                let s = store.clone();
                move |level, build: Box<dyn Fn(u32) -> (_, _) + Send>| {
                    let n = s.add_node(level, build);
                    std::future::ready(Ok(n))
                }
            },
        )
        .await
        .expect("add_child")
    }

    #[tokio::test]
    async fn sensors_by_type_at_any_depth() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        // building B1 directly under company → hn3, label "building"
        let b1 = add_child(&store, c2.clone(), "building", "B1").await;
        assert_eq!(b1.level(), Level::Hn3);
        assert_eq!(b1.label, "building");

        // group G under company → hn3, label "group"
        let g = add_child(&store, c2.clone(), "group", "G").await;
        assert_eq!(g.label, "group");

        // building B2 under group → hn4, label "building"
        let b2 = add_child(&store, g.id.clone(), "building", "B2").await;
        assert_eq!(b2.level(), Level::Hn4);
        assert_eq!(b2.label, "building");

        // attach to building at hn3 → Ok
        assert!(do_attach(store.clone(), b1.id.clone(), "daq:b1", Some(15))
            .await
            .is_ok());
        // attach to building at hn4 → Ok (same type, different depth)
        assert!(do_attach(store.clone(), b2.id.clone(), "daq:b2", Some(15))
            .await
            .is_ok());
        // attach to group → rejected, message names the type
        let err = do_attach(store.clone(), g.id.clone(), "daq:g", Some(15))
            .await
            .unwrap_err();
        // The validation message names the rejected node type.
        let msg = match &err {
            RepositoryError::Validation(errs) => errs[0].message.clone(),
            other => panic!("expected Validation, got {:?}", other),
        };
        assert!(
            msg.contains("not allowed on \"group\""),
            "got: {}",
            msg
        );
    }

    // -----------------------------------------------------------------------
    // Test: list_active_returns_attached
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_active_returns_attached() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        do_attach(store.clone(), bldg.clone(), "daq:1", Some(15))
            .await
            .expect("attach");

        let sensors = list_active(
            bldg,
            list_sensor_ids_fn(store.clone()),
            get_active_fn(store.clone()),
        )
        .await
        .expect("list_active");

        assert_eq!(sensors.len(), 1);
        assert_eq!(sensors[0].daq_id, "daq:1");
    }

    // -----------------------------------------------------------------------
    // Test: list_active_empty_when_none
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_active_empty_when_none() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let sensors = list_active(
            bldg,
            list_sensor_ids_fn(store.clone()),
            get_active_fn(store.clone()),
        )
        .await
        .expect("list_active");

        assert_eq!(sensors.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test: get_active_happy
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_active_happy() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s = do_attach(store.clone(), bldg, "daq:k", Some(15))
            .await
            .expect("attach");

        let s2 = get_active(s.id, {
            let store = store.clone();
            move |sid| store.get_active_sensor(&sid)
        })
        .expect("get_active");

        assert_eq!(s2.daq_id, "daq:k");
    }

    // -----------------------------------------------------------------------
    // Test: get_active_unknown
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_active_unknown() {
        let store = Rc::new(Store::new());
        let id = SensorId::make(999999);

        let result = get_active(id, {
            let s = store.clone();
            move |sid| s.get_active_sensor(&sid)
        });

        match result {
            Err(RepositoryError::NotFound(_)) => {} // expected
            Ok(_) => panic!("expected not found"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: replace_device_promotes_new
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn replace_device_promotes_new() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s = do_attach(store.clone(), bldg, "daq:old", Some(15))
            .await
            .expect("attach");

        let s2 = replace_device(
            s.id,
            "daq:new".to_string(),
            {
                let st = store.clone();
                move |sid| st.get_active_sensor(&sid)
            },
            {
                let st = store.clone();
                move |daq: &str| st.find_active_by_daq(daq)
            },
            replace_sensor_fn(store.clone()),
        )
        .await
        .expect("replace_device");

        assert_eq!(s2.daq_id, "daq:new");

        let cur = store.get_active_sensor(&s.id).expect("active after replace");
        assert_eq!(cur.daq_id, "daq:new");
        // created timestamp should have advanced (not equal to original)
        assert_ne!(cur.created, s.created, "created timestamp updated");
    }

    // -----------------------------------------------------------------------
    // Test: replace_unknown_fails
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn replace_unknown_fails() {
        let store = Rc::new(Store::new());
        let id = SensorId::make(999998);

        let result = replace_device(
            id,
            "x".to_string(),
            {
                let s = store.clone();
                move |sid| s.get_active_sensor(&sid)
            },
            {
                let s = store.clone();
                move |daq: &str| s.find_active_by_daq(daq)
            },
            replace_sensor_fn(store.clone()),
        )
        .await;

        match result {
            Err(RepositoryError::NotFound(_)) => {} // expected
            Ok(_) => panic!("expected Not_found"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: set_formula_detects_self_cycle
    //
    // Attach a sensor normally, then call set_formula with a formula that
    // references its own id (cycle).

    // -----------------------------------------------------------------------
    // Test: attach_rolls_back_on_post_allocation_self_cycle
    //
    // attach can only detect a self-cycle AFTER it allocates the sensor's id
    // (the formula references that not-yet-known id). This exercises the
    // post-allocation rollback branch: the allocated sensor must be deleted.

    // -----------------------------------------------------------------------
    // evaluate tests

    // -----------------------------------------------------------------------
    // Test: evaluate_identity

    // -----------------------------------------------------------------------
    // Test: evaluate_composite

    // -----------------------------------------------------------------------
    // Test: evaluate_zero_short_circuits_reading

    // -----------------------------------------------------------------------
    // Test: list_under_company_scopes_to_hn2
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_under_company_scopes_to_hn2() {
        let store = Rc::new(Store::new());

        // Seed two sensors directly — one in HN2#200, one in HN2#999.
        let mk_sensor = |id: u32, path: &str| -> Sensor {
            Sensor::builder()
                .id(SensorId::make(id))
                .created(ts())
                .daq_id(format!("d{}", id))
                .path(path.to_string())
                .energy_type(EnergyType::Electricity)
                .reading_kind(ReadingKind::Counter)
                .build()
        };

        let s1 = mk_sensor(1, "HN0#root|HN1#10|HN2#200|HN3#1|S#1");
        let s2 = mk_sensor(2, "HN0#root|HN1#10|HN2#999|HN3#9|S#2");

        // Insert sensors directly using add_sensor.
        store.add_sensor(|_raw_id| {
            let e = TestRepoEdgeSpec {
                from_: "HN0#root".to_string(),
                to_: s1.id.to_string(),
                kind: EdgeKind::HasSensor,
                name: String::new(),
            };
            (s1.clone(), e)
        });
        store.add_sensor(|_raw_id| {
            let e = TestRepoEdgeSpec {
                from_: "HN0#root".to_string(),
                to_: s2.id.to_string(),
                kind: EdgeKind::HasSensor,
                name: String::new(),
            };
            (s2.clone(), e)
        });

        // Seed parent node HN3#1 with path "HN0#root|HN1#10|HN2#200".
        let parent = NodeId::make(Level::Hn3, 1);
        let parent_node = node::make(
            1,
            Level::Hn3,
            "b",
            NodeId::make(Level::Hn2, 200),
            "HN0#root|HN1#10|HN2#200",
            serde_json::json!({}),
            None,
        );
        store.put_node(&parent_node);

        let got = list_under_company(
            parent,
            get_node_fn(store.clone()),
            list_sensors_under_path_fn(store.clone()),
        )
        .await
        .expect("list_under_company");

        assert_eq!(got.len(), 1, "only company HN2#200 sensors");
        assert!(
            got.iter().all(|s| s.path.starts_with("HN0#root|HN1#10|HN2#200|")),
            "all results under HN2#200"
        );
    }
}
