//! Sensors logic — ported 1:1 from `services/hierarchy/lib/logic/sensors.ml`.
//!
//! All repository operations are injected as async closures; no traits are used.

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::domain::formula::Formula;
use crate::domain::ids::{NodeId, SensorId};
use crate::domain::node::Node;
use crate::domain::sensor::{self, Sensor};
use crate::domain::values::{EdgeKind, MeterType};
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

/// Construct a synthetic `NotFound` for a sensor (mirrors OCaml `sensor_not_found`).
fn sensor_not_found(id: SensorId) -> RepositoryError {
    RepositoryError::NotFound(NodeId::make(crate::domain::ids::Level::Hn9, id.id()))
}

// ---------------------------------------------------------------------------
// Cycle detection
// ---------------------------------------------------------------------------

enum WalkResult {
    Ok,
    Cycle,
}

fn walk_refs_sync(
    visited: &[SensorId],
    id: SensorId,
    get_active: &dyn Fn(SensorId) -> Option<Sensor>,
) -> WalkResult {
    if visited.contains(&id) {
        return WalkResult::Cycle;
    }
    match get_active(id) {
        None => WalkResult::Ok,
        Some(s) => {
            let next = s.formula.referenced_ids();
            let mut new_visited = visited.to_vec();
            new_visited.push(id);
            let mut result = WalkResult::Ok;
            for u in next {
                match walk_refs_sync(&new_visited, u, get_active) {
                    WalkResult::Cycle => {
                        result = WalkResult::Cycle;
                        break;
                    }
                    WalkResult::Ok => {}
                }
            }
            result
        }
    }
}

/// Check whether `formula` introduces a cycle, given `self_id` is the sensor
/// whose formula is being set.
///
/// Mirrors OCaml `has_cycle`.
fn has_cycle(
    self_id: SensorId,
    formula: &Formula,
    get_active: &dyn Fn(SensorId) -> Option<Sensor>,
) -> bool {
    let next = formula.referenced_ids();
    next.iter().any(|u| {
        *u == self_id || matches!(walk_refs_sync(&[self_id], *u, get_active), WalkResult::Cycle)
    })
}

// ---------------------------------------------------------------------------
// attach
// ---------------------------------------------------------------------------

/// Attach a new sensor to `parent`.
///
/// Mirrors OCaml `Sensors.attach`.
#[allow(clippy::too_many_arguments)]
pub async fn attach<FGN, FGNFut, FAS, FASFut, FGA, FDS, FDSFut>(
    parent: NodeId,
    daq_id: String,
    purpose: String,
    meter_type: MeterType,
    unit: Option<String>,
    formula: Formula,
    resample_minutes: Option<i32>,
    get_node: FGN,
    add_sensor: FAS,
    get_active_sensor: FGA,
    delete_sensor: FDS,
) -> Result<Sensor, RepositoryError>
where
    FGN: Fn(NodeId) -> FGNFut,
    FGNFut: Future<Output = Result<Option<Node>, RepositoryError>>,
    FAS: FnOnce(Box<dyn FnOnce(u32) -> (Sensor, RepoEdgeSpec) + Send>) -> FASFut,
    FASFut: Future<Output = Result<Sensor, RepositoryError>>,
    FGA: Fn(SensorId) -> Option<Sensor> + Clone + 'static,
    FDS: FnOnce(SensorId, NodeId) -> FDSFut,
    FDSFut: Future<Output = Result<(), RepositoryError>>,
{
    // Validate parent exists.
    let parent_node = get_node(parent.clone())
        .await?
        .ok_or_else(|| RepositoryError::NotFound(parent.clone()))?;

    // Validate resample_minutes if set.
    if let Some(b) = resample_minutes {
        if b <= 0 {
            return Err(validation_err("resample_minutes must be > 0 minutes"));
        }
    }

    // Check schema allows sensors at this level.
    let parent_level = parent_node.level();
    let (_host, schema) = schema_check::find_for(parent.clone(), &get_node).await?;
    if !schema.allows_sensors(parent_level) {
        return Err(validation_err(format!(
            "sensors not allowed at {}",
            parent_level
        )));
    }

    // Capture state for the build closure.
    let parent_path = parent_node.path.clone();
    let parent_clone = parent.clone();
    let formula_clone = formula.clone();
    let unit_clone = unit.clone();

    // Allocate sensor and write edge atomically.
    let s = add_sensor(Box::new(move |raw_id| {
        let sid = SensorId::make(raw_id);
        let path = sensor::child_path(&parent_path, &sid.to_string());
        let sensor = Sensor {
            id: sid,
            created: chrono::Utc::now(),
            daq_id,
            path,
            purpose,
            meter_type,
            unit: unit_clone,
            formula: formula_clone,
            resample_minutes,
        };
        let edge = RepoEdgeSpec {
            from_: parent_clone.to_string(),
            to_: sensor.id.to_string(),
            kind: EdgeKind::HasSensor,
            name: String::new(),
        };
        (sensor, edge)
    }))
    .await?;

    // Post-allocation cycle check (mirrors OCaml: check AFTER allocation, roll
    // back via delete_sensor if cycle detected).
    let sensor_id = s.id;
    let formula_for_check = formula.clone();
    let ga = get_active_sensor.clone();
    if has_cycle(sensor_id, &formula_for_check, &move |id| ga(id)) {
        delete_sensor(sensor_id, parent.clone()).await?;
        return Err(validation_err("formula refs form a cycle"));
    }

    Ok(s)
}

