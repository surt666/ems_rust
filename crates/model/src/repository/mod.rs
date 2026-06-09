pub mod cognito;
pub mod dynamodb;

#[cfg(any(test, feature = "testing"))]
pub mod memory;

// ---------------------------------------------------------------------------
// EdgeSpec — shared by the logic layer and both repository backends
// ---------------------------------------------------------------------------

use crate::domain::values::EdgeKind;

/// The minimal information needed to write an edge.
///
/// Mirrors OCaml `Effects.edge_spec`:
/// `{ from_ : string; to_ : string; kind : Edge_kind.t; name : string }`.
///
/// Defined here (unconditionally) so the logic layer can reference it without
/// depending on the test-gated `memory` module.
pub struct EdgeSpec {
    pub from_: String,
    pub to_: String,
    pub kind: EdgeKind,
    pub name: String,
}
