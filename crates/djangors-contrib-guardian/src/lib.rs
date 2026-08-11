#![deny(missing_docs)]
//! Object-level permissions for Djangors.
//!
//! Layered on top of `djangors_auth::has_perm`, `djangors-contrib-guardian` provides
//! granular object-level permission checking and management.
//!
//! # Example: Using in a custom view handler
//! ```ignore
//! async fn edit_document_view(
//!     db: &djangors_db::Database,
//!     user_id: i64,
//!     doc_id: i64,
//! ) -> Result<(), &'static str> {
//!     let allowed = djangors_contrib_guardian::has_perm_for_object(
//!         db,
//!         user_id,
//!         "change_document",
//!         "docs",
//!         "Document",
//!         doc_id,
//!     )
//!     .await
//!     .map_err(|_| "DB error")?;
//!
//!     if !allowed {
//!         return Err("Permission denied");
//!     }
//!     Ok(())
//! }
//! ```

use djangors_macros::Model;
use djangors_orm::ForeignKey;

/// Per-object permission grant model mapping a user, permission, and target object.
#[derive(Model, Debug, Clone)]
#[djangors(
    app = "djangors_contrib_guardian",
    table_name = "djangors_object_permission"
)]
pub struct ObjectPermission {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// The user to whom object-level permission is granted.
    pub user: ForeignKey<djangors_auth::User>,
    /// The permission being granted.
    pub permission: ForeignKey<djangors_auth::Permission>,
    /// The app label of the target object model (e.g. "docs").
    #[djangors(max_length = 100)]
    pub app_label: String,
    /// The struct name of the target object model (e.g. "Document").
    #[djangors(max_length = 100)]
    pub model_name: String,
    /// The primary key ID of the target object instance.
    pub object_id: i64,
}

/// Checks whether `user_id` has been granted `codename` for a specific model instance `(app_label, model_name, object_id)`.
///
/// Returns `true` if `djangors_auth::has_perm` already grants model-level permission, OR if an [`ObjectPermission`]
/// row exists granting `codename` on this exact object instance.
pub async fn has_perm_for_object(
    db: &djangors_db::Database,
    user_id: i64,
    codename: &str,
    app_label: &str,
    model_name: &str,
    object_id: i64,
) -> Result<bool, djangors_auth::AuthError> {
    if djangors_auth::has_perm(db, user_id, codename).await? {
        return Ok(true);
    }

    let mut conn = db.conn();
    let dialect = conn.dialect();
    let p1 = dialect.placeholder(1);
    let p2 = dialect.placeholder(2);
    let p3 = dialect.placeholder(3);
    let p4 = dialect.placeholder(4);
    let p5 = dialect.placeholder(5);
    let sql = format!(
        "SELECT COUNT(*) FROM djangors_object_permission op \
         JOIN auth_permission p ON p.id = op.permission \
         WHERE op.\"user\" = {p1} AND p.codename = {p2} AND op.app_label = {p3} AND op.model_name = {p4} AND op.object_id = {p5}"
    );
    let params = vec![
        djangors_db::BindValue::I64(user_id),
        djangors_db::BindValue::Text(codename.to_string()),
        djangors_db::BindValue::Text(app_label.to_string()),
        djangors_db::BindValue::Text(model_name.to_string()),
        djangors_db::BindValue::I64(object_id),
    ];
    let row = conn
        .fetch_one(&sql, &params)
        .await
        .map_err(|e| djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    let count = row
        .try_i64(0)
        .map_err(|e| djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(e)))?
        .unwrap_or(0);

    Ok(count > 0)
}

