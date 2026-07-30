use crate::meta::{
    all_registered_models, DefaultValue, FieldKind, FieldMeta, ForeignKey, Model, ModelMeta,
    ModelRegistration, OnDelete, RelationKind, RelationMeta,
};

pub struct FictionalModel;

impl Model for FictionalModel {
    fn meta() -> &'static ModelMeta {
        static META: std::sync::OnceLock<ModelMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ModelMeta {
            struct_name: "FictionalModel",
            app_label: "test_app",
            table_name: "test_app_fictionalmodel",
            fields: &[
                FieldMeta {
                    name: "id",
                    column_name: "id",
                    kind: FieldKind::BigInt,
                    nullable: false,
                    primary_key: true,
                    auto: true,
                    unique: true,
                    db_index: true,
                    default: DefaultValue::None,
                    max_length: None,
                    verbose_name: None,
                    help_text: None,
                    choices: &[],
                },
                FieldMeta {
                    name: "name",
                    column_name: "name",
                    kind: FieldKind::Char,
                    nullable: false,
                    primary_key: false,
                    auto: false,
                    unique: false,
                    db_index: false,
                    default: DefaultValue::Text("unnamed"),
                    max_length: Some(100),
                    verbose_name: Some("Name"),
                    help_text: Some("Enter the name"),
                    choices: &[],
                },
            ],
            relations: &[],
            indexes: &[],
            unique_together: &[],
            ordering: &[],
        })
    }

    fn field_values(&self) -> Vec<(&'static str, crate::expr::Value)> {
        vec![
            ("id", crate::expr::Value::I64(1)),
            ("name", crate::expr::Value::Text("test".to_string())),
        ]
    }

    fn field_names() -> Vec<&'static str> {
        vec!["id", "name"]
    }
}

pub struct RelatedModel;

impl Model for RelatedModel {
    fn meta() -> &'static ModelMeta {
        static META: std::sync::OnceLock<ModelMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ModelMeta {
            struct_name: "RelatedModel",
            app_label: "test_app",
            table_name: "test_app_relatedmodel",
            fields: &[FieldMeta {
                name: "id",
                column_name: "id",
                kind: FieldKind::BigInt,
                nullable: false,
                primary_key: true,
                auto: true,
                unique: true,
                db_index: true,
                default: DefaultValue::None,
                max_length: None,
                verbose_name: None,
                help_text: None,
                choices: &[],
            }],
            relations: &[RelationMeta {
                field_name: "fictional",
                kind: RelationKind::ForeignKey,
                target: FictionalModel::meta,
                on_delete: OnDelete::Cascade,
                related_name: Some("related_instances"),
            }],
            indexes: &[],
            unique_together: &[],
            ordering: &[],
        })
    }

    fn field_values(&self) -> Vec<(&'static str, crate::expr::Value)> {
        vec![
            ("id", crate::expr::Value::I64(1)),
            ("fictional", crate::expr::Value::I64(2)),
        ]
    }

    fn field_names() -> Vec<&'static str> {
        vec!["id", "fictional"]
    }
}

// Register FictionalModel and RelatedModel for testing inventory iteration
inventory::submit! {
    ModelRegistration {
        meta_fn: FictionalModel::meta,
    }
}

inventory::submit! {
    ModelRegistration {
        meta_fn: RelatedModel::meta,
    }
}

#[test]
fn test_manual_model_meta_reading() {
    let meta = FictionalModel::meta();
    assert_eq!(meta.struct_name, "FictionalModel");
    assert_eq!(meta.app_label, "test_app");
    assert_eq!(meta.table_name, "test_app_fictionalmodel");

    assert_eq!(meta.fields.len(), 2);
    let id_field = meta.fields[0];
    assert_eq!(id_field.name, "id");
    assert_eq!(id_field.kind, FieldKind::BigInt);
    assert!(id_field.primary_key);
    assert_eq!(id_field.default, DefaultValue::None);

    let name_field = meta.fields[1];
    assert_eq!(name_field.name, "name");
    assert_eq!(name_field.kind, FieldKind::Char);
    assert_eq!(name_field.default, DefaultValue::Text("unnamed"));
    assert_eq!(name_field.max_length, Some(100));

    let related_meta = RelatedModel::meta();
    assert_eq!(related_meta.relations.len(), 1);
    let relation = &related_meta.relations[0];
    assert_eq!(relation.field_name, "fictional");
    assert_eq!(relation.kind, RelationKind::ForeignKey);
    assert_eq!(relation.on_delete, OnDelete::Cascade);
    assert_eq!(relation.related_name, Some("related_instances"));

    // Verify resolving target returns FictionalModel's meta
    let target_meta = (relation.target)();
    assert_eq!(target_meta.struct_name, "FictionalModel");
}

#[test]
fn test_inventory_registration() {
    let registered: Vec<&'static ModelMeta> = all_registered_models().collect();

    // Verify FictionalModel is present
    let fictional_found = registered
        .iter()
        .any(|m| m.struct_name == "FictionalModel" && m.app_label == "test_app");
    assert!(
        fictional_found,
        "FictionalModel should be found in registered models"
    );

    // Verify RelatedModel is present
    let related_found = registered
        .iter()
        .any(|m| m.struct_name == "RelatedModel" && m.app_label == "test_app");
    assert!(
        related_found,
        "RelatedModel should be found in registered models"
    );
}

#[test]
fn test_foreign_key_clone_copy() {
    let fk = ForeignKey::<FictionalModel>::new(42);
    assert_eq!(fk.id, 42);

    // Verify copy semantics
    let fk_copy = fk;
    let fk_copy2 = fk;
    assert_eq!(fk_copy.id, 42);
    assert_eq!(fk_copy2.id, 42);

    // Verify debug print format
    let debug_str = format!("{:?}", fk);
    assert!(debug_str.contains("ForeignKey"));
    assert!(debug_str.contains("id: 42"));
}

use crate::q;
use djangors_macros::Model;

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_queryset_model")]
#[allow(dead_code)]
pub struct QuerySetTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub name: String,

    pub is_active: bool,
}

