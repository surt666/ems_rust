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
    /// Schema missing.
    #[error("no hn2 schema found above {0}")]
    SchemaMissing(NodeId),
    /// Validation failure.
    #[error("validation failed")]
    Validation(Vec<MetadataError>),
    /// Bad request.
    #[error("{0}")]
    BadRequest(String),
    /// AWS SDK call failed.
    #[error("aws: {0}")]
    Aws(String),
}
