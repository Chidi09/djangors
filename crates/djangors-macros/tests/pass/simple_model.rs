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

    #[djangors(file_field)]
    pub attachment: Option<String>,

    #[djangors(choices = ["draft", "published"])]
    pub status: String,

    #[djangors(auto_now_add = true)]
    pub created_at: chrono::DateTime<chrono::Utc>,

    #[djangors(auto_now = true)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn main() {
    let meta = Simple::meta();
    assert_eq!(meta.struct_name, "Simple");
    assert_eq!(meta.app_label, "test_app");
    assert_eq!(meta.table_name, "custom_table");
    assert_eq!(meta.fields.len(), 7);
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
    assert_eq!(meta.fields[3].kind, FieldKind::FileField);

    let status_field = meta.fields[4];
    assert_eq!(status_field.name, "status");
    assert_eq!(status_field.choices, &[("draft", "draft"), ("published", "published")]);

    let created_field = meta.fields[5];
    assert_eq!(created_field.name, "created_at");
    assert_eq!(created_field.kind, FieldKind::DateTime);

    let updated_field = meta.fields[6];
    assert_eq!(updated_field.name, "updated_at");
    assert_eq!(updated_field.kind, FieldKind::DateTime);
}