#[tokio::test]
async fn test_queryset_operations() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    // Create table
    sqlx::query("DROP TABLE IF EXISTS test_queryset_model")
        .execute(db.pool())
        .await
        .unwrap();

    sqlx::query(
        "CREATE TABLE test_queryset_model (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL,
            is_active BOOLEAN NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // 1. Test empty queryset
    let count = QuerySetTestModel::objects().count(&db).await.unwrap();
    assert_eq!(count, 0);

    let exists = QuerySetTestModel::objects().exists(&db).await.unwrap();
    assert!(!exists);

    let first = QuerySetTestModel::objects().first(&db).await.unwrap();
    assert!(first.is_none());

    let get_res = QuerySetTestModel::objects().get(&db).await;
    assert!(matches!(
        get_res,
        Err(crate::error::OrmError::NotFound { .. })
    ));

    // 2. Insert out-of-order test data
    sqlx::query("INSERT INTO test_queryset_model (name, is_active) VALUES ('Alice', true), ('Charlie', false), ('Bob', true)")
        .execute(db.pool())
        .await
        .unwrap();

    // 3. Test basic operations: count, exists, first, all
    assert_eq!(QuerySetTestModel::objects().count(&db).await.unwrap(), 3);
    assert!(QuerySetTestModel::objects().exists(&db).await.unwrap());

    // 4. Test ordering (ascending name)
    let sorted_asc = QuerySetTestModel::objects()
        .order_by("name")
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(sorted_asc.len(), 3);
    assert_eq!(sorted_asc[0].name, "Alice");
    assert_eq!(sorted_asc[1].name, "Bob");
    assert_eq!(sorted_asc[2].name, "Charlie");

    // Test ordering (descending name)
    let sorted_desc = QuerySetTestModel::objects()
        .order_by("-name")
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(sorted_desc.len(), 3);
    assert_eq!(sorted_desc[0].name, "Charlie");
    assert_eq!(sorted_desc[1].name, "Bob");
    assert_eq!(sorted_desc[2].name, "Alice");

    // 5. Test filter: exact match
    let active_qs = QuerySetTestModel::objects()
        .filter(q!(is_active = true))
        .unwrap();
    assert_eq!(active_qs.count(&db).await.unwrap(), 2);
    let active_rows = active_qs.order_by("name").unwrap().all(&db).await.unwrap();
    assert_eq!(active_rows[0].name, "Alice");
    assert_eq!(active_rows[1].name, "Bob");

    // Test filter: contains lookup
    let ch_qs = QuerySetTestModel::objects()
        .filter(q!(name__contains = "ch"))
        .unwrap();
    assert_eq!(ch_qs.count(&db).await.unwrap(), 0); // case sensitive
    let ich_qs = QuerySetTestModel::objects()
        .filter(q!(name__icontains = "ch"))
        .unwrap();
    assert_eq!(ich_qs.count(&db).await.unwrap(), 1);
    assert_eq!(ich_qs.get(&db).await.unwrap().name, "Charlie");

    // Test filter: starts with
    let starts_qs = QuerySetTestModel::objects()
        .filter(q!(name__startswith = "Al"))
        .unwrap();
    assert_eq!(starts_qs.get(&db).await.unwrap().name, "Alice");

    // Test filter: ends with
    let ends_qs = QuerySetTestModel::objects()
        .filter(q!(name__endswith = "ob"))
        .unwrap();
    assert_eq!(ends_qs.get(&db).await.unwrap().name, "Bob");

    // 6. Test limit & offset
    let limit_qs = QuerySetTestModel::objects()
        .order_by("name")
        .unwrap()
        .limit(2);
    let limit_rows = limit_qs.all(&db).await.unwrap();
    assert_eq!(limit_rows.len(), 2);
    assert_eq!(limit_rows[0].name, "Alice");
    assert_eq!(limit_rows[1].name, "Bob");

    let offset_qs = QuerySetTestModel::objects()
        .order_by("name")
        .unwrap()
        .offset(1);
    let offset_rows = offset_qs.all(&db).await.unwrap();
    assert_eq!(offset_rows.len(), 2);
    assert_eq!(offset_rows[0].name, "Bob");
    assert_eq!(offset_rows[1].name, "Charlie");

    // 7. Test get multiple error
    let multi_get = QuerySetTestModel::objects()
        .filter(q!(is_active = true))
        .unwrap()
        .get(&db)
        .await;
    assert!(matches!(
        multi_get,
        Err(crate::error::OrmError::MultipleObjectsReturned { .. })
    ));

    // 8. Test typo'd field name validation in filter()
    let typo_res = QuerySetTestModel::objects().filter(q!(name_typo = "val"));
    match typo_res {
        Err(crate::error::OrmError::FieldNotFound { field, .. }) => {
            assert_eq!(field, "name_typo");
        }
        other => panic!("expected FieldNotFound, got {other:?}"),
    }

    // 9. Test typo'd field name validation in order_by() — this is a real SQL
    // injection guard, not just a nice error message: without it, an
    // unvalidated field name would be interpolated directly into the ORDER BY
    // clause. Confirms the fix that closed that gap actually rejects unknown
    // fields instead of silently passing them through as raw SQL text.
    let order_typo_res =
        QuerySetTestModel::objects().order_by("'; DROP TABLE test_queryset_model; --");
    match order_typo_res {
        Err(crate::error::OrmError::FieldNotFound { field, .. }) => {
            assert_eq!(field, "'; DROP TABLE test_queryset_model; --");
        }
        other => panic!("expected FieldNotFound, got {other:?}"),
    }
    // Confirm the table really is still there (the rejected order_by must not
    // have executed any SQL at all).
    assert_eq!(QuerySetTestModel::objects().count(&db).await.unwrap(), 3);

    // 10. Test model lifecycle (save, update, delete)
    let initial = QuerySetTestModel {
        id: 0,
        name: "David".to_string(),
        is_active: true,
    };
    let saved = initial.save(&db).await.unwrap();
    assert_ne!(saved.id, 0);
    assert_eq!(saved.name, "David");
    assert!(saved.is_active);

    // Confirm it exists in DB directly
    let db_count = sqlx::query("SELECT COUNT(*) FROM test_queryset_model WHERE id = $1 AND name = 'David' AND is_active = true")
        .bind(saved.id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    use sqlx::Row;
    let count: i64 = db_count.try_get(0).unwrap();
    assert_eq!(count, 1);

    // 11. Test update()
    let mut to_update = saved;
    to_update.name = "David Updated".to_string();
    to_update.update(&db).await.unwrap();

    // Re-fetch via QuerySet
    let fetched = QuerySetTestModel::objects()
        .filter(q!(id = to_update.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(fetched.name, "David Updated");

    // 12. Test update() on non-existent PK
    let non_existent = QuerySetTestModel {
        id: 999999,
        name: "Ghost".to_string(),
        is_active: false,
    };
    let update_res = non_existent.update(&db).await;
    assert!(matches!(
        update_res,
        Err(crate::error::OrmError::NotFound { .. })
    ));

    // 13. Test delete()
    assert_eq!(QuerySetTestModel::objects().count(&db).await.unwrap(), 4);
    fetched.delete(&db).await.unwrap();
    assert_eq!(QuerySetTestModel::objects().count(&db).await.unwrap(), 3);

    let get_deleted = QuerySetTestModel::objects()
        .filter(q!(id = fetched.id))
        .unwrap()
        .get(&db)
        .await;
    assert!(matches!(
        get_deleted,
        Err(crate::error::OrmError::NotFound { .. })
    ));

    // 14. Test delete() on non-existent PK
    let delete_res = non_existent.delete(&db).await;
    assert!(matches!(
        delete_res,
        Err(crate::error::OrmError::NotFound { .. })
    ));

    // Cleanup table
    sqlx::query("DROP TABLE test_queryset_model")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_aggregate_model")]
#[allow(dead_code)]
pub struct AggregateTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub category: String,

    pub score: i32,
}

#[tokio::test]
async fn test_queryset_aggregation() {
    use crate::aggregate::{AggExpr, AggResult};

    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_aggregate_model")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE test_aggregate_model (
            id BIGSERIAL PRIMARY KEY,
            category TEXT NOT NULL,
            score INTEGER NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO test_aggregate_model (category, score) VALUES
            ('a', 10), ('a', 20), ('a', 30), ('b', 100)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // COUNT(*) via aggregate() must match the existing .count() method.
    let count_via_count = AggregateTestModel::objects().count(&db).await.unwrap();
    let count_via_agg = AggregateTestModel::objects()
        .aggregate(&db, vec![AggExpr::Count { field: "*" }])
        .await
        .unwrap();
    assert_eq!(count_via_agg, vec![AggResult::I64(4)]);
    assert_eq!(count_via_count, 4);

    // SUM/AVG/MIN/MAX over the whole table.
    let results = AggregateTestModel::objects()
        .aggregate(
            &db,
            vec![
                AggExpr::Sum { field: "score" },
                AggExpr::Avg { field: "score" },
                AggExpr::Min { field: "score" },
                AggExpr::Max { field: "score" },
            ],
        )
        .await
        .unwrap();
    assert_eq!(results[0], AggResult::F64(160.0)); // SUM: 10+20+30+100
    assert_eq!(results[1], AggResult::F64(40.0)); // AVG: 160/4
    assert_eq!(results[2], AggResult::F64(10.0)); // MIN
    assert_eq!(results[3], AggResult::F64(100.0)); // MAX

    // Aggregation composed with filter() — only category 'a' rows.
    let filtered_sum = AggregateTestModel::objects()
        .filter(q!(category = "a"))
        .unwrap()
        .aggregate(&db, vec![AggExpr::Sum { field: "score" }])
        .await
        .unwrap();
    assert_eq!(filtered_sum, vec![AggResult::F64(60.0)]); // 10+20+30, not +100

    // SUM/AVG/MIN/MAX over zero matching rows -> Null, not an error.
    let empty_results = AggregateTestModel::objects()
        .filter(q!(category = "nonexistent"))
        .unwrap()
        .aggregate(
            &db,
            vec![
                AggExpr::Sum { field: "score" },
                AggExpr::Avg { field: "score" },
                AggExpr::Min { field: "score" },
                AggExpr::Max { field: "score" },
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        empty_results,
        vec![
            AggResult::Null,
            AggResult::Null,
            AggResult::Null,
            AggResult::Null
        ]
    );
    // COUNT over zero matching rows is 0, never Null.
    let empty_count = AggregateTestModel::objects()
        .filter(q!(category = "nonexistent"))
        .unwrap()
        .aggregate(&db, vec![AggExpr::Count { field: "*" }])
        .await
        .unwrap();
    assert_eq!(empty_count, vec![AggResult::I64(0)]);

    // Typo'd field name is rejected before any SQL executes — the same
    // injection-safety discipline as filter()/order_by().
    let typo_res = AggregateTestModel::objects()
        .aggregate(&db, vec![AggExpr::Sum { field: "scoer" }])
        .await;
    match typo_res {
        Err(crate::error::OrmError::FieldNotFound { field, .. }) => {
            assert_eq!(field, "scoer");
        }
        other => panic!("expected FieldNotFound, got {other:?}"),
    }

    sqlx::query("DROP TABLE test_aggregate_model")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_bulk_update_model")]
#[allow(dead_code)]
pub struct BulkUpdateTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub votes: i64,

    pub price: f64,

    pub is_active: bool,
}

#[tokio::test]
async fn test_queryset_bulk_update() {
    use crate::expr::F;
    use crate::set;

    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    // Create table
    sqlx::query("DROP TABLE IF EXISTS test_bulk_update_model")
        .execute(db.pool())
        .await
        .unwrap();

    sqlx::query(
        "CREATE TABLE test_bulk_update_model (
            id BIGSERIAL PRIMARY KEY,
            votes BIGINT NOT NULL,
            price DOUBLE PRECISION NOT NULL,
            is_active BOOLEAN NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // Insert initial rows
    sqlx::query(
        "INSERT INTO test_bulk_update_model (votes, price, is_active) VALUES
            (10, 1.5, true),
            (20, 2.5, true),
            (30, 3.5, false)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // Test 1: Literal and F() expression updates on filtered queryset
    // votes = votes + 5, price = price * 2.0, is_active = false
    let affected = BulkUpdateTestModel::objects()
        .filter(q!(is_active = true))
        .unwrap()
        .update(
            &db,
            set!(
                votes = F("votes") + 5,
                price = F("price") * 2.0,
                is_active = false
            ),
        )
        .await
        .unwrap();

    assert_eq!(affected, 2);

    // Verify row 1
    let row1 = BulkUpdateTestModel::objects()
        .filter(q!(id = 1))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(row1.votes, 15);
    assert_eq!(row1.price, 3.0);
    assert!(!row1.is_active);

    // Verify row 2
    let row2 = BulkUpdateTestModel::objects()
        .filter(q!(id = 2))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(row2.votes, 25);
    assert_eq!(row2.price, 5.0);
    assert!(!row2.is_active);

    // Verify row 3 (unaffected because it was not active)
    let row3 = BulkUpdateTestModel::objects()
        .filter(q!(id = 3))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(row3.votes, 30);
    assert_eq!(row3.price, 3.5);
    assert!(!row3.is_active);

    // Test 2: Double sequential increment
    let affected_seq = BulkUpdateTestModel::objects()
        .filter(q!(id = 1))
        .unwrap()
        .update(&db, set!(votes = F("votes") + 1))
        .await
        .unwrap();
    assert_eq!(affected_seq, 1);

    let affected_seq2 = BulkUpdateTestModel::objects()
        .filter(q!(id = 1))
        .unwrap()
        .update(&db, set!(votes = F("votes") + 1))
        .await
        .unwrap();
    assert_eq!(affected_seq2, 1);

    let row1_double = BulkUpdateTestModel::objects()
        .filter(q!(id = 1))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(row1_double.votes, 17); // 15 + 1 + 1

    // Test 3: Concurrent updates using tokio::join!
    let qs1 = BulkUpdateTestModel::objects().filter(q!(id = 2)).unwrap();
    let qs2 = BulkUpdateTestModel::objects().filter(q!(id = 2)).unwrap();
    let fut1 = qs1.update(&db, set!(votes = F("votes") + 1));
    let fut2 = qs2.update(&db, set!(votes = F("votes") + 1));
    let (res1, res2) = tokio::join!(fut1, fut2);
    assert_eq!(res1.unwrap(), 1);
    assert_eq!(res2.unwrap(), 1);

    let row2_concurrent = BulkUpdateTestModel::objects()
        .filter(q!(id = 2))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(row2_concurrent.votes, 27); // 25 + 1 + 1

    // Test 4: Typo'd field name (LHS) is rejected
    let typo_lhs = BulkUpdateTestModel::objects()
        .update(&db, set!(votes_typo = 5))
        .await;
    assert!(matches!(
        typo_lhs,
        Err(crate::error::OrmError::FieldNotFound { .. })
    ));

    // Test 5: Typo'd field name (RHS F()) is rejected
    let typo_rhs = BulkUpdateTestModel::objects()
        .update(&db, set!(votes = F("votes_typo") + 1))
        .await;
    assert!(matches!(
        typo_rhs,
        Err(crate::error::OrmError::FieldNotFound { .. })
    ));

    // Test 6: Bulk update matching zero rows returns Ok(0)
    let affected_zero = BulkUpdateTestModel::objects()
        .filter(q!(id = 9999))
        .unwrap()
        .update(&db, set!(votes = F("votes") + 1))
        .await
        .unwrap();
    assert_eq!(affected_zero, 0);

    // Clean up
    sqlx::query("DROP TABLE test_bulk_update_model")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_select_related_parent")]
pub struct SelectRelatedParent {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_select_related_child")]
pub struct SelectRelatedChild {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(on_delete = "cascade", related_name = "children"))]
    pub parent: ForeignKey<SelectRelatedParent>,
}

#[tokio::test]
async fn test_queryset_select_related() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    // Clean up tables if they exist
    sqlx::query("DROP TABLE IF EXISTS test_select_related_child")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS test_select_related_parent")
        .execute(db.pool())
        .await
        .unwrap();

    // Create parent table
    sqlx::query(
        "CREATE TABLE test_select_related_parent (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // Create child table
    sqlx::query(
        "CREATE TABLE test_select_related_child (
            id BIGSERIAL PRIMARY KEY,
            parent BIGINT NOT NULL REFERENCES test_select_related_parent(id) ON DELETE CASCADE
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // Insert test data
    let parent1 = SelectRelatedParent {
        id: 0,
        name: "Parent A".to_string(),
    }
    .save(&db)
    .await
    .unwrap();

    let parent2 = SelectRelatedParent {
        id: 0,
        name: "Parent B".to_string(),
    }
    .save(&db)
    .await
    .unwrap();

    let child1 = SelectRelatedChild {
        id: 0,
        parent: ForeignKey::new(parent1.id),
    }
    .save(&db)
    .await
    .unwrap();

    let child2 = SelectRelatedChild {
        id: 0,
        parent: ForeignKey::new(parent2.id),
    }
    .save(&db)
    .await
    .unwrap();

    // Run select_related
    let results = SelectRelatedChild::objects()
        .select_related::<SelectRelatedParent, _>(&db, "parent")
        .await
        .unwrap();

    assert_eq!(results.len(), 2);

    // Verify child1 points to parent1
    let (c1, p1_opt) = results.iter().find(|(c, _)| c.id == child1.id).unwrap();
    assert_eq!(c1.parent.id, parent1.id);
    let p1 = p1_opt.as_ref().unwrap();
    assert_eq!(p1.id, parent1.id);
    assert_eq!(p1.name, "Parent A");

    // Verify child2 points to parent2
    let (c2, p2_opt) = results.iter().find(|(c, _)| c.id == child2.id).unwrap();
    assert_eq!(c2.parent.id, parent2.id);
    let p2 = p2_opt.as_ref().unwrap();
    assert_eq!(p2.id, parent2.id);
    assert_eq!(p2.name, "Parent B");

    // Test validation: typo'd field name
    let typo_res = SelectRelatedChild::objects()
        .select_related::<SelectRelatedParent, _>(&db, "nonexistent")
        .await;
    assert!(matches!(
        typo_res,
        Err(crate::error::OrmError::FieldNotFound { .. })
    ));

    // Test validation: mismatched R type parameter
    let mismatch_res = SelectRelatedChild::objects()
        .select_related::<SelectRelatedChild, _>(&db, "parent")
        .await;
    assert!(matches!(
        mismatch_res,
        Err(crate::error::OrmError::FieldNotFound { .. })
    ));

    // Clean up
    sqlx::query("DROP TABLE test_select_related_child")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("DROP TABLE test_select_related_parent")
        .execute(db.pool())
        .await
        .unwrap();
}

// Dedicated models/tables for the prefetch test below, kept separate from
// `SelectRelatedParent`/`SelectRelatedChild` above: reusing those tables led to a real race,
// since Rust tests in this file run concurrently against the same live database, and
// `test_queryset_select_related` drops its own tables partway through the run.
#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_prefetch_parent")]
pub struct PrefetchParent {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_prefetch_child")]
pub struct PrefetchChild {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(on_delete = "cascade", related_name = "children"))]
    pub parent: ForeignKey<PrefetchParent>,
}

#[tokio::test]
async fn test_prefetch_related_is_constant_query_count() {
    let config = djangors_db::config::DatabaseConfig::new(
        "postgres://postgres:postgres@localhost/djangors_test",
    );
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_prefetch_child")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS test_prefetch_parent")
        .execute(db.pool())
        .await
        .unwrap();

    sqlx::query(
        "CREATE TABLE test_prefetch_parent (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE test_prefetch_child (
            id BIGSERIAL PRIMARY KEY,
            parent BIGINT NOT NULL REFERENCES test_prefetch_parent(id) ON DELETE CASCADE
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let seed_parents: Vec<_> = (0..5)
        .map(|i| PrefetchParent {
            id: 0,
            name: format!("Prefetch {i}"),
        })
        .collect();
    for parent in &seed_parents {
        parent.clone().save(&db).await.unwrap();
    }
    let parents = PrefetchParent::objects().all(&db).await.unwrap();
    for parent in &parents {
        for _ in 0..2 {
            PrefetchChild {
                id: 0,
                parent: ForeignKey::new(parent.id),
            }
            .save(&db)
            .await
            .unwrap();
        }
    }

    db.reset_query_count();
    let parents = PrefetchParent::objects().all(&db).await.unwrap();
    let grouped =
        crate::prefetch_related::<PrefetchParent, PrefetchChild, _>(&db, &parents, "children")
            .await
            .unwrap();
    assert_eq!(db.query_count(), 2);
    assert_eq!(grouped.len(), 5);
    assert!(parents
        .iter()
        .all(|p| grouped[&p.id].len() == 2 && grouped[&p.id].iter().all(|c| c.parent.id == p.id)));

    db.reset_query_count();
    for parent in &parents {
        PrefetchChild::objects()
            .filter(q!(parent = parent.id))
            .unwrap()
            .all(&db)
            .await
            .unwrap();
    }
    assert_eq!(db.query_count(), 5);

    sqlx::query("DROP TABLE test_prefetch_child")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("DROP TABLE test_prefetch_parent")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_field_values_parent")]
pub struct FieldValuesParent {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_field_values_model")]
#[allow(dead_code)]
pub struct FieldValuesModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub title: String,
    pub optional_text: Option<String>,
    pub optional_num: Option<i32>,
    #[djangors(foreign_key(on_delete = "cascade"))]
    pub parent: ForeignKey<FieldValuesParent>,
}

#[test]
fn test_value_display() {
    use crate::expr::Value;
    use chrono::{TimeZone, Utc};

    assert_eq!(Value::I64(42).to_string(), "42");
    assert_eq!(Value::F64(1.23).to_string(), "1.23");
    assert_eq!(Value::Bool(true).to_string(), "true");
    assert_eq!(Value::Bool(false).to_string(), "false");
    assert_eq!(Value::Text("hello".to_string()).to_string(), "hello");
    assert_eq!(Value::Null.to_string(), "-");

    let dt = Utc.with_ymd_and_hms(2026, 7, 17, 22, 27, 47).unwrap();
    assert_eq!(Value::DateTime(dt).to_string(), "2026-07-17 22:27:47");
}

#[test]
fn test_model_field_values_and_names() {
    let parent = FieldValuesParent {
        id: 10,
        name: "Parent Name".to_string(),
    };

    let model_some = FieldValuesModel {
        id: 42,
        title: "Test Title".to_string(),
        optional_text: Some("hello".to_string()),
        optional_num: Some(100),
        parent: ForeignKey::new(parent.id),
    };

    let values_some = model_some.field_values();
    assert_eq!(values_some.len(), 5);
    assert_eq!(values_some[0].0, "id");
    assert_eq!(values_some[0].1, crate::expr::Value::I64(42));
    assert_eq!(values_some[1].0, "title");
    assert_eq!(
        values_some[1].1,
        crate::expr::Value::Text("Test Title".to_string())
    );
    assert_eq!(values_some[2].0, "optional_text");
    assert_eq!(
        values_some[2].1,
        crate::expr::Value::Text("hello".to_string())
    );
    assert_eq!(values_some[3].0, "optional_num");
    assert_eq!(values_some[3].1, crate::expr::Value::I64(100));
    assert_eq!(values_some[4].0, "parent");
    assert_eq!(values_some[4].1, crate::expr::Value::I64(10));

    let model_none = FieldValuesModel {
        id: 43,
        title: "Test Title 2".to_string(),
        optional_text: None,
        optional_num: None,
        parent: ForeignKey::new(parent.id),
    };

    let values_none = model_none.field_values();
    assert_eq!(values_none[2].0, "optional_text");
    assert_eq!(values_none[2].1, crate::expr::Value::Null);
    assert_eq!(values_none[3].0, "optional_num");
    assert_eq!(values_none[3].1, crate::expr::Value::Null);

    let names = FieldValuesModel::field_names();
    assert_eq!(
        names,
        vec!["id", "title", "optional_text", "optional_num", "parent"]
    );
}

#[derive(Model, Debug)]
#[djangors(
    app = "test_app",
    table_name = "test_ordered_agg_model",
    ordering = "-name"
)]
#[allow(dead_code)]
pub struct OrderedAggModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

/// Regression test: `count()`, `exists()`, and `aggregate()` on a model with
/// default `meta.ordering` used to emit `SELECT COUNT(*) ... ORDER BY name`,
/// which Postgres rejects ("must appear in the GROUP BY clause") — clearing
/// `order_by` on a clone wasn't enough because an empty `order_by` falls back
/// to `meta.ordering` during SQL compilation.
#[tokio::test]
async fn test_count_and_aggregate_on_model_with_default_ordering() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_ordered_agg_model")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE test_ordered_agg_model (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    sqlx::query("INSERT INTO test_ordered_agg_model (name) VALUES ('a'), ('b'), ('c')")
        .execute(db.pool())
        .await
        .unwrap();

    assert_eq!(OrderedAggModel::objects().count(&db).await.unwrap(), 3);
    assert!(OrderedAggModel::objects().exists(&db).await.unwrap());

    let aggs = OrderedAggModel::objects()
        .aggregate(&db, vec![crate::aggregate::AggExpr::Count { field: "*" }])
        .await
        .unwrap();
    assert_eq!(aggs.len(), 1);

    // The default ordering itself must still apply to regular selects.
    let rows = OrderedAggModel::objects().all(&db).await.unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["c", "b", "a"]);

    sqlx::query("DROP TABLE test_ordered_agg_model")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_insert_raw_model")]
#[allow(dead_code)]
pub struct InsertRawTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    pub age: i64,
}

#[tokio::test]
async fn test_queryset_insert_raw() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_insert_raw_model")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE test_insert_raw_model (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL,
            age BIGINT NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // 1. Valid insert_raw
    let values = vec![
        ("name", crate::expr::Value::Text("Alice".to_string())),
        ("age", crate::expr::Value::I64(30)),
    ];
    let pk = crate::queryset::QuerySet::<InsertRawTestModel>::insert_raw(&db, values)
        .await
        .unwrap();

    // Verify it exists in DB via typed get()
    let row = crate::queryset::QuerySet::<InsertRawTestModel>::new()
        .filter(q!(id = pk))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(row.name, "Alice");
    assert_eq!(row.age, 30);

    // 2. Reject unknown field name
    let bad_values = vec![
        ("name", crate::expr::Value::Text("Bob".to_string())),
        ("invalid_field", crate::expr::Value::I64(42)),
    ];
    let err = crate::queryset::QuerySet::<InsertRawTestModel>::insert_raw(&db, bad_values).await;
    match err {
        Err(crate::error::OrmError::FieldNotFound { field, .. }) => {
            assert_eq!(field, "invalid_field");
        }
        other => panic!("Expected FieldNotFound error, got {:?}", other),
    }

    sqlx::query("DROP TABLE test_insert_raw_model")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_bulk_create_model")]
#[allow(dead_code)]
pub struct BulkCreateTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    pub age: i64,
}

#[tokio::test]
async fn test_queryset_bulk_create() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_bulk_create_model")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE test_bulk_create_model (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL,
            age BIGINT NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // Empty input is a no-op, not an error, and issues no query.
    let empty: Vec<BulkCreateTestModel> = Vec::new();
    let pks = crate::queryset::QuerySet::<BulkCreateTestModel>::bulk_create(&db, &empty)
        .await
        .unwrap();
    assert!(pks.is_empty());

    // A real multi-row insert in one statement.
    let items = vec![
        BulkCreateTestModel {
            id: 0,
            name: "Alice".to_string(),
            age: 30,
        },
        BulkCreateTestModel {
            id: 0,
            name: "Bob".to_string(),
            age: 25,
        },
        BulkCreateTestModel {
            id: 0,
            name: "Carol".to_string(),
            age: 40,
        },
    ];
    let pks = crate::queryset::QuerySet::<BulkCreateTestModel>::bulk_create(&db, &items)
        .await
        .unwrap();
    assert_eq!(pks.len(), 3);
    // Every generated pk must be distinct - a real 3-row insert, not the same row 3 times.
    let mut sorted_pks = pks.clone();
    sorted_pks.sort_unstable();
    sorted_pks.dedup();
    assert_eq!(sorted_pks.len(), 3);

    // Every row must genuinely exist with the right data, in the same order as the input.
    for (pk, expected) in pks.iter().zip(items.iter()) {
        let row = crate::queryset::QuerySet::<BulkCreateTestModel>::new()
            .filter(q!(id = *pk))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        assert_eq!(row.name, expected.name);
        assert_eq!(row.age, expected.age);
    }

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_bulk_create_model")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        total, 3,
        "bulk_create must not insert more or fewer rows than given"
    );

    sqlx::query("DROP TABLE test_bulk_create_model")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_nullable_bind_model")]
#[allow(dead_code)]
pub struct NullableBindTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    pub bio: Option<String>,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
}

