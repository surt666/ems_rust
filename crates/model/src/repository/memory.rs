//! In-memory store — the test substrate that mirrors the DynamoDB hierarchy
//! table and exposes the same operations as the real repository.
//!
//! Enabled by the `testing` cargo feature (or inside `#[cfg(test)]`).
//! All methods are synchronous; logic-layer tests wrap them in
//! `|x| async move { Ok(store.get_node(&x)) }` closures.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{DateTime, Utc};

use crate::domain::ids::{Level, NodeId, SensorId, UserId};
use crate::domain::node::Node;
use crate::domain::node_formula::NodeFormula;
use crate::domain::sensor::Sensor;
use crate::domain::user::User;
use crate::domain::values::{EdgeKind, EnergyType, Purpose};
use crate::logic::formulas::MatrixRow;

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A sensor can be active or historical (replaced device).
#[derive(Clone, Debug)]
enum SensorRow {
    Active(Sensor),
    // The inner sensor is kept for potential future history queries;
    // the field is intentionally retained even though it is not read today.
    #[allow(dead_code)]
    History(Sensor),
}

/// A directed edge between two named entities.
#[derive(Clone, Debug)]
struct Edge {
    from_: String,
    to_: String,
    kind: EdgeKind,
    name: String,
}

/// A counter used for id allocation (monotonic `n`) and cardinality (`live`).
#[derive(Clone, Debug)]
struct Counter {
    n: u32,
    live: u32,
}

