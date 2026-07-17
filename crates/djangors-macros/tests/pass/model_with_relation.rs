use djangors_macros::Model;
use djangors_orm::{ForeignKey, OnDelete, RelationKind};

#[derive(Model)]
#[djangors(app = "test_app")]
pub struct Parent {
    #[djangors(primary_key, auto)]
    pub id: i64,
}

#[derive(Model)]
#[djangors(app = "test_app")]
pub struct Child {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(foreign_key(on_delete = "protect", related_name = "children"))]
    pub parent: ForeignKey<Parent>,
}

fn main() {
    let parent_meta = Parent::meta();
    assert_eq!(parent_meta.struct_name, "Parent");
    assert_eq!(parent_meta.fields.len(), 1);

    let child_meta = Child::meta();
    assert_eq!(child_meta.struct_name, "Child");
    assert_eq!(child_meta.fields.len(), 1);

    let rel = child_meta.relations[0];
    assert_eq!(rel.field_name, "parent");
    assert_eq!(rel.kind, RelationKind::ForeignKey);
    assert_eq!(rel.on_delete, OnDelete::Protect);
    assert_eq!(rel.related_name, Some("children"));

    let target_meta = (rel.target)();
    assert_eq!(target_meta.struct_name, "Parent");
}
