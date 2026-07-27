use thiserror::Error;

/// Error types for database migration planning and execution.
#[derive(Error, Debug)]
pub enum MigrationError {
    /// A character field is missing a required `max_length` attribute.
    #[error("Field {field} is missing max_length attribute")]
    MissingMaxLength {
        /// The name of the model field.
        field: String,
    },

    /// A cyclic dependency loop was detected between model foreign keys.
    #[error("Cyclic dependency detected between models: {models:?}")]
    CyclicDependency {
        /// Model names involved in the dependency cycle.
        models: Vec<String>,
    },

    /// A database operation or connection error occurred.
    #[error("Database error: {0}")]
    Database(#[from] djangors_db::DbError),

    /// A raw SQL query execution error occurred.
    #[error("Query execution error: {0}")]
    Query(#[from] sqlx::Error),
}
