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

    let has_perm_u1 = has_perm_for_object(db, user1.id, "change_document", "docs", "Document", 100)
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
