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
