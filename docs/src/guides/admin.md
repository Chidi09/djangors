# Admin Site

`djangors-admin` provides an automatic, customizable administration interface for managing database models.

## `AdminSite` Initialization & Branding

Create and configure an `AdminSite` instance:

```rust
use djangors_admin::{AdminSite, ModelAdminConfig};
use polls::models::{Choice, Question};

let admin = AdminSite::new()
    .with_site_header("Polls Administration")
    .with_site_title("Polls Admin")
    .with_logo_url("/static/logo.png")
    .with_accent_color("#2563eb");

// Register models
admin.register::<Question>();
admin.register_with::<Choice>(ModelAdminConfig {
    list_display: Some(&["choice_text", "votes", "question"]),
    search_fields: Some(&["choice_text"]),
    list_filter: Some(&[]),
    ..Default::default()
});
```

---

## `ModelAdminConfig` Options

Customize how models are displayed and edited in the admin interface:

```rust
pub struct ModelAdminConfig {
    pub list_display: Option<&'static [&'static str]>,
    pub search_fields: Option<&'static [&'static str]>,
    pub list_filter: Option<&'static [&'static str]>,
    pub date_hierarchy: Option<&'static str>,
    pub list_editable: Option<&'static [&'static str]>,
    pub computed_columns: Option<
        &'static [(
            &'static str,
            fn(&[(&'static str, djangors_orm::expr::Value)]) -> String,
        )],
    >,
    pub actions: Option<&'static [AdminAction]>,
    pub fieldsets: Option<&'static [(&'static str, &'static [&'static str])]>,
    pub readonly_fields: Option<&'static [&'static str]>,
    pub raw_id_fields: Option<&'static [&'static str]>,
    pub base_filter: Option<djangors_orm::UnresolvedExpr>,
}
```

### Config Field Details
- **`list_display`**: Fields to render as columns on the changelist page.
- **`search_fields`**: Text fields to `ILIKE`-search against `?q=` queries.
- **`list_filter`**: Boolean fields to render filter options in the sidebar.
- **`date_hierarchy`**: DateTime field for date drill-down navigation.
- **`list_editable`**: Fields editable inline directly from the changelist table.
- **`computed_columns`**: Custom display column functions evaluated dynamically per row.
- **`actions`**: Bulk actions callable on selected rows (a `"delete_selected"` bulk action is registered by default).
- **`fieldsets`**: Groups fields into titled sections on the add/change form.
- **`readonly_fields`**: Prevents editing specified fields in forms.
- **`raw_id_fields`**: Renders foreign key inputs as raw integer ID fields with target lookup links.
- **`base_filter`**: Baseline queryset filter automatically applied to all changelist queries.

---

## Favicons & Static Branding Routes

`djangors-admin` includes static favicon serving helper `favicon_routes`:

```rust
use djangors_admin::favicon_routes;
use djangors_core::Router;

let router = favicon_routes(Router::new());
```

Routes served:
- `/favicon.ico`
- `/favicon-16x16.png`
- `/favicon-32x32.png`
- `/apple-touch-icon.png`
- `/android-chrome-192x192.png`
- `/android-chrome-512x512.png`
- `/manifest.json`

---

## Audit Logging (`LogEntry`)

All admin actions (additions, updates, deletions) are recorded in the `djangors_admin_log` table via `LogEntry`:
- **`ACTION_ADDITION`** (`1`): Object creation.
- **`ACTION_CHANGE`** (`2`): Object field edits (with field diff payload).
- **`ACTION_DELETION`** (`3`): Object removal.
