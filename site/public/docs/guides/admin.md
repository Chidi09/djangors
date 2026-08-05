# Admin Site

`djangors-admin` provides an automatic, customizable administration interface for managing database models.

## `AdminSite` Initialization & Branding

Create and configure an `AdminSite` instance:

```rust,compile
# fn main() {
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
# }
```

---

## `ModelAdminConfig` Options

Customize how models are displayed and edited in the admin interface:

```rust,compile
# use djangors_admin::AdminAction;
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

> [!IMPORTANT]
> `register_with` **validates your config at registration time, at startup, by
> `panic`** — not at first request. A field named in `list_display`,
> `search_fields`, `list_filter`, `date_hierarchy`, or `list_editable` that does
> not exist on the model, or that has the wrong kind (a `String` in
> `list_filter`, which must be Boolean; a `ForeignKey`/integer in
> `search_fields`, which must be text-like; a non-`DateTime` in
> `date_hierarchy`), produces a hard panic the moment the admin site is
> registered. Treat the config as compile-time-ish: a wrong type surfaces as an
> immediate startup crash, which is precise but not graceful. The allowed kinds
> are: `list_filter` → Boolean only; `search_fields` → `Char`/`Text`/`Email`/
> `Url`/`Slug`/`Ip`; `list_editable` → text and numeric kinds; `date_hierarchy`
> → `DateTime`.

---

## Favicons & Static Branding Routes

`djangors-admin` includes static favicon serving helper `favicon_routes`:

```rust,compile
# fn main() {
use djangors_admin::favicon_routes;
use djangors_core::Router;

let router = favicon_routes(Router::new());
# }
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

---

## Computed columns & `base_filter`

`computed_columns` lets a changelist column be produced by a function instead of
a raw model field. The function receives the row's field/value pairs and returns
the display string; list it in `list_display` alongside real fields.

```rust,compile
# use djangors_admin::{AdminSite, ModelAdminConfig};
# use djangors_orm::expr::Value;
# use polls::models::Question;
# fn main() {
// A column computed from two fields (`id` and `question_text`).
fn id_and_text(values: &[(&'static str, Value)]) -> String {
    let text = values
        .iter()
        .find(|(n, _)| *n == "question_text")
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();
    format!("#{} {text}", values.iter().find(|(n, _)| *n == "id").map(|(_, v)| v.to_string()).unwrap_or_default())
}

let site = AdminSite::new();
site.register_with::<Question>(ModelAdminConfig {
    list_display: Some(&["id", "combo"]),
    computed_columns: Some(&[("combo", id_and_text)]),
    ..Default::default()
});
# }
```

`base_filter` applies an `UnresolvedExpr` (the same value `q!` produces) to
every changelist query — Django's `get_queryset`-scoping for the admin list.
Combine it with [`Scoped`](rest.md) semantics when a multi-tenant admin exists,
or simply to default a soft-deleted/model-state filter:

```rust,compile
# use djangors_admin::{AdminSite, ModelAdminConfig};
# use polls::models::Question;
# fn main() {
let site = AdminSite::new();
site.register_with::<Question>(ModelAdminConfig {
    // Only list questions with any votes (q! result used verbatim).
    base_filter: Some(djangors_orm::q!(votes__gt = 0i32) as djangors_orm::UnresolvedExpr),
    ..Default::default()
});
# }
```

> [!NOTE]
> `q!` returns `UnresolvedExpr` already, so no cast is needed in real code; the
> explicit `as` above only illustrates the expected type of the field.

## The `ModelAdmin` trait

`register_with(ModeAdminConfig)` covers the common cases; for full control,
implement the `ModelAdmin` trait directly and register it off the standard
config flow. The required methods describe the model (`model_meta`,
`field_names`, `changelist`, `export_csv_rows`, `get_by_pk`,
`update_from_form`/`create_from_form`, `delete_by_pk`), and the optional ones
default when omitted (`actions`, `fieldsets`, `readonly_fields`,
`raw_id_fields`, plus the search/filter/editable/hierarchy accessors).
`DefaultModelAdmin<M>` in `djangors-admin` is the built-in implementation
`register_with` uses, and you can use it as a reference when writing your own.
