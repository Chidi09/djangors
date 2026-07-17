use thiserror::Error;

/// Errors that can occur within the database layer of the Djangors framework.
#[derive(Debug, Error)]
pub enum DbError {
    /// Failed to establish a connection to the database (e.g. pool creation failure).
    #[error("Database connection failed: {0}")]
    ConnectionFailed(String),

    /// Database query execution failed.
    #[error("Database query failed: {0}")]
    QueryFailed(#[from] sqlx::Error),

    /// Transaction lifecycle (begin, commit, rollback, or setting isolation level) failed.
    #[error("Transaction failed: {0}")]
    TransactionFailed(String),

    /// Connection pool has been exhausted.
    #[error("Database connection pool exhausted")]
    PoolExhausted,
}
