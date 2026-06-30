use chrono::{DateTime, Utc};
use typed_builder::TypedBuilder;

use crate::domain::ids::{Level, NodeId};
use crate::domain::schema::Schema;

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A hierarchy node.
///
/// `path` is a pipe-separated list of ancestor node-ids from root down to and
/// including self (the `gsi1sk` attribute in DynamoDB).
/// `metadata` is an arbitrary JSON object.
#[derive(Clone, Debug, PartialEq, TypedBuilder)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    #[builder(default)]
    pub parent: Option<NodeId>,
    pub path: String,
    #[builder(default = chrono::Utc::now())]
    pub created: DateTime<Utc>,
    #[builder(default = serde_json::json!({}))]
    pub metadata: serde_json::Value,
    #[builder(default)]
    pub schema: Option<Schema>,
    /// Node type from the company schema ("partner", "company", "building", …).
    /// Empty string when not yet backfilled (pre-migration items).
    #[builder(default)]
    pub label: String,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Path separator used in all node (and sensor) paths.
pub const PATH_SEP: &str = "|";

/// Build the path of a child node given the parent's full path and the child's
/// id as a string.
pub fn child_path(parent_path: &str, child_id_str: &str) -> String {
    format!("{}{}{}", parent_path, PATH_SEP, child_id_str)
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

/// Create a non-root node.
///
/// - Computes the `NodeId` from (`level`, `id`).
/// - Computes `path = child_path(parent_path, node_id_str)`.
///
/// `created` is set to the current UTC time via the builder default.
#[allow(clippy::too_many_arguments)]
pub fn make(
    id: u32,
    level: Level,
    name: &str,
    parent: NodeId,
    parent_path: &str,
    metadata: serde_json::Value,
    schema: Option<Schema>,
) -> Node {
    let nid = NodeId::make(level, id);
    let path = child_path(parent_path, &nid.to_string());
    Node::builder()
        .id(nid)
        .name(name.to_owned())
        .parent(Some(parent))
        .path(path)
        .metadata(metadata)
        .schema(schema)
        .build()
}

/// Create the root node.
///
/// `id = Node_id::root`, `name = "root"`, `parent = None`, `path = root id string`.
/// `created` is set to the current UTC time via the builder default.
pub fn make_root() -> Node {
    Node::builder()
        .id(NodeId::root())
        .name("root".to_owned())
        .path(NodeId::root().to_string())
        .build()
}

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

impl Node {
    /// Return the hierarchy level of this node's id.
    pub const fn level(&self) -> Level {
        self.id.level()
    }
}

/// Extract the path segment whose level matches `lvl`.
///
/// Splits `path` on `|` and returns the first segment starting with `HN<depth>#`.
pub fn segment_at_level(path: &str, lvl: Level) -> Option<String> {
    let prefix = format!("HN{}#", lvl.depth());
    path.split(PATH_SEP)
        .filter(|s| !s.is_empty())
        .find(|s| s.starts_with(&prefix))
        .map(|s| s.to_owned())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn make_root_test() {
        let before = Utc::now();
        let n = make_root();
        let after = Utc::now();
        assert_eq!(n.id.to_string(), "HN0#root");
        assert!(n.parent.is_none());
        assert_eq!(n.path, "HN0#root");
        assert_eq!(n.name, "root");
        assert!(n.created >= before && n.created <= after);
    }

    #[test]
    fn make_child_test() {
        let parent = NodeId::root();
        let parent_path = NodeId::root().to_string();
        let before = Utc::now();
        let n = make(
            10001,
            Level::Hn1,
            "Acme",
            parent,
            &parent_path,
            serde_json::json!({}),
            None,
        );
        let after = Utc::now();
        assert_eq!(n.id.to_string(), "HN1#10001");
        assert_eq!(n.parent.as_ref().unwrap().to_string(), "HN0#root");
        // path = "HN0#root|HN1#10001"
        assert_eq!(n.path, "HN0#root|HN1#10001");
        assert!(n.created >= before && n.created <= after);
    }

    /// child_path concatenates with `|`.
    #[test]
    fn child_path_concat() {
        let p = child_path("HN0#root", "HN1#42");
        assert_eq!(p, "HN0#root|HN1#42");
    }

    /// segment_at_level finds the right segment.
    #[test]
    fn segment_at_level_finds() {
        let path = "HN0#root|HN1#10001|HN2#20002";
        assert_eq!(
            segment_at_level(path, Level::Hn1),
            Some("HN1#10001".to_owned())
        );
        assert_eq!(
            segment_at_level(path, Level::Hn2),
            Some("HN2#20002".to_owned())
        );
        assert_eq!(segment_at_level(path, Level::Hn5), None);
    }

    /// level() delegates to NodeId::level().
    #[test]
    fn node_level_accessor() {
        let n = make(
            42,
            Level::Hn3,
            "x",
            NodeId::root(),
            "HN0#root",
            serde_json::json!({}),
            None,
        );
        assert_eq!(n.level(), Level::Hn3);
    }

    /// make_root metadata is an empty JSON object.
    #[test]
    fn make_root_metadata_empty_object() {
        let n = make_root();
        assert!(n.metadata.is_object());
        assert_eq!(n.metadata.as_object().unwrap().len(), 0);
    }

    /// label defaults to empty and is settable.
    #[test]
    fn label_default_and_set() {
        let mut n = make(
            42, Level::Hn3, "x", NodeId::root(), "HN0#root",
            serde_json::json!({}), None,
        );
        assert_eq!(n.label, "");
        n.label = "building".to_string();
        assert_eq!(n.label, "building");
    }

    /// Deeper path: Hn2 child of Hn1 child of root.
    #[test]
    fn make_deep_child_path() {
        let hn1 = make(
            10001,
            Level::Hn1,
            "Hn1",
            NodeId::root(),
            &NodeId::root().to_string(),
            serde_json::json!({}),
            None,
        );
        let hn2 = make(
            20002,
            Level::Hn2,
            "Hn2",
            hn1.id.clone(),
            &hn1.path,
            serde_json::json!({}),
            None,
        );
        assert_eq!(hn2.path, "HN0#root|HN1#10001|HN2#20002");
        assert_eq!(hn2.parent.as_ref().unwrap().to_string(), "HN1#10001");
    }
}
