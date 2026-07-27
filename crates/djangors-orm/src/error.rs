use thiserror::Error;

/// Error types for ORM database query and mapping operations.
#[derive(Debug, Error)]
pub enum OrmError {
    /// An underlying SQL query execution failed.
    #[error("Database query failed: {0}")]
    Query(#[from] sqlx::Error),

    /// A query for a single object found no matching record.
    #[error("Model {model} not found")]
    NotFound {
        /// Struct name of the target model.
        model: &'static str,
    },

    /// A query for a single object returned more than one matching record.
    #[error("Multiple {model} objects returned")]
    MultipleObjectsReturned {
        /// Struct name of the target model.
        model: &'static str,
    },

    /// Specified field name was not found on the model metadata.
    #[error("Field {field} not found on model {model}")]
    FieldNotFound {
        /// Name of the requested field.
        field: String,
        /// Struct name of the target model.
        model: &'static str,
    },
}

/// Trait implemented by models to construct instances from a database row.
pub trait FromRow: Sized {
    /// Converts a PostgreSQL database row into an instance of `Self`.
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, OrmError>;
}
