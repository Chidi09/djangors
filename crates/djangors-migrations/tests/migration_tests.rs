use djangors_db::Database;
use djangors_macros::Model;
use djangors_migrations::migrate;
use djangors_orm::ForeignKey;

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_migrated_parent")]
pub struct ParentModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_migrated_child")]
pub struct ChildModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(on_delete = "cascade", related_name = "children"))]
    pub parent: ForeignKey<ParentModel>,
}

#[tokio::test]
async fn test_migrations_e2e() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = Database::connect(&config).await.expect("Failed to connect");

    // Clean slate
    sqlx::query("DROP TABLE IF EXISTS test_migrated_child")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS test_migrated_parent")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS djangors_migrations")
        .execute(db.pool())
        .await
        .unwrap();

    // Call migrate the first time
    migrate(&db).await.expect("Migration failed");

    // Verify djangors_migrations has "0001_initial"
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM djangors_migrations WHERE name = $1")
        .bind("0001_initial")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(row.0, 1, "0001_initial should be in djangors_migrations");

    // Verify direct insertions and foreign key constraint work
    // 1. Insert parent
    sqlx::query("INSERT INTO test_migrated_parent (name) VALUES ('Parent 1')")
        .execute(db.pool())
        .await
        .unwrap();

    // 2. Insert child referencing parent 1 (id 1)
    sqlx::query("INSERT INTO test_migrated_child (parent) VALUES (1)")
        .execute(db.pool())
        .await
        .unwrap();

    // 3. Try inserting child referencing non-existent parent (id 999), should fail
    let fk_err = sqlx::query("INSERT INTO test_migrated_child (parent) VALUES (999)")
        .execute(db.pool())
        .await;
    assert!(fk_err.is_err(), "Expected foreign key violation error");

    // Call migrate a second time and confirm it is a no-op
    migrate(&db)
        .await
        .expect("Second migration should not error");

    let row2: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM djangors_migrations WHERE name = $1")
        .bind("0001_initial")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        row2.0, 1,
        "Should still have exactly one row in djangors_migrations"
    );
}
