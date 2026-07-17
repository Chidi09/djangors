use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrmError {
    #[error("Database query failed: {0}")]
    Query(#[from] sqlx::Error),

    #[error("Model {model} not found")]
    NotFound { model: &'static str },

    #[error("Multiple {model} objects returned")]
    MultipleObjectsReturned { model: &'static str },

    #[error("Field {field} not found on model {model}")]
    FieldNotFound { field: String, model: &'static str },
}

pub trait FromRow: Sized {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, OrmError>;
}