/// Regression test: `insert_raw` and `update`'s bind loops used to type
/// every `Value::Null` as `None::<i64>` regardless of the target column,
/// which Postgres rejects for a non-integer column (the same bug class
/// fixed in the derive macro for `save`/`update` during Phase 5 part 3 —
/// see that commit message — but reintroduced here since `insert_raw` and
/// generic `QuerySet::update` build binds at runtime, not compile time, so
/// they need their own field-kind-aware fix). Neither of `insert_raw`'s or
/// `update`'s own required tests catch this because their test models have
/// no nullable non-i64 field; this one does.
#[tokio::test]
async fn test_null_bind_respects_field_type() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_nullable_bind_model")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE test_nullable_bind_model (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL,
            bio TEXT,
            last_seen TIMESTAMPTZ
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // insert_raw with a nullable TEXT and a nullable TIMESTAMPTZ both Null.
    let values = vec![
        ("name", crate::expr::Value::Text("Alice".to_string())),
        ("bio", crate::expr::Value::Null),
        ("last_seen", crate::expr::Value::Null),
    ];
    let pk = crate::queryset::QuerySet::<NullableBindTestModel>::insert_raw(&db, values)
        .await
        .unwrap();

    let row = crate::queryset::QuerySet::<NullableBindTestModel>::new()
        .filter(q!(id = pk))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(row.bio, None);
    assert_eq!(row.last_seen, None);

    // Now set bio to a real value via insert, then clear it back to Null
    // through `update` (the generic QuerySet path djangors-admin's
    // update_from_form drives) — must not error typing NULL as int8.
    let pk2 = crate::queryset::QuerySet::<NullableBindTestModel>::insert_raw(
        &db,
        vec![
            ("name", crate::expr::Value::Text("Bob".to_string())),
            ("bio", crate::expr::Value::Text("hello".to_string())),
        ],
    )
    .await
    .unwrap();

    crate::queryset::QuerySet::<NullableBindTestModel>::new()
        .filter(q!(id = pk2))
        .unwrap()
        .update(
            &db,
            vec![
                (
                    "bio",
                    crate::expr::SetExpr::Literal(crate::expr::Value::Null),
                ),
                (
                    "last_seen",
                    crate::expr::SetExpr::Literal(crate::expr::Value::Null),
                ),
            ],
        )
        .await
        .unwrap();

    let row2 = crate::queryset::QuerySet::<NullableBindTestModel>::new()
        .filter(q!(id = pk2))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(row2.bio, None);
    assert_eq!(row2.last_seen, None);

    sqlx::query("DROP TABLE test_nullable_bind_model")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_file_field_model")]
