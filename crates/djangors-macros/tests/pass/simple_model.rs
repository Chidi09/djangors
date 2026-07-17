use djangors_macros::Model;
use djangors_orm::{FieldKind, DefaultValue};

#[derive(Model)]
#[djangors(app = "test_app", ordering = "-name", table_name = "custom_table")]
pub struct Simple {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 100, default = "default_name")]
    pub name: String,

    pub is_active: bool,
}

fn main() {
    let meta = Simple::meta();
    assert_eq!(meta.struct_name, "Simple");
    assert_eq!(meta.app_label, "test_app");
    assert_eq!(meta.table_name, "custom_table");
    assert_eq!(meta.fields.len(), 3);
    assert_eq!(meta.ordering, &["-name"]);

    let id_field = meta.fields[0];
    assert_eq!(id_field.name, "id");
    assert!(id_field.primary_key);
    assert!(id_field.auto);
    assert_eq!(id_field.kind, FieldKind::BigInt);

    let name_field = meta.fields[1];
    assert_eq!(name_field.name, "name");
    assert_eq!(name_field.kind, FieldKind::Char);
    assert_eq!(name_field.max_length, Some(100));
    assert_eq!(name_field.default, DefaultValue::Text("default_name"));

    let active_field = meta.fields[2];
    assert_eq!(active_field.name, "is_active");
    assert_eq!(active_field.kind, FieldKind::Boolean);
}