impl Counter {
    const fn new() -> Self {
        // Starting value: `n = 10_000`.
        Counter { n: 10_000, live: 0 }
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Shared mutable state; wrap in `Rc<RefCell<…>>` for closure capture.
struct StoreInner {
    nodes: std::collections::HashMap<String, Node>,
    edges: Vec<Edge>,
    sensors: std::collections::HashMap<String, Vec<SensorRow>>,
    users: std::collections::HashMap<String, User>,
    /// Node formulas, keyed by `(node, energy_type, purpose)`.
    formulas: Vec<NodeFormula>,
    /// The materialised coefficient matrix, keyed by company path.
    matrix: std::collections::HashMap<String, Vec<MatrixRow>>,
    counters: std::collections::HashMap<String, Counter>,
    clock: Box<dyn Fn() -> DateTime<Utc>>,
}

/// An in-memory store that mirrors the hierarchy DynamoDB table.
///
/// Clone is cheap — it is just an `Rc` bump.
#[derive(Clone)]
pub struct Store {
    inner: Rc<RefCell<StoreInner>>,
}

impl Store {
    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

    /// Create an empty store.  The default clock reads the real wall-clock.
    pub fn new() -> Self {
        Store {
            inner: Rc::new(RefCell::new(StoreInner {
                nodes: Default::default(),
                edges: Vec::new(),
                sensors: Default::default(),
                users: Default::default(),
                formulas: Vec::new(),
                matrix: Default::default(),
                counters: Default::default(),
                clock: Box::new(Utc::now),
            })),
        }
    }

    /// Create an empty store with a fixed clock (useful for deterministic
    /// tests that inspect `created` timestamps).
    pub fn with_clock(clock: impl Fn() -> DateTime<Utc> + 'static) -> Self {
        Store {
            inner: Rc::new(RefCell::new(StoreInner {
                nodes: Default::default(),
                edges: Vec::new(),
                sensors: Default::default(),
                users: Default::default(),
                formulas: Vec::new(),
                matrix: Default::default(),
                counters: Default::default(),
                clock: Box::new(clock),
            })),
        }
    }

    // -----------------------------------------------------------------------
    // Clock seam
    // -----------------------------------------------------------------------

    pub fn now(&self) -> DateTime<Utc> {
        (self.inner.borrow().clock)()
    }

    // -----------------------------------------------------------------------
    // Counter / id allocation
    // -----------------------------------------------------------------------

    fn counter_pk_node(level: Level) -> String {
        format!("count#HN{}", level.depth())
    }

    const COUNTER_PK_SENSOR: &'static str = "count#S";

    /// Allocate the next node id for `level`; returns the raw integer.
    pub fn allocate_node_id(&self, level: Level) -> u32 {
        let key = Self::counter_pk_node(level);
        let mut inner = self.inner.borrow_mut();
        let c = inner.counters.entry(key).or_insert_with(Counter::new);
        c.n += 1;
        c.live += 1;
        c.n
    }

    /// Allocate the next sensor id; returns the raw integer.
    pub fn allocate_sensor_id(&self) -> u32 {
        let mut inner = self.inner.borrow_mut();
        let c = inner
            .counters
            .entry(Self::COUNTER_PK_SENSOR.to_owned())
            .or_insert_with(Counter::new);
        c.n += 1;
        c.live += 1;
        c.n
    }

    fn release_node_count(inner: &mut StoreInner, level: Level) {
        let key = Self::counter_pk_node(level);
        if let Some(c) = inner.counters.get_mut(&key) {
            c.live = c.live.saturating_sub(1);
        }
    }

    fn release_sensor_count(inner: &mut StoreInner) {
        if let Some(c) = inner.counters.get_mut(Self::COUNTER_PK_SENSOR) {
            c.live = c.live.saturating_sub(1);
        }
    }

    // -----------------------------------------------------------------------
    // Node operations
    // -----------------------------------------------------------------------

    /// Fetch a node by id.  Returns `None` if absent.
    pub fn get_node(&self, id: &NodeId) -> Option<Node> {
        self.inner.borrow().nodes.get(&id.to_string()).cloned()
    }

    /// Insert or replace a node.
    pub fn put_node(&self, node: &Node) {
        self.inner
            .borrow_mut()
            .nodes
            .insert(node.id.to_string(), node.clone());
    }

    /// Delete a node by id, remove all edges that touch it, and decrement
    /// the live counter.  No-op if the node is absent.
    pub fn delete_node(&self, id: &NodeId) {
        let id_s = id.to_string();
        let mut inner = self.inner.borrow_mut();
        if let Some(n) = inner.nodes.remove(&id_s) {
            Self::release_node_count(&mut inner, n.level());
        }
        inner
            .edges
            .retain(|e| e.from_ != id_s && e.to_ != id_s);
    }

    /// Allocate a new node id, call `build(id)` to get `(Node, edge_spec)`,
    /// store both, and return the node.
    ///
    /// `build` receives the raw integer allocated for the node.
    /// Accepts `Fn` so the same closure can be reused across retries.
    pub fn add_node<F>(&self, level: Level, build: F) -> Node
    where
        F: Fn(u32) -> (Node, EdgeSpec),
    {
        let id = self.allocate_node_id(level);
        let (node, edge_spec) = build(id);
        {
            let mut inner = self.inner.borrow_mut();
            inner.nodes.insert(node.id.to_string(), node.clone());
            inner.edges.push(Edge {
                from_: edge_spec.from_,
                to_: edge_spec.to_,
                kind: edge_spec.kind,
                name: edge_spec.name,
            });
        }
        node
    }

    // -----------------------------------------------------------------------
    // Edge operations
    // -----------------------------------------------------------------------

    /// Append an edge.
    pub fn put_edge(&self, spec: EdgeSpec) {
        self.inner.borrow_mut().edges.push(Edge {
            from_: spec.from_,
            to_: spec.to_,
            kind: spec.kind,
            name: spec.name,
        });
    }

    /// Remove edges matching all three of `(from_, to_, kind)`.
    pub fn delete_edge(&self, from_: &str, to_: &str, kind: &EdgeKind) {
        self.inner
            .borrow_mut()
            .edges
            .retain(|e| !(e.from_ == from_ && e.to_ == to_ && &e.kind == kind));
    }

    /// List child *nodes* reachable via edges from `parent` (optionally
    /// filtered by `kind_opt`).  Missing nodes are silently skipped.
    pub fn list_children(&self, parent: &NodeId, kind_opt: Option<&EdgeKind>) -> Vec<Node> {
        let inner = self.inner.borrow();
        let parent_s = parent.to_string();
        inner
            .edges
            .iter()
            .filter(|e| e.from_ == parent_s)
            .filter(|e| kind_opt.map_or(true, |k| &e.kind == k))
            .filter_map(|e| inner.nodes.get(&e.to_).cloned())
            .collect()
    }

    /// List `(child_node_id, edge_name)` pairs for all edges from `parent`
    /// (optionally filtered by `kind_opt`).
    pub fn list_child_refs(
        &self,
        parent: &NodeId,
        kind_opt: Option<&EdgeKind>,
    ) -> Vec<(NodeId, String)> {
        let inner = self.inner.borrow();
        let parent_s = parent.to_string();
        inner
            .edges
            .iter()
            .filter(|e| e.from_ == parent_s)
            .filter(|e| kind_opt.map_or(true, |k| &e.kind == k))
            .filter_map(|e| {
                NodeId::parse(&e.to_)
                    .ok()
                    .map(|child_id| (child_id, e.name.clone()))
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Sensor operations
    // -----------------------------------------------------------------------

    /// Allocate a new sensor id, call `build(id)` to get `(Sensor, edge_spec)`,
    /// store both, and return the sensor.
    ///
    /// Accepts `Fn` so the same closure can be reused across retries.
    pub fn add_sensor<F>(&self, build: F) -> Sensor
    where
        F: Fn(u32) -> (Sensor, EdgeSpec),
    {
        let id = self.allocate_sensor_id();
        let (sensor, edge_spec) = build(id);
        let mut inner = self.inner.borrow_mut();
        inner
            .sensors
            .entry(sensor.id.to_string())
            .or_default()
            .insert(0, SensorRow::Active(sensor.clone()));
        inner.edges.push(Edge {
            from_: edge_spec.from_,
            to_: edge_spec.to_,
            kind: edge_spec.kind,
            name: edge_spec.name,
        });
        sensor
    }

    /// Fetch the active sensor for `id`.  Returns `None` if absent or only
    /// history rows exist.
    pub fn get_active_sensor(&self, id: &SensorId) -> Option<Sensor> {
        let inner = self.inner.borrow();
        inner
            .sensors
            .get(&id.to_string())
            .and_then(|rows| Self::active_of_rows(rows))
    }

    fn active_of_rows(rows: &[SensorRow]) -> Option<Sensor> {
        rows.iter().find_map(|r| match r {
            SensorRow::Active(s) => Some(s.clone()),
            SensorRow::History(_) => None,
        })
    }

    /// The single ACTIVE sensor (logical) currently fed by `daq_id`, if any —
    /// backs the "one active logical per physical device" rule in `logic::sensors`.
    pub fn find_active_by_daq(&self, daq_id: &str) -> Option<Sensor> {
        let inner = self.inner.borrow();
        inner
            .sensors
            .values()
            .find_map(|rows| Self::active_of_rows(rows).filter(|s| s.daq_id == daq_id))
    }

    /// List all sensor ids directly attached to `parent` via `Has_sensor`
    /// edges.
    pub fn list_sensor_ids(&self, parent: &NodeId) -> Vec<SensorId> {
        let inner = self.inner.borrow();
        let parent_s = parent.to_string();
        inner
            .edges
            .iter()
            .filter(|e| e.from_ == parent_s && e.kind == EdgeKind::HasSensor)
            .filter_map(|e| SensorId::parse(&e.to_).ok())
            .collect()
    }

    /// Demote the currently active sensor row whose `created` matches
    /// `old_created` to `History`, then prepend `new_sensor` as `Active`.
    pub fn replace_sensor_device(
        &self,
        old_created: DateTime<Utc>,
        new_sensor: &Sensor,
    ) {
        let mut inner = self.inner.borrow_mut();
        let rows = inner
            .sensors
            .entry(new_sensor.id.to_string())
            .or_default();
        // Demote the matching active row.
        for row in rows.iter_mut() {
            if let SensorRow::Active(s) = row {
                if s.created == old_created {
                    *row = SensorRow::History(s.clone());
                }
            }
        }
        rows.insert(0, SensorRow::Active(new_sensor.clone()));
    }

    /// Delete a sensor: remove its row map, the `Has_sensor` edge from
    /// `parent`, and decrement the sensor counter.
    pub fn delete_sensor(&self, sensor_id: &SensorId, parent: &NodeId) {
        let sensor_s = sensor_id.to_string();
        let parent_s = parent.to_string();
        let mut inner = self.inner.borrow_mut();
        Self::release_sensor_count(&mut inner);
        inner.sensors.remove(&sensor_s);
        inner.edges.retain(|e| {
            !(e.from_ == parent_s
                && e.kind == EdgeKind::HasSensor
                && e.to_ == sensor_s)
        });
    }

    /// Always returns `None` — no reading data in the in-memory store.
    #[allow(unused_variables)]
    pub const fn get_sensor_reading(&self, id: &SensorId) -> Option<()> {
        None
    }

    /// List all active sensors whose `path` starts with `prefix`.
    pub fn list_sensors_under_path(&self, prefix: &str) -> Vec<Sensor> {
        let inner = self.inner.borrow();
        inner
            .sensors
            .values()
            .filter_map(|rows| Self::active_of_rows(rows))
            .filter(|s| s.path.starts_with(prefix))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Node formulas
    // -----------------------------------------------------------------------

    /// Upsert on `(node, energy_type, purpose)`.
    pub fn put_node_formula(&self, f: &NodeFormula) {
        let mut inner = self.inner.borrow_mut();
        inner.formulas.retain(|o| {
            !(o.node == f.node && o.energy_type == f.energy_type && o.purpose == f.purpose)
        });
        inner.formulas.push(f.clone());
    }

    pub fn delete_node_formula(&self, node: &NodeId, et: EnergyType, purpose: Purpose) {
        self.inner
            .borrow_mut()
            .formulas
            .retain(|o| !(&o.node == node && o.energy_type == et && o.purpose == purpose));
    }

    pub fn list_node_formulas(&self, node: &NodeId) -> Vec<NodeFormula> {
        self.inner
            .borrow()
            .formulas
            .iter()
            .filter(|f| &f.node == node)
            .cloned()
            .collect()
    }

    /// Every formula whose node lies under `company_path`.
    pub fn list_company_formulas(&self, company_path: &str) -> Vec<NodeFormula> {
        let inner = self.inner.borrow();
        let under = |id: &NodeId| {
            inner
                .nodes
                .get(&id.to_string())
                .is_some_and(|n| n.path.starts_with(company_path))
        };
        inner.formulas.iter().filter(|f| under(&f.node)).cloned().collect()
    }

    /// Every node under `company_path`, including the company itself.
    pub fn list_company_nodes(&self, company_path: &str) -> Vec<Node> {
        self.inner
            .borrow()
            .nodes
            .values()
            .filter(|n| n.path.starts_with(company_path))
            .cloned()
            .collect()
    }

    // -----------------------------------------------------------------------
    // Materialised coefficient matrix
    // -----------------------------------------------------------------------

    /// Replace a company's matrix wholesale — it is derived data.
    pub fn replace_company_matrix(&self, company_path: &str, rows: Vec<MatrixRow>) {
        self.inner
            .borrow_mut()
            .matrix
            .insert(company_path.to_string(), rows);
    }

    pub fn company_matrix(&self, company_path: &str) -> Vec<MatrixRow> {
        self.inner
            .borrow()
            .matrix
            .get(company_path)
            .cloned()
            .unwrap_or_default()
    }

    /// Every matrix row in the store, whatever company it belongs to — the
    /// convenient shape for asserting "a recompute happened".
    pub fn weight_rows(&self) -> Vec<MatrixRow> {
        self.inner.borrow().matrix.values().flatten().cloned().collect()
    }

    // -----------------------------------------------------------------------
    // User operations
    // -----------------------------------------------------------------------

    /// Insert or replace a user.
    pub fn put_user(&self, user: &User) {
        self.inner
            .borrow_mut()
            .users
            .insert(user.id.to_string(), user.clone());
    }

    /// Fetch a user by id.  Returns `None` if absent.
    pub fn get_user(&self, id: &UserId) -> Option<User> {
        self.inner.borrow().users.get(&id.to_string()).cloned()
    }

    /// List all users (order unspecified).
    pub fn list_users(&self) -> Vec<User> {
        self.inner.borrow().users.values().cloned().collect()
    }

    /// Delete a user by id.  No-op if absent.
    pub fn delete_user(&self, id: &UserId) {
        self.inner.borrow_mut().users.remove(&id.to_string());
    }

    // -----------------------------------------------------------------------
    // Blocked / administrated lookups
    // -----------------------------------------------------------------------

    /// List all node ids that `user_id` has blocked.
    pub fn list_blocked_nodes(&self, user_id: &UserId) -> Vec<NodeId> {
        let inner = self.inner.borrow();
        let user_s = user_id.to_string();
        inner
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Blocked && e.from_ == user_s)
            .filter_map(|e| NodeId::parse(&e.to_).ok())
            .collect()
    }

    /// List all user ids that have blocked `node_id`.
    pub fn list_blocked_users(&self, node_id: &NodeId) -> Vec<UserId> {
        let inner = self.inner.borrow();
        let node_s = node_id.to_string();
        inner
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Blocked && e.to_ == node_s)
            .filter_map(|e| UserId::parse(&e.from_).ok())
            .collect()
    }

    /// List all node ids that `user_id` administrates.
    pub fn list_administrated_nodes(&self, user_id: &UserId) -> Vec<NodeId> {
        let inner = self.inner.borrow();
        let user_s = user_id.to_string();
        inner
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Administrates && e.from_ == user_s)
            .filter_map(|e| NodeId::parse(&e.to_).ok())
            .collect()
    }

    /// List all `(node_id, edge_kind)` access edges for `user_id`.
    ///
    /// Returns edges with kind `Administrates`, `Reads`, or `Writes`.
    pub fn list_access_edges(&self, user_id: &UserId) -> Vec<(NodeId, EdgeKind)> {
        let inner = self.inner.borrow();
        let user_s = user_id.to_string();
        inner
            .edges
            .iter()
            .filter(|e| {
                e.from_ == user_s
                    && matches!(
                        e.kind,
                        EdgeKind::Administrates | EdgeKind::Reads | EdgeKind::Writes
                    )
            })
            .filter_map(|e| NodeId::parse(&e.to_).ok().map(|nid| (nid, e.kind.clone())))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Convenience seeding API
    // -----------------------------------------------------------------------

    /// Seed the store with a node (for test setup).
    pub fn with_node(self, node: Node) -> Self {
        self.put_node(&node);
        self
    }

    /// Seed the store with a user (for test setup).
    pub fn with_user(self, user: User) -> Self {
        self.put_user(&user);
        self
    }

    /// Seed the store with a sensor (for test setup).
    pub fn with_sensor(self, sensor: Sensor) -> Self {
        {
            let mut inner = self.inner.borrow_mut();
            inner
                .sensors
                .entry(sensor.id.to_string())
                .or_default()
                .push(SensorRow::Active(sensor));
        }
        self
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Re-export EdgeSpec from the parent repository module so callers that
// import from `memory` still see it.
// ---------------------------------------------------------------------------

pub use super::EdgeSpec;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::Level;
    use crate::domain::node::{make_root, make as make_node};
    use crate::domain::values::{CognitoGroup, ReadingKind, EnergyType};

    fn ts() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z"
            .parse::<DateTime<Utc>>()
            .unwrap()
    }

    fn make_user(email: &str) -> User {
        User::builder()
            .email(email.to_owned())
            .name("Test User".to_owned())
            .cognito_group(CognitoGroup::Reader)
            .created(ts())
            .build()
    }

    fn make_sensor_for(parent: &Node, sid: u32) -> Sensor {
        let path = format!("{}|S#{}", parent.path, sid);
        Sensor::builder()
            .id(SensorId::make(sid))
            .created(ts())
            .daq_id(format!("daq:{}", sid))
            .path(path)
            .energy_type(EnergyType::Electricity)
            .reading_kind(ReadingKind::Counter)
            .build()
    }

    // -----------------------------------------------------------------------
    // Node put/get round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn node_put_get_roundtrip() {
        let store = Store::new();
        let root = make_root();
        store.put_node(&root);

        let got = store.get_node(&NodeId::root());
        assert!(got.is_some());
        let got = got.unwrap();
        assert_eq!(got.id, NodeId::root());
        assert_eq!(got.name, "root");
    }

    #[test]
    fn get_absent_node_returns_none() {
        let store = Store::new();
        let id = NodeId::make(Level::Hn1, 99999);
        assert!(store.get_node(&id).is_none());
    }

    #[test]
    fn put_node_overwrites() {
        let store = Store::new();
        let n1 = make_root();
        store.put_node(&n1);
        let mut n2 = n1.clone();
        n2.name = "updated".to_owned();
        store.put_node(&n2);
        assert_eq!(store.get_node(&NodeId::root()).unwrap().name, "updated");
    }

    // -----------------------------------------------------------------------
    // Node delete
    // -----------------------------------------------------------------------

    #[test]
    fn delete_node_removes_it_and_edges() {
        let store = Store::new();
        let root = make_root();
        store.put_node(&root);

        let child = make_node(10001, Level::Hn1, "child", NodeId::root(),
                              &NodeId::root().to_string(),
                              serde_json::json!({}), None);
        store.put_node(&child);
        store.put_edge(EdgeSpec {
            from_: NodeId::root().to_string(),
            to_: child.id.to_string(),
            kind: EdgeKind::HasLabel("building".to_owned()),
            name: "child".to_owned(),
        });

        // Sanity: child is listed.
        assert_eq!(store.list_child_refs(&NodeId::root(), None).len(), 1);

        store.delete_node(&child.id);
        assert!(store.get_node(&child.id).is_none());
        // Edge should be gone.
        assert!(store.list_child_refs(&NodeId::root(), None).is_empty());
    }

    // -----------------------------------------------------------------------
    // Child ref listing
    // -----------------------------------------------------------------------

    #[test]
    fn list_child_refs_returns_edges_under_parent() {
        let store = Store::new();
        let root = make_root();
        store.put_node(&root);

        let a = make_node(10001, Level::Hn1, "A", NodeId::root(),
                          &NodeId::root().to_string(), serde_json::json!({}), None);
        let b = make_node(10002, Level::Hn1, "B", NodeId::root(),
                          &NodeId::root().to_string(), serde_json::json!({}), None);
        store.put_node(&a);
        store.put_node(&b);

        store.put_edge(EdgeSpec { from_: NodeId::root().to_string(), to_: a.id.to_string(),
                                  kind: EdgeKind::HasLabel("building".to_owned()), name: "A".to_owned() });
        store.put_edge(EdgeSpec { from_: NodeId::root().to_string(), to_: b.id.to_string(),
                                  kind: EdgeKind::HasLabel("building".to_owned()), name: "B".to_owned() });

        let refs = store.list_child_refs(&NodeId::root(), None);
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn list_child_refs_filters_by_kind() {
        let store = Store::new();
        let parent_id = NodeId::root();

        let a_id = NodeId::make(Level::Hn1, 1);
        let b_id = NodeId::make(Level::Hn1, 2);

        store.put_edge(EdgeSpec { from_: parent_id.to_string(), to_: a_id.to_string(),
                                  kind: EdgeKind::HasLabel("building".to_owned()), name: "A".to_owned() });
        store.put_edge(EdgeSpec { from_: parent_id.to_string(), to_: b_id.to_string(),
                                  kind: EdgeKind::HasLabel("floor".to_owned()), name: "B".to_owned() });

        let building_kind = EdgeKind::HasLabel("building".to_owned());
        let refs = store.list_child_refs(&parent_id, Some(&building_kind));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, a_id);
    }

    // -----------------------------------------------------------------------
    // list_children
    // -----------------------------------------------------------------------

    #[test]
    fn list_children_returns_nodes() {
        let store = Store::new();
        let root = make_root();
        store.put_node(&root);

        let child = make_node(10001, Level::Hn1, "Child", NodeId::root(),
                              &NodeId::root().to_string(), serde_json::json!({}), None);
        store.put_node(&child);
        store.put_edge(EdgeSpec { from_: NodeId::root().to_string(), to_: child.id.to_string(),
                                  kind: EdgeKind::HasLabel("building".to_owned()), name: "Child".to_owned() });

        let children = store.list_children(&NodeId::root(), None);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "Child");
    }

    // -----------------------------------------------------------------------
    // delete_edge
    // -----------------------------------------------------------------------

    #[test]
    fn delete_edge_removes_matching_edge() {
        let store = Store::new();
        let from_ = NodeId::root().to_string();
        let to_ = NodeId::make(Level::Hn1, 1).to_string();
        let kind = EdgeKind::HasLabel("building".to_owned());

        store.put_edge(EdgeSpec { from_: from_.clone(), to_: to_.clone(),
                                  kind: kind.clone(), name: "x".to_owned() });
        assert_eq!(store.list_child_refs(&NodeId::root(), None).len(), 1);

        store.delete_edge(&from_, &to_, &kind);
        assert!(store.list_child_refs(&NodeId::root(), None).is_empty());
    }

    // -----------------------------------------------------------------------
    // Id allocation
    // -----------------------------------------------------------------------

    #[test]
    fn allocate_node_id_increments() {
        let store = Store::new();
        let id1 = store.allocate_node_id(Level::Hn1);
        let id2 = store.allocate_node_id(Level::Hn1);
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn allocate_node_id_per_level_independent() {
        let store = Store::new();
        let a = store.allocate_node_id(Level::Hn1);
        let b = store.allocate_node_id(Level::Hn2);
        // Both start from 10_001 (seed 10_000 + 1).
        assert_eq!(a, 10_001);
        assert_eq!(b, 10_001);
    }

    #[test]
    fn allocate_sensor_id_increments() {
        let store = Store::new();
        let s1 = store.allocate_sensor_id();
        let s2 = store.allocate_sensor_id();
        assert_eq!(s2, s1 + 1);
    }

    // -----------------------------------------------------------------------
    // add_node
    // -----------------------------------------------------------------------

    #[test]
    fn add_node_stores_node_and_edge() {
        let store = Store::new();
        let root = make_root();
        store.put_node(&root);

        let node = store.add_node(Level::Hn1, |raw_id| {
            let n = make_node(raw_id, Level::Hn1, "New", NodeId::root(),
                              &NodeId::root().to_string(), serde_json::json!({}), None);
            let e = EdgeSpec {
                from_: NodeId::root().to_string(),
                to_: n.id.to_string(),
                kind: EdgeKind::HasLabel("building".to_owned()),
                name: "New".to_owned(),
            };
            (n, e)
        });

        assert!(store.get_node(&node.id).is_some());
        let refs = store.list_child_refs(&NodeId::root(), None);
        assert_eq!(refs.len(), 1);
    }

    // -----------------------------------------------------------------------
    // User put/get/delete/list
    // -----------------------------------------------------------------------

    #[test]
    fn user_put_get_roundtrip() {
        let store = Store::new();
        let u = make_user("alice@example.com");
        store.put_user(&u);
        let got = store.get_user(&u.id);
        assert!(got.is_some());
        assert_eq!(got.unwrap().email, "alice@example.com");
    }

    #[test]
    fn get_absent_user_returns_none() {
        let store = Store::new();
        let id = UserId::of_email("nobody@example.com");
        assert!(store.get_user(&id).is_none());
    }

    #[test]
    fn user_delete() {
        let store = Store::new();
        let u = make_user("bob@example.com");
        store.put_user(&u);
        store.delete_user(&u.id);
        assert!(store.get_user(&u.id).is_none());
    }

    #[test]
    fn list_users_returns_all() {
        let store = Store::new();
        store.put_user(&make_user("a@a.com"));
        store.put_user(&make_user("b@b.com"));
        let users = store.list_users();
        assert_eq!(users.len(), 2);
    }

    #[test]
    fn list_users_empty() {
        let store = Store::new();
        assert!(store.list_users().is_empty());
    }

    // -----------------------------------------------------------------------
    // Sensor get/add/delete
    // -----------------------------------------------------------------------

    #[test]
    fn sensor_put_get_roundtrip() {
        let root = make_root();
        let s = make_sensor_for(&root, 20001);
        let store = Store::new().with_node(root).with_sensor(s.clone());
        let got = store.get_active_sensor(&s.id);
        assert!(got.is_some());
        assert_eq!(got.unwrap().id, s.id);
    }

    #[test]
    fn get_absent_sensor_returns_none() {
        let store = Store::new();
        assert!(store.get_active_sensor(&SensorId::make(99999)).is_none());
    }

    #[test]
    fn add_sensor_stores_sensor_and_edge() {
        let store = Store::new();
        let root = make_root();
        store.put_node(&root);

        let sensor = store.add_sensor(|raw_id| {
            let path = format!("{}|S#{}", root.path, raw_id);
            let s = Sensor::builder()
                .id(SensorId::make(raw_id))
                .created(ts())
                .daq_id("daq:test".to_owned())
                .path(path)
                .energy_type(EnergyType::Electricity)
                .reading_kind(ReadingKind::Counter)
                .build();
            let e = EdgeSpec {
                from_: root.id.to_string(),
                to_: s.id.to_string(),
                kind: EdgeKind::HasSensor,
                name: "".to_owned(),
            };
            (s, e)
        });

        assert!(store.get_active_sensor(&sensor.id).is_some());
        let ids = store.list_sensor_ids(&root.id);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], sensor.id);
    }

    #[test]
    fn delete_sensor_removes_it() {
        let store = Store::new();
        let root = make_root();
        store.put_node(&root);

        let sensor = store.add_sensor(|raw_id| {
            let path = format!("{}|S#{}", root.path, raw_id);
            let s = Sensor::builder()
                .id(SensorId::make(raw_id))
                .created(ts())
                .daq_id("daq:test".to_owned())
                .path(path)
                .energy_type(EnergyType::Electricity)
                .reading_kind(ReadingKind::Counter)
                .build();
            let e = EdgeSpec {
                from_: root.id.to_string(),
                to_: s.id.to_string(),
                kind: EdgeKind::HasSensor,
                name: "".to_owned(),
            };
            (s, e)
        });

        store.delete_sensor(&sensor.id, &root.id);
        assert!(store.get_active_sensor(&sensor.id).is_none());
        assert!(store.list_sensor_ids(&root.id).is_empty());
    }

    // -----------------------------------------------------------------------
    // list_sensors_under_path
    // -----------------------------------------------------------------------

    #[test]
    fn list_sensors_under_path_filters_by_prefix() {
        let store = Store::new();
        let root = make_root();
        store.put_node(&root);

        let s1 = Sensor::builder()
            .id(SensorId::make(1))
            .created(ts())
            .daq_id("d1".to_owned())
            .path("HN0#root|HN1#1|S#1".to_owned())
            .energy_type(EnergyType::Electricity)
            .reading_kind(ReadingKind::Counter)
            .build();
        let s2 = Sensor::builder()
            .id(SensorId::make(2))
            .created(ts())
            .daq_id("d2".to_owned())
            .path("HN0#root|HN1#2|S#2".to_owned())
            .energy_type(EnergyType::Electricity)
            .reading_kind(ReadingKind::Counter)
            .build();
        {
            let mut inner = store.inner.borrow_mut();
            inner.sensors.entry("S#1".to_owned()).or_default().push(SensorRow::Active(s1));
            inner.sensors.entry("S#2".to_owned()).or_default().push(SensorRow::Active(s2));
        }

        let results = store.list_sensors_under_path("HN0#root|HN1#1");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, SensorId::make(1));
    }

