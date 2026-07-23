# Tutorial Part 7: The Admin Site and Customization

In Part 7, we set up the Djangors Admin site, register our application models (`Question`, `Choice`, `Group`, `Permission`), configure admin search and filtering using `ModelAdminConfig`, mount admin routes, create a superuser account, and grant permissions.

> [!NOTE]
> All admin code in this part comes directly from [`examples/polls/src/admin.rs`](file:///root/dev/Rango/examples/polls/src/admin.rs).

---

## 1. Setting Up `src/admin.rs`

In Djangors, the admin site is configured in `src/admin.rs` using [`AdminSite`](file:///root/dev/Rango/crates/djangors-admin).

Create `src/admin.rs` and register the models:

```rust
use crate::models::{Choice, Question};
use djangors_admin::{AdminSite, ModelAdminConfig};

pub fn admin_site() -> AdminSite {
    let site = AdminSite::new();
    
    // Register application models with default configuration
    site.register::<Question>();
    site.register::<Choice>();
    
    // Register auth models
    site.register::<djangors_auth::Permission>();
    
    // Register Group with custom ModelAdminConfig (enabling search_fields)
    site.register_with::<djangors_auth::Group>(ModelAdminConfig {
        search_fields: Some(&["name"]),
        ..Default::default()
    });
    
    site.register::<djangors_auth::UserGroup>();
    site.register::<djangors_auth::GroupPermission>();
    site.register::<djangors_auth::UserPermission>();
    
    site
}
```

---

## 2. Mounting Admin Routes

Mount the admin site onto your main router in `src/urls.rs`:

```rust
use djangors_core::Router;
use crate::views;

pub fn urls() -> Router {
    djangors_admin::favicon_routes(
        Router::new()
            .get("/", views::index)
            .get("/{question_id:i64}/", views::detail)
            .get("/{question_id:i64}/results/", views::results)
            .post("/{question_id:i64}/vote/", views::vote)
            .post("/accounts/login/", views::login_view)
            .post("/accounts/logout/", views::logout_view)
            .mount("/admin", crate::admin::admin_site().urls()),
    )
}
```

---

## 3. Creating Superusers & Permissions

Use `djangors-cli` subcommands to seed system permissions and create an administrator account:

```bash
# Create standard view/add/change/delete permissions for registered models
dj createpermissions

# Create an administrative user with full superuser privileges
dj createsuperuser --username admin --email admin@example.com
```

---

## 4. Customizing ModelAdmin with `ModelAdminConfig`

The `ModelAdminConfig` struct allows customizing changelists and search functionality:

- `search_fields`: Specifies text fields for `?q=` ILIKE queries (e.g. `Some(&["name"])`).
- `list_display`: Controls visible columns on model changelists.
- `list_filter`: Filters changelist rows by boolean or specified fields.
- `date_hierarchy`: Groups records by date/time hierarchies.

---

## What's Real vs. What Django Has That Djangors Doesn't Yet

> [!IMPORTANT]
> **Key Architecture Differences from Django:**
> - **Code-Driven Admin**: Admin registration uses Rust functions and method calls (`site.register::<Question>()`) rather than `@admin.register` Python decorators.
> - **Type-Safe Model Admin Configuration**: Customizations are passed via `ModelAdminConfig` structs to `register_with::<T>()`.
> - **Staff Authentication Gating**: Access to `/admin/` endpoints requires an authenticated user with `is_staff = true` or `is_superuser = true`. Unauthenticated requests receive HTTP 401 Unauthorized.

---

## Running and Accessing Admin

1. Start the server:

```bash
DATABASE_URL="postgres://postgres:postgres@localhost/djangors_dev" dj run --port 8000
```

2. Open `http://localhost:8000/admin/` in your browser. Log in using your superuser credentials to manage `Question` and `Choice` records.