pub struct FileFieldTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    #[djangors(file_field)]
    pub attachment: Option<String>,
}

#[test]
fn test_file_field_kind_is_set_on_the_model() {
    let meta = FileFieldTestModel::meta();
    let field = meta.fields.iter().find(|f| f.name == "attachment").unwrap();
    assert_eq!(field.kind, FieldKind::FileField);
}

#[tokio::test]
async fn test_file_field_saves_and_reloads_a_real_path_string() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_file_field_model")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE test_file_field_model (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL,
            attachment TEXT
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let saved = FileFieldTestModel {
        id: 0,
        name: "invoice".to_string(),
        attachment: Some("uploads/invoice-42.pdf".to_string()),
    }
    .save(&db)
    .await
    .unwrap();
    assert_eq!(saved.attachment.as_deref(), Some("uploads/invoice-42.pdf"));

    let reloaded = crate::queryset::QuerySet::<FileFieldTestModel>::new()
        .filter(q!(id = saved.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(
        reloaded.attachment.as_deref(),
        Some("uploads/invoice-42.pdf")
    );

    let no_attachment = FileFieldTestModel {
        id: 0,
        name: "no-file".to_string(),
        attachment: None,
    }
    .save(&db)
    .await
    .unwrap();
    let reloaded_none = crate::queryset::QuerySet::<FileFieldTestModel>::new()
        .filter(q!(id = no_attachment.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(reloaded_none.attachment, None);

    sqlx::query("DROP TABLE test_file_field_model")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_signal_lifecycle")]
#[allow(dead_code)]
pub struct SignalLifecycleModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[tokio::test]
async fn test_model_lifecycle_signals() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_signal_lifecycle")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE test_signal_lifecycle (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // 1. No receivers connected -> save/update/delete must work normally
    let model = SignalLifecycleModel {
        id: 0,
        name: "no-receiver".to_string(),
    };
    let saved = model.save(&db).await.unwrap();
    assert!(saved.id > 0);
    let mut updated = saved;
    updated.name = "still-no-receiver".to_string();
    updated.update(&db).await.unwrap();
    updated.delete(&db).await.unwrap();

    // 2. pre_save fires before the row exists in the database
    let pre_save_fired = Arc::new(AtomicBool::new(false));
    let pre_save_row_count = Arc::new(Mutex::new(-1i64));
    let cb_db = db.clone();
    let fired = pre_save_fired.clone();
    let count = pre_save_row_count.clone();
    SignalLifecycleModel::pre_save_signal().connect(move |_payload| {
        let fired = fired.clone();
        let count = count.clone();
        let db = cb_db.clone();
        async move {
            fired.store(true, Ordering::SeqCst);
            let c: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_signal_lifecycle")
                .fetch_one(db.pool())
                .await
                .unwrap();
            *count.lock().unwrap() = c;
        }
    });

    let model2 = SignalLifecycleModel {
        id: 0,
        name: "pre-save-test".to_string(),
    };
    let _saved2 = model2.save(&db).await.unwrap();
    assert!(
        pre_save_fired.load(Ordering::SeqCst),
        "pre_save signal must fire during save()"
    );
    assert_eq!(
        *pre_save_row_count.lock().unwrap(),
        0,
        "row must NOT exist during pre_save callback"
    );

    // 3. post_save fires with real generated pk and row IS queryable
    let post_save_fired = Arc::new(AtomicBool::new(false));
    let post_save_row_count = Arc::new(Mutex::new(-1i64));
    let cb_db2 = db.clone();
    let fired2 = post_save_fired.clone();
    let count2 = post_save_row_count.clone();
    SignalLifecycleModel::post_save_signal().connect(move |_payload| {
        let fired2 = fired2.clone();
        let count2 = count2.clone();
        let db = cb_db2.clone();
        async move {
            fired2.store(true, Ordering::SeqCst);
            let c: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_signal_lifecycle")
                .fetch_one(db.pool())
                .await
                .unwrap();
            *count2.lock().unwrap() = c;
        }
    });

    let model3 = SignalLifecycleModel {
        id: 0,
        name: "post-save-test".to_string(),
    };
    let saved3 = model3.save(&db).await.unwrap();
    assert!(
        post_save_fired.load(Ordering::SeqCst),
        "post_save signal must fire during save()"
    );
    assert!(
        saved3.id > 0,
        "saved instance must have a real generated pk"
    );
    assert!(
        *post_save_row_count.lock().unwrap() > 0,
        "row must exist and be queryable during post_save callback"
    );

    // 4. update() fires pre_save/post_save (not a separate signal pair)
    let update_pre_fired = Arc::new(AtomicBool::new(false));
    let update_post_fired = Arc::new(AtomicBool::new(false));
    let u_pre = update_pre_fired.clone();
    SignalLifecycleModel::pre_save_signal().connect(move |_payload| {
        let u_pre = u_pre.clone();
        async move {
            u_pre.store(true, Ordering::SeqCst);
        }
    });
    let u_post = update_post_fired.clone();
    SignalLifecycleModel::post_save_signal().connect(move |_payload| {
        let u_post = u_post.clone();
        async move {
            u_post.store(true, Ordering::SeqCst);
        }
    });

    let mut to_update = saved3;
    to_update.name = "updated".to_string();
    to_update.update(&db).await.unwrap();
    assert!(
        update_pre_fired.load(Ordering::SeqCst),
        "pre_save must fire during update()"
    );
    assert!(
        update_post_fired.load(Ordering::SeqCst),
        "post_save must fire during update()"
    );

    // 5. pre_delete/post_delete fire around delete() with correct row-existence timing
    let pre_del_fired = Arc::new(AtomicBool::new(false));
    let pre_del_row_count = Arc::new(Mutex::new(-1i64));
    let cb_db3 = db.clone();
    let pdf = pre_del_fired.clone();
    let pdr = pre_del_row_count.clone();
    SignalLifecycleModel::pre_delete_signal().connect(move |_payload| {
        let pdf = pdf.clone();
        let pdr = pdr.clone();
        let db = cb_db3.clone();
        async move {
            pdf.store(true, Ordering::SeqCst);
            let c: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_signal_lifecycle")
                .fetch_one(db.pool())
                .await
                .unwrap();
            *pdr.lock().unwrap() = c;
        }
    });

    let delete_target = SignalLifecycleModel {
        id: 0,
        name: "delete-me".to_string(),
    }
    .save(&db)
    .await
    .unwrap();
    let delete_id = delete_target.id;

    let post_del_fired = Arc::new(AtomicBool::new(false));
    let post_del_row_gone = Arc::new(Mutex::new(false));
    let post_del_fired_clone = post_del_fired.clone();
    let post_del_row_gone_clone = post_del_row_gone.clone();
    let cb_db4 = db.clone();
    SignalLifecycleModel::post_delete_signal().connect(move |_payload| {
        let pdf2 = post_del_fired_clone.clone();
        let prg = post_del_row_gone_clone.clone();
        let db = cb_db4.clone();
        async move {
            pdf2.store(true, Ordering::SeqCst);
            let c: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM test_signal_lifecycle WHERE id = $1")
                    .bind(delete_id)
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
            *prg.lock().unwrap() = c == 0;
        }
    });

    delete_target.delete(&db).await.unwrap();
    assert!(pre_del_fired.load(Ordering::SeqCst), "pre_delete must fire");
    assert!(
        *pre_del_row_count.lock().unwrap() > 0,
        "row must still exist during pre_delete"
    );
    assert!(
        post_del_fired.load(Ordering::SeqCst),
        "post_delete must fire"
    );
    assert!(
        *post_del_row_gone.lock().unwrap(),
        "specific row must be gone by the time post_delete fires"
    );

    // 6. A panicking receiver does not break save/update/delete
    SignalLifecycleModel::pre_save_signal().connect(|_| async move {
        panic!("intentional panic in pre_save receiver");
    });
    SignalLifecycleModel::post_save_signal().connect(|_| async move {
        panic!("intentional panic in post_save receiver");
    });
    SignalLifecycleModel::pre_delete_signal().connect(|_| async move {
        panic!("intentional panic in pre_delete receiver");
    });
    SignalLifecycleModel::post_delete_signal().connect(|_| async move {
        panic!("intentional panic in post_delete receiver");
    });

    let panic_saved = SignalLifecycleModel {
        id: 0,
        name: "panic-test".to_string(),
    }
    .save(&db)
    .await
    .unwrap();
    assert!(
        panic_saved.id > 0,
        "save must succeed despite panicking receivers"
    );

    let mut panic_updated = panic_saved;
    panic_updated.name = "panic-updated".to_string();
    panic_updated.update(&db).await.unwrap();

    panic_updated.delete(&db).await.unwrap();

    sqlx::query("DROP TABLE test_signal_lifecycle")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_modelform_model")]
pub struct ModelFormTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 10)]
    pub name: String,
    pub age: i64,
    pub active: bool,
    #[djangors(file_field)]
    pub attachment: Option<String>,
}

#[test]
fn test_modelform_valid_data_produces_correctly_typed_cleaned_values() {
    let mut data = std::collections::HashMap::new();
    data.insert("name".to_string(), "Alice".to_string());
    data.insert("age".to_string(), "30".to_string());
    data.insert("active".to_string(), "true".to_string());
    // `attachment` (a FileField) and `id` (auto/primary-key) are deliberately omitted -
    // they must not be required inputs on the generated form.

    let cleaned =
        ModelFormTestModel::validate_form(&data).expect("valid data must validate successfully");
    assert_eq!(cleaned.name, "Alice");
    assert_eq!(cleaned.age, Some(30));
    assert!(cleaned.active);
}

#[test]
fn test_modelform_missing_required_field_produces_a_named_error() {
    let mut data = std::collections::HashMap::new();
    data.insert("age".to_string(), "30".to_string());
    data.insert("active".to_string(), "true".to_string());
    // `name` is missing entirely.

    let err = ModelFormTestModel::validate_form(&data)
        .expect_err("missing required field must fail validation");
    assert!(
        err.fields.contains_key("name"),
        "error map must name the missing field, got: {:?}",
        err.fields
    );
}

#[test]
fn test_modelform_non_numeric_integer_produces_a_named_error() {
    let mut data = std::collections::HashMap::new();
    data.insert("name".to_string(), "Bob".to_string());
    data.insert("age".to_string(), "not-a-number".to_string());
    data.insert("active".to_string(), "true".to_string());

    let err = ModelFormTestModel::validate_form(&data)
        .expect_err("non-numeric integer must fail validation");
    assert!(
        err.fields.contains_key("age"),
        "error map must name the invalid field, got: {:?}",
        err.fields
    );
}

#[test]
fn test_modelform_over_max_length_string_produces_a_named_error() {
    let mut data = std::collections::HashMap::new();
    data.insert("name".to_string(), "way-too-long-a-name".to_string());
    data.insert("age".to_string(), "30".to_string());
    data.insert("active".to_string(), "true".to_string());

    let err = ModelFormTestModel::validate_form(&data)
        .expect_err("over-max_length string must fail validation");
    assert!(
        err.fields.contains_key("name"),
        "error map must name the over-length field, got: {:?}",
        err.fields
    );
}

#[tokio::test]
async fn test_modelform_saves_and_updates_a_real_row() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_modelform_model")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE test_modelform_model (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL,
            age BIGINT NOT NULL,
            active BOOLEAN NOT NULL,
            attachment TEXT
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // Create path: validate -> construct -> save -> read back.
    let mut data = std::collections::HashMap::new();
    data.insert("name".to_string(), "Carol".to_string());
    data.insert("age".to_string(), "25".to_string());
    data.insert("active".to_string(), "false".to_string());
    let cleaned = ModelFormTestModel::validate_form(&data).unwrap();
    let instance = ModelFormTestModel::from_cleaned_form(cleaned);
    let saved = instance.save(&db).await.unwrap();
    assert!(saved.id > 0);
    assert_eq!(saved.name, "Carol");
    assert_eq!(saved.age, 25);
    assert!(!saved.active);

    let reloaded = crate::queryset::QuerySet::<ModelFormTestModel>::new()
        .filter(q!(id = saved.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(reloaded.name, "Carol");
    assert_eq!(reloaded.age, 25);

    // Update path: apply cleaned data onto the existing instance, leaving the pk intact.
    let mut update_data = std::collections::HashMap::new();
    update_data.insert("name".to_string(), "CarolUpdt2".to_string());
    update_data.insert("age".to_string(), "26".to_string());
    update_data.insert("active".to_string(), "true".to_string());
    let update_cleaned = ModelFormTestModel::validate_form(&update_data).unwrap();

    let mut to_update = saved.clone();
    let original_id = to_update.id;
    to_update.apply_cleaned_form(update_cleaned);
    assert_eq!(
        to_update.id, original_id,
        "apply_cleaned_form must not touch the primary key"
    );
    to_update.update(&db).await.unwrap();

    let reloaded_after_update = crate::queryset::QuerySet::<ModelFormTestModel>::new()
        .filter(q!(id = original_id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(reloaded_after_update.name, "CarolUpdt2");
    assert_eq!(reloaded_after_update.age, 26);
    assert!(reloaded_after_update.active);
    assert_eq!(
        reloaded_after_update.id, original_id,
        "the real persisted row's primary key must be unchanged"
    );

    sqlx::query("DROP TABLE test_modelform_model")
        .execute(db.pool())
        .await
        .unwrap();
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_reserved_keyword_model")]
#[allow(dead_code)]
pub struct ReservedKeywordTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    // `user` and `group` are reserved SQL keywords - filtering, ordering, and
    // aggregating on a field with this name must not produce a syntax error.
    // Same landmine class as the identifier-quoting bugs fixed in derive(Model)'s
    // INSERT/UPDATE SQL and djangors-migrations' CREATE/ALTER TABLE generation.
    pub user: String,

    #[djangors(default = 0)]
    pub group: i32,
}

#[tokio::test]
async fn test_filter_order_and_aggregate_on_reserved_keyword_field_names() {
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query("DROP TABLE IF EXISTS test_reserved_keyword_model")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE test_reserved_keyword_model (
            id BIGSERIAL PRIMARY KEY,
            \"user\" TEXT NOT NULL,
            \"group\" INTEGER NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    sqlx::query("INSERT INTO test_reserved_keyword_model (\"user\", \"group\") VALUES ('alice', 1), ('bob', 2), ('carol', 1)")
        .execute(db.pool())
        .await
        .unwrap();

    // filter() on a reserved-keyword field name
    let filtered = crate::queryset::QuerySet::<ReservedKeywordTestModel>::new()
        .filter(q!(group = 1))
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(filtered.len(), 2);

    // order_by() on a reserved-keyword field name
    let ordered = crate::queryset::QuerySet::<ReservedKeywordTestModel>::new()
        .order_by("user")
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        ordered.iter().map(|m| m.user.as_str()).collect::<Vec<_>>(),
        vec!["alice", "bob", "carol"]
    );

    // count() with a WHERE clause on a reserved-keyword field
    let count = crate::queryset::QuerySet::<ReservedKeywordTestModel>::new()
        .filter(q!(group = 1))
        .unwrap()
        .count(&db)
        .await
        .unwrap();
    assert_eq!(count, 2);

    sqlx::query("DROP TABLE test_reserved_keyword_model")
        .execute(db.pool())
        .await
        .unwrap();
}

/// Compile-time proof that a `QuerySet` runs against both execution targets:
/// the pool (`&Database`) and an open transaction (`&mut PgConnection`, as
/// handed to the `Database::transaction` closure).
///
/// This is never called. It exists so that a regression in the `DbExecutor`
/// bounds — especially the higher-ranked lifetimes on the transaction closure —
/// fails the build instead of silently forcing users back onto raw SQLx.
#[allow(dead_code)]
async fn assert_queryset_runs_on_pool_and_in_transaction(
    db: &djangors_db::Database,
) -> Result<(), Box<dyn std::error::Error>> {
    // Against the pool.
    QuerySetTestModel::objects().all(db).await?;

    // Inside a transaction: every ORM call shares the transaction, so they
    // commit together or roll back together.
    db.transaction(|conn| {
        Box::pin(async move {
            let rows = QuerySetTestModel::objects().all(&mut *conn).await?;
            QuerySetTestModel::objects().count(&mut *conn).await?;
            if rows.is_empty() {
                crate::QuerySet::<QuerySetTestModel>::insert_raw(
                    &mut *conn,
                    vec![
                        ("name", crate::expr::Value::Text("seeded".into())),
                        ("is_active", crate::expr::Value::Bool(true)),
                    ],
                )
                .await?;
            }
            Ok::<_, crate::error::OrmError>(())
        })
    })
    .await?;

    Ok(())
}
