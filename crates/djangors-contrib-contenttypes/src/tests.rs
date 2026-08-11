use super::*;
// `objects()` is a `Model` trait method, so the trait has to be in scope to call it here.
use djangors_orm::Model as _;
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
    // Go through TestDatabase rather than a hardcoded Postgres URL. The old version connected
    // to a literal `postgres://postgres:postgres@localhost/djangors_test` and used `BIGSERIAL`
    // plus `Database::pool()`, so it ignored both TEST_BACKEND and DATABASE_URL and could only
    // ever pass against one specific local Postgres instance.
    let Ok(test_db) = djangors_test::TestDatabase::connect().await else {
        return;
    };
    let db = test_db.database().clone();
    let auto_pk = db.dialect().auto_pk_type();
    test_db
        .create_table(&format!(
            "CREATE TABLE IF NOT EXISTS djangors_content_type (id {auto_pk}, app_label VARCHAR(100) NOT NULL, model_name VARCHAR(100) NOT NULL, UNIQUE (app_label, model_name))"
        ))
        .await
        .unwrap();
    ContentType::objects().delete(&db).await.unwrap();

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
    // Counted through the ORM: the raw form used `$1` placeholders and `Database::pool()`,
    // neither of which exists on a SQLite handle.
    let rows = ContentType::objects()
        .filter(djangors_orm::q!(app_label = "contenttypes_tests"))
        .unwrap()
        .filter(djangors_orm::q!(model_name = "Alpha"))
        .unwrap()
        .count(&db)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(alpha.content_type_id, alpha_after.content_type_id);
}
