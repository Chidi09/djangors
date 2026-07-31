use djangors_db::BindValue;
use djangors_macros::Model;
use djangors_migrations::migrate;
use djangors_orm::ForeignKey;
use djangors_test::TestDatabase;

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
    let test_db = TestDatabase::connect().await.expect("Failed to connect");
    let db = test_db.database();

    if db.dialect() == djangors_db::Dialect::Sqlite {
        db.conn()
            .execute("PRAGMA foreign_keys = ON;", &[])
            .await
            .ok();
    }

    // Clean slate
    db.conn()
        .execute("DROP TABLE IF EXISTS test_migrated_child", &[])
        .await
        .unwrap();
    db.conn()
        .execute("DROP TABLE IF EXISTS test_migrated_parent", &[])
        .await
        .unwrap();
    db.conn()
        .execute("DROP TABLE IF EXISTS djangors_migrations", &[])
        .await
        .unwrap();

    // Call migrate the first time
    migrate(db).await.expect("Migration failed");

    // Verify djangors_migrations has "0001_initial"
    let ph = db.dialect().placeholder(1);
    let query_sql = format!("SELECT COUNT(*) FROM djangors_migrations WHERE name = {ph}");
    let row = db
        .conn()
        .fetch_one(&query_sql, &[BindValue::Text("0001_initial".to_string())])
        .await
        .unwrap();
    let count = row.try_i64(0).unwrap().unwrap();
    assert_eq!(count, 1, "0001_initial should be in djangors_migrations");

    // Verify direct insertions and foreign key constraint work
    // 1. Insert parent
    db.conn()
        .execute(
            "INSERT INTO test_migrated_parent (name) VALUES ('Parent 1')",
            &[],
        )
        .await
        .unwrap();

    // 2. Insert child referencing parent 1 (id 1)
    db.conn()
        .execute("INSERT INTO test_migrated_child (parent) VALUES (1)", &[])
        .await
        .unwrap();

    // 3. Try inserting child referencing non-existent parent (id 999), should fail
    let fk_err = db
        .conn()
        .execute("INSERT INTO test_migrated_child (parent) VALUES (999)", &[])
        .await;
    assert!(fk_err.is_err(), "Expected foreign key violation error");

    // Call migrate a second time and confirm it is a no-op
    migrate(db)
        .await
        .expect("Second migration should not error");

    let row2 = db
        .conn()
        .fetch_one(&query_sql, &[BindValue::Text("0001_initial".to_string())])
        .await
        .unwrap();
    let count2 = row2.try_i64(0).unwrap().unwrap();
    assert_eq!(
        count2, 1,
        "Should still have exactly one row in djangors_migrations"
    );
}
