use thiserror::Error;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("codec: {0}")]
    Codec(String),
    #[error("not found")]
    NotFound,
}
