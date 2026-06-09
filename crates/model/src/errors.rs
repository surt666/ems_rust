use thiserror::Error;

use crate::domain::ids::{NodeId, UserId};

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
}
