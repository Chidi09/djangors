use thiserror::Error;

#[derive(Error, Debug)]
pub enum StaticFilesError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Hashing error: {0}")]
    Hashing(String),
}
