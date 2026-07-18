use crate::models::{Course, Enrollment, Student};
use djangors_admin::{AdminSite, ModelAdminConfig};

pub fn admin_site() -> AdminSite {
    let site = AdminSite::new();

    site.register_with::<Student>(ModelAdminConfig {
        list_display: Some(&["last_name", "first_name", "email", "is_active"]),
        search_fields: Some(&["first_name", "last_name", "email"]),
        list_filter: Some(&["is_active"]),
        date_hierarchy: Some("enrolled_date"),
        ..Default::default()
    });

    site.register_with::<Course>(ModelAdminConfig {
        list_display: Some(&["code", "name", "credits"]),
        search_fields: Some(&["code", "name"]),
        ..Default::default()
    });

    site.register_with::<Enrollment>(ModelAdminConfig {
        list_display: Some(&["id", "grade", "enrolled_on"]),
        list_editable: Some(&["grade"]),
        date_hierarchy: Some("enrolled_on"),
        ..Default::default()
    });

    site.register::<djangors_auth::Permission>();
    site.register_with::<djangors_auth::Group>(ModelAdminConfig {
        search_fields: Some(&["name"]),
        ..Default::default()
    });
    site.register::<djangors_auth::UserGroup>();
    site.register::<djangors_auth::GroupPermission>();
    site.register::<djangors_auth::UserPermission>();

    site
}
