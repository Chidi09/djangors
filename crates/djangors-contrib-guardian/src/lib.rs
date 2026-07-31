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
mod tests {
    use super::*;
    use djangors_auth::User;
    use std::sync::Mutex;

    static DB_MUTEX: Mutex<()> = Mutex::new(());

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_guardian_has_perm_for_object() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let test_db = djangors_test::TestDatabase::connect().await.unwrap();
        let db = test_db.database();
        let dialect = db.dialect();
        let auto_pk = dialect.auto_pk_type();
        let ts_type = dialect.timestamp_type();

        let drop_tables = [
            "djangors_object_permission",
            "auth_user_permissions",
            "auth_user_groups",
            "auth_group_permissions",
            "auth_group",
            "auth_permission",
            "auth_user",
        ];
        for table in drop_tables {
            let sql = format!("DROP TABLE IF EXISTS {table}");
            let _ = db.conn().execute(&sql, &[]).await;
        }

        let create_user_sql = format!(
            "CREATE TABLE auth_user (
                id {auto_pk},
                username TEXT NOT NULL,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
        );
        db.conn().execute(&create_user_sql, &[]).await.unwrap();

        let create_perm_sql = format!(
            "CREATE TABLE auth_permission (
                id {auto_pk},
                codename TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL
            )"
        );
        db.conn().execute(&create_perm_sql, &[]).await.unwrap();

        let create_user_perms_sql = format!(
            "CREATE TABLE auth_user_permissions (
                id {auto_pk},
                \"user\" BIGINT NOT NULL REFERENCES auth_user(id),
                permission BIGINT NOT NULL REFERENCES auth_permission(id)
            )"
        );
        db.conn()
            .execute(&create_user_perms_sql, &[])
            .await
            .unwrap();

        let create_group_sql = format!(
            "CREATE TABLE auth_group (
                id {auto_pk},
                name TEXT NOT NULL UNIQUE
            )"
        );
        db.conn().execute(&create_group_sql, &[]).await.unwrap();

        let create_user_groups_sql = format!(
            "CREATE TABLE auth_user_groups (
                id {auto_pk},
                \"user\" BIGINT NOT NULL REFERENCES auth_user(id),
                \"group\" BIGINT NOT NULL REFERENCES auth_group(id)
            )"
        );
        db.conn()
            .execute(&create_user_groups_sql, &[])
            .await
            .unwrap();

        let create_group_perms_sql = format!(
            "CREATE TABLE auth_group_permissions (
                id {auto_pk},
                \"group\" BIGINT NOT NULL REFERENCES auth_group(id),
                permission BIGINT NOT NULL REFERENCES auth_permission(id)
            )"
        );
        db.conn()
            .execute(&create_group_perms_sql, &[])
            .await
            .unwrap();

        let create_obj_perm_sql = format!(
            "CREATE TABLE djangors_object_permission (
                id {auto_pk},
                \"user\" BIGINT NOT NULL REFERENCES auth_user(id),
                permission BIGINT NOT NULL REFERENCES auth_permission(id),
                app_label TEXT NOT NULL,
                model_name TEXT NOT NULL,
                object_id BIGINT NOT NULL
            )"
        );
        db.conn().execute(&create_obj_perm_sql, &[]).await.unwrap();

        let now = chrono::Utc::now();
        let user1 = User {
            id: 0,
            username: "guardian_u1".to_string(),
            email: "g1@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(db)
        .await
        .unwrap();

        let user2 = User {
            id: 0,
            username: "guardian_u2".to_string(),
            email: "g2@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(db)
        .await
        .unwrap();

        let perm1 = djangors_auth::Permission {
            id: 0,
            codename: "change_document".to_string(),
            name: "Can change document".to_string(),
        }
        .save(db)
        .await
        .unwrap();

        // 1. Model-level grant alone gives true for user1
        let insert_user_perm_sql = format!(
            "INSERT INTO auth_user_permissions (\"user\", permission) VALUES ({}, {})",
            dialect.placeholder(1),
            dialect.placeholder(2)
        );
        db.conn()
            .execute(
                &insert_user_perm_sql,
                &[
                    djangors_db::BindValue::I64(user1.id),
                    djangors_db::BindValue::I64(perm1.id),
                ],
            )
            .await
            .unwrap();

        let has_perm_u1 =
            has_perm_for_object(db, user1.id, "change_document", "docs", "Document", 100)
                .await
                .unwrap();
        assert!(
            has_perm_u1,
            "User1 has model-level permission so has_perm_for_object must return true"
        );

        // 2. User2 has NO model-level perm initially
        let has_perm_u2_before =
            has_perm_for_object(db, user2.id, "change_document", "docs", "Document", 100)
                .await
                .unwrap();
        assert!(!has_perm_u2_before, "User2 has no permissions initially");

        // 3. Grant object-specific perm for user2 on object 100
        grant_object_permission(db, user2.id, "change_document", "docs", "Document", 100)
            .await
            .unwrap();

        let has_perm_u2_obj100 =
            has_perm_for_object(db, user2.id, "change_document", "docs", "Document", 100)
                .await
                .unwrap();
        assert!(
            has_perm_u2_obj100,
            "User2 has object permission for object 100"
        );

        // 4. User2 on a DIFFERENT object id (200) should be FALSE
        let has_perm_u2_obj200 =
            has_perm_for_object(db, user2.id, "change_document", "docs", "Document", 200)
                .await
                .unwrap();
        assert!(
            !has_perm_u2_obj200,
            "User2 object perm for object 100 does not grant perm for object 200"
        );

        // 5. Test revoke_object_permission
        let revoked =
            revoke_object_permission(db, user2.id, "change_document", "docs", "Document", 100)
                .await
                .unwrap();
        assert!(revoked, "Revoking object perm returned true");

        let has_perm_u2_after_revoke =
            has_perm_for_object(db, user2.id, "change_document", "docs", "Document", 100)
                .await
                .unwrap();
        assert!(!has_perm_u2_after_revoke, "User2 has no perm after revoke");

        for table in drop_tables {
            let sql = format!("DROP TABLE IF EXISTS {table}");
            let _ = db.conn().execute(&sql, &[]).await;
        }
    }
}
