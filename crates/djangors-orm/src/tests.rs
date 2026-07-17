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
    assert_eq!(saved.is_active, true);

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