// ---------------------------------------------------------------------------
// list_active
// ---------------------------------------------------------------------------

/// List all active sensors attached to `parent`.
///
/// Mirrors OCaml `Sensors.list_active`.
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
///
/// Mirrors OCaml `Sensors.get_active`.
pub fn get_active<FGA>(id: SensorId, get_active_sensor: FGA) -> Result<Sensor, RepositoryError>
where
    FGA: FnOnce(SensorId) -> Option<Sensor>,
{
    get_active_sensor(id)
        .ok_or_else(|| sensor_not_found(id))
}

// ---------------------------------------------------------------------------
// replace_device
// ---------------------------------------------------------------------------

/// Replace the daq device on a sensor (bumps `created`).
///
/// Mirrors OCaml `Sensors.replace_device`.
pub async fn replace_device<FGA, FRD, FRDFut>(
    sensor_id: SensorId,
    new_daq_id: String,
    get_active_sensor: FGA,
    replace_sensor_device: FRD,
) -> Result<Sensor, RepositoryError>
where
    FGA: FnOnce(SensorId) -> Option<Sensor>,
    FRD: FnOnce(DateTime<Utc>, Sensor) -> FRDFut,
    FRDFut: Future<Output = Result<(), RepositoryError>>,
{
    let old = get_active_sensor(sensor_id).ok_or_else(|| sensor_not_found(sensor_id))?;
    let old_created = old.created;
    let new_sensor = Sensor {
        created: chrono::Utc::now(),
        daq_id: new_daq_id,
        ..old
    };
    replace_sensor_device(old_created, new_sensor.clone()).await?;
    Ok(new_sensor)
}

// ---------------------------------------------------------------------------
// set_formula
// ---------------------------------------------------------------------------

/// Update the formula on an existing sensor.
///
/// Mirrors OCaml `Sensors.set_formula`.
pub async fn set_formula<FGA, FRD, FRDFut>(
    sensor_id: SensorId,
    formula: Formula,
    get_active_sensor: FGA,
    replace_sensor_device: FRD,
) -> Result<Sensor, RepositoryError>
where
    FGA: Fn(SensorId) -> Option<Sensor> + Clone,
    FRD: FnOnce(DateTime<Utc>, Sensor) -> FRDFut,
    FRDFut: Future<Output = Result<(), RepositoryError>>,
{
    let old = get_active_sensor(sensor_id).ok_or_else(|| sensor_not_found(sensor_id))?;

    // Cycle check before write.
    let ga = get_active_sensor.clone();
    if has_cycle(sensor_id, &formula, &move |id| ga(id)) {
        return Err(validation_err("formula refs form a cycle"));
    }

    let old_created = old.created;
    let new_sensor = Sensor {
        created: chrono::Utc::now(),
        formula,
        ..old
    };
    replace_sensor_device(old_created, new_sensor.clone()).await?;
    Ok(new_sensor)
}

// ---------------------------------------------------------------------------
// evaluate
// ---------------------------------------------------------------------------

/// Evaluate the formula of a sensor, resolving referenced sensors recursively.
///
/// Mirrors OCaml `Sensors.evaluate`.
pub fn evaluate<FGA, FGR>(
    id: SensorId,
    get_active_sensor: &FGA,
    get_sensor_reading: &FGR,
) -> Result<f64, RepositoryError>
where
    FGA: Fn(SensorId) -> Option<Sensor>,
    FGR: Fn(SensorId) -> Option<f64>,
{
    let s = get_active_sensor(id).ok_or_else(|| sensor_not_found(id))?;
    let reading = || {
        get_sensor_reading(id).ok_or_else(|| sensor_not_found(id))
    };

    match &s.formula {
        Formula::Zero => Ok(0.0),
        Formula::Identity => reading(),
        Formula::Expr { refs, .. } => {
            let self_reading = reading()?;
            // Resolve all referenced sensors.
            let mut resolved: Vec<(String, f64)> = Vec::new();
            for (alias, sid) in refs {
                let v = evaluate(*sid, get_active_sensor, get_sensor_reading)?;
                resolved.push((alias.clone(), v));
            }
            let resolve_alias = |alias: &str| -> f64 {
                resolved
                    .iter()
                    .find(|(a, _)| a == alias)
                    .map(|(_, v)| *v)
                    .unwrap_or_else(|| panic!("Unknown_ref: {}", alias))
            };
            Ok(s.formula.eval(self_reading, &resolve_alias))
        }
    }
}

