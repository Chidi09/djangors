use thiserror::Error;

#[derive(Error, Debug)]
pub enum MigrationError {
    #[error("Field {field} is missing max_length attribute")]
    MissingMaxLength { field: String },

    #[error("Cyclic dependency detected between models: {models:?}")]
    CyclicDependency { models: Vec<String> },

    #[error("Database error: {0}")]
    Database(#[from] djangors_db::DbError),

    #[error("Query execution error: {0}")]
    Query(#[from] sqlx::Error),
}
