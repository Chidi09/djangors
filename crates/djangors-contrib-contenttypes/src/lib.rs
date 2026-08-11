#![deny(missing_docs)]
//! Stable model identities and generic foreign-key pairs for Djangors.
//!
//! The contenttypes framework tracks all registered models in the Djangors application
//! by storing their unique `(app_label, model_name)` pairs in the database. This enables:
//! - [`ContentType`]: Stably mapped IDs for any model structure registered in the ORM.
//! - [`GenericForeignKey`]: Dynamic references linking to any row in any database table via a content-type ID and instance ID.
//! - Synchronization: [`sync_content_types`] iterates over all registered models at runtime to register them in the database.

use djangors_macros::Model;
use thiserror::Error;

/// A registered model identity stored in the content-types table.
#[derive(Model, Debug, Clone)]
#[djangors(
    app = "djangors_contrib_contenttypes",
    table_name = "djangors_content_type",
    unique_together = [["app_label", "model_name"]]
)]
pub struct ContentType {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Application label from the model metadata.
    #[djangors(max_length = 100)]
    pub app_label: String,
    /// Rust model struct name from the model metadata.
    #[djangors(max_length = 100)]
    pub model_name: String,
}

/// A `(content_type_id, object_id)` reference to a concrete object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenericForeignKey {
    /// ID of the referenced [`ContentType`].
    pub content_type_id: i64,
    /// ID of the referenced object.
    pub object_id: i64,
}

/// Errors returned by content-type operations.
#[derive(Debug, Error)]
pub enum ContentTypeError {
    /// A database query failed.
    #[error("database error: {0}")]
    Database(#[from] djangors_orm::OrmError),
    /// No content type exists for the requested ID.
    #[error("content type {0} was not found")]
    NotFound(i64),
}

/// Ensures one content-type row exists for every registered model.
pub async fn sync_content_types(db: &djangors_db::Database) -> Result<usize, ContentTypeError> {
    let mut count = 0;
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let p1 = dialect.placeholder(1);
    let p2 = dialect.placeholder(2);
    let sql = format!(
        "INSERT INTO djangors_content_type (app_label, model_name) VALUES ({p1}, {p2}) \
         ON CONFLICT (app_label, model_name) DO NOTHING"
    );
    for meta in djangors_orm::meta::all_registered_models() {
        let params = vec![
            djangors_db::BindValue::Text(meta.app_label.to_string()),
            djangors_db::BindValue::Text(meta.struct_name.to_string()),
        ];
        conn.execute(&sql, &params)
            .await
            .map_err(|e| ContentTypeError::Database(djangors_orm::OrmError::Query(e)))?;
        count += 1;
    }
    Ok(count)
}

/// Builds a generic key for a compile-time-known model type, creating its row lazily.
pub async fn generic_key_for<T: djangors_orm::Model>(
    db: &djangors_db::Database,
    object_id: i64,
) -> Result<GenericForeignKey, ContentTypeError> {
    let meta = T::meta();
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let p1 = dialect.placeholder(1);
    let p2 = dialect.placeholder(2);
    let sql_ins = format!(
        "INSERT INTO djangors_content_type (app_label, model_name) VALUES ({p1}, {p2}) \
         ON CONFLICT (app_label, model_name) DO NOTHING"
    );
    let params = vec![
        djangors_db::BindValue::Text(meta.app_label.to_string()),
        djangors_db::BindValue::Text(meta.struct_name.to_string()),
    ];
    conn.execute(&sql_ins, &params)
        .await
        .map_err(|e| ContentTypeError::Database(djangors_orm::OrmError::Query(e)))?;

    let sql_sel = format!(
        "SELECT id FROM djangors_content_type WHERE app_label = {p1} AND model_name = {p2}"
    );
    let row = conn
        .fetch_one(&sql_sel, &params)
        .await
        .map_err(|e| ContentTypeError::Database(djangors_orm::OrmError::Query(e)))?;
    let content_type_id = row
        .try_i64(0)
        .map_err(|e| ContentTypeError::Database(djangors_orm::OrmError::Query(e)))?
        .ok_or_else(|| {
            ContentTypeError::Database(djangors_orm::OrmError::Query(
                djangors_orm::sqlx::Error::RowNotFound,
            ))
        })?;

    Ok(GenericForeignKey {
        content_type_id,
        object_id,
    })
}

/// Resolves a content-type ID to its `(app_label, model_name)` pair.
pub async fn resolve_content_type(
    db: &djangors_db::Database,
    content_type_id: i64,
) -> Result<(String, String), ContentTypeError> {
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let sql = format!("SELECT app_label, model_name FROM djangors_content_type WHERE id = {p1}");
    let params = vec![djangors_db::BindValue::I64(content_type_id)];
    let row_opt = conn
        .fetch_optional(&sql, &params)
        .await
        .map_err(|e| ContentTypeError::Database(djangors_orm::OrmError::Query(e)))?;

    let row = row_opt.ok_or(ContentTypeError::NotFound(content_type_id))?;
    let app_label = row
        .try_string(0)
        .map_err(|e| ContentTypeError::Database(djangors_orm::OrmError::Query(e)))?
        .unwrap_or_default();
    let model_name = row
        .try_string(1)
        .map_err(|e| ContentTypeError::Database(djangors_orm::OrmError::Query(e)))?
        .unwrap_or_default();

    Ok((app_label, model_name))
}

#[cfg(test)]
mod tests;
