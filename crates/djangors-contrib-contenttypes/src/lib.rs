#![deny(missing_docs)]
//! Stable model identities and generic foreign-key pairs for Djangors.

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
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Model, Debug, Clone)]
    #[djangors(app = "contenttypes_tests", table_name = "contenttypes_test_alpha")]
    struct Alpha {
        #[djangors(primary_key, auto)]
        id: i64,
    }

    #[derive(Model, Debug, Clone)]
    #[djangors(app = "contenttypes_tests", table_name = "contenttypes_test_beta")]
    struct Beta {
        #[djangors(primary_key, auto)]
        id: i64,
    }

    static DB_MUTEX: Mutex<()> = Mutex::new(());

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn sync_and_generic_lookup_are_stable_and_reversible() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let config = djangors_db::config::DatabaseConfig::new(
            "postgres://postgres:postgres@localhost/djangors_test",
        );
        let db = djangors_db::Database::connect(&config).await.unwrap();
        djangors_orm::sqlx::query("CREATE TABLE IF NOT EXISTS djangors_content_type (id BIGSERIAL PRIMARY KEY, app_label VARCHAR(100) NOT NULL, model_name VARCHAR(100) NOT NULL, UNIQUE (app_label, model_name))")
            .execute(db.pool())
            .await
            .unwrap();
        djangors_orm::sqlx::query("DELETE FROM djangors_content_type")
            .execute(db.pool())
            .await
            .unwrap();

        sync_content_types(&db).await.unwrap();
        let alpha = generic_key_for::<Alpha>(&db, 42).await.unwrap();
        let beta = generic_key_for::<Beta>(&db, 42).await.unwrap();
        let alpha_again = generic_key_for::<Alpha>(&db, 99).await.unwrap();
        assert_eq!(alpha.object_id, 42);
        assert_ne!(alpha.content_type_id, beta.content_type_id);
        assert_eq!(alpha.content_type_id, alpha_again.content_type_id);
        assert_eq!(
            resolve_content_type(&db, alpha.content_type_id)
                .await
                .unwrap(),
            ("contenttypes_tests".to_string(), "Alpha".to_string())
        );

        sync_content_types(&db).await.unwrap();
        let alpha_after = generic_key_for::<Alpha>(&db, 7).await.unwrap();
        let rows: i64 = djangors_orm::sqlx::query_scalar(
            "SELECT COUNT(*) FROM djangors_content_type WHERE app_label = $1 AND model_name = $2",
        )
        .bind("contenttypes_tests")
        .bind("Alpha")
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(alpha.content_type_id, alpha_after.content_type_id);
    }
}