    // -----------------------------------------------------------------------
    // replace_sensor_device
    // -----------------------------------------------------------------------

    #[test]
    fn replace_sensor_device_demotes_old_and_activates_new() {
        let store = Store::new();
        let old_created = ts();
        let old_s = Sensor::builder()
            .id(SensorId::make(10))
            .created(old_created)
            .daq_id("old".to_owned())
            .path("HN0#root|S#10".to_owned())
            .energy_type(EnergyType::Electricity)
            .reading_kind(ReadingKind::Counter)
            .build();
        {
            let mut inner = store.inner.borrow_mut();
            inner.sensors.entry("S#10".to_owned()).or_default()
                .push(SensorRow::Active(old_s.clone()));
        }

        let new_ts = "2026-06-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let new_s = Sensor::builder()
            .id(SensorId::make(10))
            .created(new_ts)
            .daq_id("new".to_owned())
            .path("HN0#root|S#10".to_owned())
            .energy_type(EnergyType::Electricity)
            .reading_kind(ReadingKind::Counter)
            .build();

        store.replace_sensor_device(old_created, &new_s);

        let active = store.get_active_sensor(&SensorId::make(10));
        assert!(active.is_some());
        assert_eq!(active.unwrap().daq_id, "new");
    }

    // -----------------------------------------------------------------------
    // Administrated / blocked edges
    // -----------------------------------------------------------------------

