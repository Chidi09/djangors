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