/// Grants an object-level permission `codename` on `(app_label, model_name, object_id)` to `user_id`.
pub async fn grant_object_permission(
    db: &djangors_db::Database,
    user_id: i64,
    codename: &str,
    app_label: &str,
    model_name: &str,
    object_id: i64,
) -> Result<ObjectPermission, djangors_auth::AuthError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let p1 = dialect.placeholder(1);
    let sql1 = format!("SELECT id FROM auth_permission WHERE codename = {p1}");
    let params1 = vec![djangors_db::BindValue::Text(codename.to_string())];
    let perm_row = conn
        .fetch_optional(&sql1, &params1)
        .await
        .map_err(|e| djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(e)))?
        .ok_or_else(|| {
            djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(
                djangors_orm::sqlx::Error::RowNotFound,
            ))
        })?;
    let perm_id = perm_row
        .try_i64(0)
        .map_err(|e| djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(e)))?
        .ok_or_else(|| {
            djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(
                djangors_orm::sqlx::Error::RowNotFound,
            ))
        })?;

    let p2 = dialect.placeholder(2);
    let p3 = dialect.placeholder(3);
    let p4 = dialect.placeholder(4);
    let p5 = dialect.placeholder(5);
    let sql2 = format!(
        "SELECT id FROM djangors_object_permission WHERE \"user\" = {p1} AND permission = {p2} AND app_label = {p3} AND model_name = {p4} AND object_id = {p5}"
    );
    let params2 = vec![
        djangors_db::BindValue::I64(user_id),
        djangors_db::BindValue::I64(perm_id),
        djangors_db::BindValue::Text(app_label.to_string()),
        djangors_db::BindValue::Text(model_name.to_string()),
        djangors_db::BindValue::I64(object_id),
    ];
    let exist_row = conn
        .fetch_optional(&sql2, &params2)
        .await
        .map_err(|e| djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    let existing = match exist_row {
        Some(r) => r
            .try_i64(0)
            .map_err(|e| djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(e)))?,
        None => None,
    };

    if let Some(id) = existing {
        return Ok(ObjectPermission {
            id,
            user: ForeignKey::new(user_id),
            permission: ForeignKey::new(perm_id),
            app_label: app_label.to_string(),
            model_name: model_name.to_string(),
            object_id,
        });
    }

    let obj_perm = ObjectPermission {
        id: 0,
        user: ForeignKey::new(user_id),
        permission: ForeignKey::new(perm_id),
        app_label: app_label.to_string(),
        model_name: model_name.to_string(),
        object_id,
    };
    let saved = obj_perm
        .save(db)
        .await
        .map_err(djangors_auth::AuthError::Database)?;
    Ok(saved)
}

/// Revokes an object-level permission `codename` on `(app_label, model_name, object_id)` from `user_id`.
pub async fn revoke_object_permission(
    db: &djangors_db::Database,
    user_id: i64,
    codename: &str,
    app_label: &str,
    model_name: &str,
    object_id: i64,
) -> Result<bool, djangors_auth::AuthError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let p1 = dialect.placeholder(1);
    let sql1 = format!("SELECT id FROM auth_permission WHERE codename = {p1}");
    let params1 = vec![djangors_db::BindValue::Text(codename.to_string())];
    let perm_row = conn
        .fetch_optional(&sql1, &params1)
        .await
        .map_err(|e| djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    let perm_id = match perm_row {
        Some(r) => r
            .try_i64(0)
            .map_err(|e| djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(e)))?,
        None => None,
    };

    let Some(p_id) = perm_id else {
        return Ok(false);
    };

    let p2 = dialect.placeholder(2);
    let p3 = dialect.placeholder(3);
    let p4 = dialect.placeholder(4);
    let p5 = dialect.placeholder(5);
    let sql2 = format!(
        "DELETE FROM djangors_object_permission WHERE \"user\" = {p1} AND permission = {p2} AND app_label = {p3} AND model_name = {p4} AND object_id = {p5}"
    );
    let params2 = vec![
        djangors_db::BindValue::I64(user_id),
        djangors_db::BindValue::I64(p_id),
        djangors_db::BindValue::Text(app_label.to_string()),
        djangors_db::BindValue::Text(model_name.to_string()),
        djangors_db::BindValue::I64(object_id),
    ];
    let rows_affected = conn
        .execute(&sql2, &params2)
        .await
        .map_err(|e| djangors_auth::AuthError::Database(djangors_orm::OrmError::Query(e)))?;

    Ok(rows_affected > 0)
}

#[cfg(test)]
mod tests;