    #[test]
    fn list_administrated_nodes() {
        let store = Store::new();
        let user_id = UserId::of_email("admin@example.com");
        let node1 = NodeId::make(Level::Hn1, 100);
        let node2 = NodeId::make(Level::Hn2, 200);

        store.put_edge(EdgeSpec {
            from_: user_id.to_string(), to_: node1.to_string(),
            kind: EdgeKind::Administrates, name: "".to_owned(),
        });
        store.put_edge(EdgeSpec {
            from_: user_id.to_string(), to_: node2.to_string(),
            kind: EdgeKind::Administrates, name: "".to_owned(),
        });

        let nodes = store.list_administrated_nodes(&user_id);
        assert_eq!(nodes.len(), 2);
        assert!(nodes.contains(&node1));
        assert!(nodes.contains(&node2));
    }

    #[test]
    fn list_blocked_nodes() {
        let store = Store::new();
        let user_id = UserId::of_email("blocker@example.com");
        let node = NodeId::make(Level::Hn3, 300);

        store.put_edge(EdgeSpec {
            from_: user_id.to_string(), to_: node.to_string(),
            kind: EdgeKind::Blocked, name: "".to_owned(),
        });

        let nodes = store.list_blocked_nodes(&user_id);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0], node);
    }

    #[test]
    fn list_blocked_users() {
        let store = Store::new();
        let node = NodeId::make(Level::Hn2, 42);
        let user1 = UserId::of_email("u1@example.com");
        let user2 = UserId::of_email("u2@example.com");

        store.put_edge(EdgeSpec {
            from_: user1.to_string(), to_: node.to_string(),
            kind: EdgeKind::Blocked, name: "".to_owned(),
        });
        store.put_edge(EdgeSpec {
            from_: user2.to_string(), to_: node.to_string(),
            kind: EdgeKind::Blocked, name: "".to_owned(),
        });

        let users = store.list_blocked_users(&node);
        assert_eq!(users.len(), 2);
        assert!(users.contains(&user1));
        assert!(users.contains(&user2));
    }

    // -----------------------------------------------------------------------
    // Clock seam
    // -----------------------------------------------------------------------

    #[test]
    fn clock_seam_returns_fixed_time() {
        let fixed: DateTime<Utc> = "2026-03-15T12:00:00Z".parse().unwrap();
        let store = Store::with_clock(move || fixed);
        assert_eq!(store.now(), fixed);
    }

    // -----------------------------------------------------------------------
    // Seeding API
    // -----------------------------------------------------------------------

    #[test]
    fn with_node_seeds_store() {
        let root = make_root();
        let store = Store::new().with_node(root.clone());
        assert!(store.get_node(&NodeId::root()).is_some());
    }

    #[test]
    fn with_user_seeds_store() {
        let u = make_user("seed@example.com");
        let store = Store::new().with_user(u.clone());
        assert!(store.get_user(&u.id).is_some());
    }

    #[test]
    fn get_sensor_reading_always_none() {
        let store = Store::new();
        assert!(store.get_sensor_reading(&SensorId::make(1)).is_none());
    }
}
