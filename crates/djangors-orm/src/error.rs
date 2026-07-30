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

    /// A queryset was built in a way that cannot produce valid SQL — for
    /// example `annotate` with no group-by field. This is a programming
    /// mistake, caught before the query reaches the database.
    #[error("Invalid query: {0}")]
    InvalidQuery(String),

    /// Specified field name was not found on the model metadata.
    #[error("Field {field} not found on model {model}")]
    FieldNotFound {
        /// Name of the requested field.
        field: String,
        /// Struct name of the target model.
        model: &'static str,
    },
}

/// Lets ORM calls be used directly inside
/// [`Database::transaction`](djangors_db::Database::transaction), whose closure
/// must return an error convertible into [`DbError`](djangors_db::DbError).
///
/// SQL failures pass through with their original `sqlx::Error` intact; the
/// ORM's own failure modes are carried as [`DbError::Orm`](djangors_db::DbError::Orm).
impl From<OrmError> for djangors_db::DbError {
    fn from(err: OrmError) -> Self {
        match err {
            OrmError::Query(e) => djangors_db::DbError::QueryFailed(e),
            other => djangors_db::DbError::Orm(other.to_string()),
        }
    }
}

/// Trait implemented by models to construct instances from a database row.
pub trait FromRow: Sized {
    /// Converts a PostgreSQL database row into an instance of `Self`.
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, OrmError>;
}