// ---------------------------------------------------------------------------
// list_under_company
// ---------------------------------------------------------------------------

/// List all active sensors in the same HN2 company subtree as `parent`.
///
/// Mirrors OCaml `Sensors.list_under_company`.
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

/// Extract the path prefix up to and including the HN2 (company) segment,
/// with a trailing `|`.
///
/// Mirrors OCaml `company_prefix_of_path`.
fn company_prefix_of_path(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split('|').filter(|s| !s.is_empty()).collect();
    let mut acc: Vec<&str> = Vec::new();
    for seg in &segs {
        acc.push(seg);
        if let Ok(nid) = NodeId::parse(seg) {
            if nid.level() == crate::domain::ids::Level::Hn2 {
                // Build prefix with trailing |
                let mut p = acc.join("|");
                p.push('|');
                return Some(p);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests — port of `services/hierarchy/test/test_logic_sensors.ml`
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use chrono::{DateTime, Utc};

    use super::*;
    use crate::domain::formula::{Expr, Formula};
    use crate::domain::ids::{Level, NodeId, SensorId};
    use crate::domain::node;
    use crate::domain::schema::{EdgeSpec as SchemaEdgeSpec, Schema};
    use crate::repository::EdgeSpec as TestRepoEdgeSpec;
    use crate::domain::values::{EdgeKind, MeterType};
    use crate::errors::RepositoryError;
    use crate::logic::hierarchy;
    use crate::repository::memory::Store;

    fn ts() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn sample_schema() -> Schema {
        Schema {
            version: 1,
            edges: vec![(
                Level::Hn2,
                vec![(
                    Level::Hn3,
                    vec![SchemaEdgeSpec::builder()
                        .label("building".to_string())
                        .build()],
                )],
            )],
            metadata: vec![],
            sensors: vec![Level::Hn3],
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
        let n2 = node::make(
            10002,
            Level::Hn2,
            "Acme",
            NodeId::root(),
            &parent_path_for_hn2(),
            ts(),
            serde_json::json!({}),
            Some(sample_schema()),
        );
        store.put_node(&n2);
        c2
    }

    async fn seed_building(store: &Rc<Store>, c2: NodeId) -> NodeId {
        let n = hierarchy::add_node(
            c2,
            Some(Level::Hn3),
            None,
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
                move |level, build| {
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
    ) -> impl FnOnce(Box<dyn FnOnce(u32) -> (Sensor, TestRepoEdgeSpec) + Send>) -> std::future::Ready<Result<Sensor, RepositoryError>>
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
        formula: Formula,
        resample_minutes: Option<i32>,
    ) -> Result<Sensor, RepositoryError> {
        attach(
            parent,
            daq_id.to_string(),
            "Electricity".to_string(),
            MeterType::Counter,
            Some("kWh".to_string()),
            formula,
            resample_minutes,
            get_node_fn(store.clone()),
            add_sensor_fn(store.clone()),
            get_active_fn(store.clone()),
            delete_sensor_fn(store.clone()),
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Test: attach_happy  (OCaml: `attach_happy`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn attach_happy() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s = do_attach(store.clone(), bldg.clone(), "daq:1", Formula::Identity, Some(15))
            .await
            .expect("attach should succeed");

        assert_eq!(s.daq_id, "daq:1");
        assert_eq!(s.parent_id().to_string(), bldg.to_string());
    }

    // -----------------------------------------------------------------------
    // Test: rejects_level_not_allowed  (OCaml: `rejects_level_not_allowed`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_level_not_allowed() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);

        // Sensors allowed at Hn3 but not Hn2 in sample_schema.
        let result = do_attach(store.clone(), c2, "daq:2", Formula::Identity, Some(15)).await;

        match result {
            Err(RepositoryError::Validation(_)) => {} // expected
            Ok(_) => panic!("expected Validation at disallowed level"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // Test: list_active_returns_attached  (OCaml: `list_active_returns_attached`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_active_returns_attached() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        do_attach(store.clone(), bldg.clone(), "daq:1", Formula::Identity, Some(15))
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
    // Test: list_active_empty_when_none  (OCaml: `list_active_empty_when_none`)
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
    // Test: get_active_happy  (OCaml: `get_active_happy`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_active_happy() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s = do_attach(store.clone(), bldg, "daq:k", Formula::Identity, Some(15))
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
    // Test: get_active_unknown  (OCaml: `get_active_unknown`)
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
    // Test: replace_device_promotes_new  (OCaml: `replace_device_promotes_new`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn replace_device_promotes_new() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s = do_attach(store.clone(), bldg, "daq:old", Formula::Identity, Some(15))
            .await
            .expect("attach");

        let s2 = replace_device(
            s.id,
            "daq:new".to_string(),
            {
                let st = store.clone();
                move |sid| st.get_active_sensor(&sid)
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
    // Test: replace_unknown_fails  (OCaml: `replace_unknown_fails`)
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
    // Test: attach_detects_self_cycle  (OCaml: `attach_detects_self_cycle`)
    //
    // Attach a sensor normally, then call set_formula with a formula that
    // references its own id (cycle).
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn attach_detects_self_cycle() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s = do_attach(store.clone(), bldg, "daq:1", Formula::Identity, Some(15))
            .await
            .expect("attach");

        let cyc = Formula::Expr {
            refs: vec![("self_again".to_string(), s.id)],
            expr: Expr::Ref("self_again".to_string()),
        };

        let result = set_formula(
            s.id,
            cyc,
            get_active_fn(store.clone()),
            replace_sensor_fn(store.clone()),
        )
        .await;

        match result {
            Err(RepositoryError::Validation(_)) => {} // expected
            Ok(_) => panic!("should reject cycle"),
            Err(e) => panic!("wrong error: {:?}", e),
        }
    }

    // -----------------------------------------------------------------------
    // evaluate tests
    // -----------------------------------------------------------------------

    /// Evaluate with a fixed reading map.
    fn eval_with_readings(
        store: &Rc<Store>,
        id: SensorId,
        readings: &[(SensorId, f64)],
    ) -> Result<f64, RepositoryError> {
        let ga = {
            let s = store.clone();
            move |sid| s.get_active_sensor(&sid)
        };
        let gr = |sid: SensorId| {
            readings
                .iter()
                .find(|(s, _)| *s == sid)
                .map(|(_, v)| *v)
        };
        evaluate(id, &ga, &gr)
    }

    // -----------------------------------------------------------------------
    // Test: evaluate_identity  (OCaml: `evaluate_identity`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn evaluate_identity() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s = do_attach(store.clone(), bldg, "daq:1", Formula::Identity, Some(15))
            .await
            .expect("attach");

        let readings = [(s.id, 42.0)];
        let v = eval_with_readings(&store, s.id, &readings).expect("evaluate");
        assert!((v - 42.0).abs() < 1e-9, "raw passed through");
    }

    // -----------------------------------------------------------------------
    // Test: evaluate_composite  (OCaml: `evaluate_composite`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn evaluate_composite() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s4 = do_attach(store.clone(), bldg.clone(), "daq:4", Formula::Identity, Some(15))
            .await
            .expect("attach s4");

        let formula_s3 = Formula::Expr {
            refs: vec![("s4".to_string(), s4.id)],
            expr: Expr::Abs(Box::new(Expr::Sub(
                Box::new(Expr::SelfRef),
                Box::new(Expr::Ref("s4".to_string())),
            ))),
        };

        let s3 = do_attach(store.clone(), bldg, "daq:3", formula_s3, Some(15))
            .await
            .expect("attach s3");

        let readings = [(s4.id, 3.0), (s3.id, 10.0)];
        let v = eval_with_readings(&store, s3.id, &readings).expect("evaluate");
        assert!((v - 7.0).abs() < 1e-9, "|10 - 3| = 7, got {}", v);
    }

    // -----------------------------------------------------------------------
    // Test: evaluate_zero_short_circuits_reading
    //       (OCaml: `evaluate_zero_short_circuits_reading`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn evaluate_zero_short_circuits_reading() {
        let store = Rc::new(Store::new());
        let c2 = seed_company(&store);
        let bldg = seed_building(&store, c2).await;

        let s = do_attach(store.clone(), bldg, "daq:z", Formula::Zero, Some(15))
            .await
            .expect("attach");

        // No readings provided — Zero must not try to fetch one.
        let readings: &[(SensorId, f64)] = &[];
        let v = eval_with_readings(&store, s.id, readings).expect("evaluate");
        assert_eq!(v, 0.0);
    }

    // -----------------------------------------------------------------------
    // Test: list_under_company_scopes_to_hn2
    //       (OCaml: `list_under_company_scopes_to_hn2`)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_under_company_scopes_to_hn2() {
        let store = Rc::new(Store::new());

        // Seed two sensors directly — one in HN2#200, one in HN2#999.
        let mk_sensor = |id: u32, path: &str| -> Sensor {
            Sensor {
                id: SensorId::make(id),
                created: ts(),
                daq_id: format!("d{}", id),
                path: path.to_string(),
                purpose: "E".to_string(),
                meter_type: MeterType::Counter,
                unit: None,
                formula: Formula::Identity,
                resample_minutes: None,
            }
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
            ts(),
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
