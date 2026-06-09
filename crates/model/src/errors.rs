use thiserror::Error;

use crate::domain::ids::{NodeId, UserId};
use crate::domain::schema::MetadataError;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("codec: {0}")]
    Codec(String),
    #[error("node {0} not found")]
    NotFound(NodeId),
    #[error("user {0} not found")]
    NotFoundUser(UserId),
    #[error("conflict: {0}")]
    Conflict(String),
    /// Schema missing — mirrors OCaml `Errors.Schema_missing`.
    #[error("no hn2 schema found above {0}")]
    SchemaMissing(NodeId),
    /// Validation failure — mirrors OCaml `Errors.Validation`.
    #[error("validation failed")]
    Validation(Vec<MetadataError>),
    /// Bad request — mirrors OCaml `Errors.Bad_request`.
    #[error("{0}")]
    BadRequest(String),
}
