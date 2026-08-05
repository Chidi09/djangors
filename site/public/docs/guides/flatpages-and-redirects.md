# Flat Pages and Redirects

Two database-driven, admin-editable cousins of Django's `django.contrib.flatpages` and `django.contrib.redirects`. Content editors manage rows in the admin site and the DB drives the runtime behavior — there is no code to edit for a new page or redirect.

---

## Flat Pages (`djangors-contrib-flatpages`)

A `FlatPage` is a standalone HTML page with a unique URL.

| Item | Description |
| ---- | ----------- |
| `struct FlatPage` | `#[derive(Model)]` with `id`, `url: String` (unique, max 255), `title: String` (max 255), `content: String`. Backed by the `djangors_flatpage` table. |
| `flatpage_handler(req, params)` | `Result<Response, DjangorsError>`; looks up the exact request path and serves `page.content` as HTML (or `DjangorsError::NotFound`). |
| `flatpage_routes(router, paths)` | Registers an **explicit route per known URL** — not a catch-all (Djangors v1 has no catch-all fallback). |
| `register_admin(&AdminSite)` | Registers the model for admin editing. |

Because content is served as trusted HTML, only trusted staff should be allowed to edit the model.

```rust,illustrative
use djangors_admin::AdminSite;
use djangors_core::Router;
use djangors_contrib_flatpages::{flatpage_routes, register_admin};

// Explicit, known URLs. Each one becomes a route.
let router = flatpage_routes(Router::new(), ["/about/", "/terms/", "/privacy/"]);

// Make the model editable from the admin site.
let admin = AdminSite::new();
register_admin(&admin);
```

---

## Redirects (`djangors-contrib-redirects`)

A `Redirect` maps an incoming path to a destination.

| Item | Description |
| ---- | ----------- |
| `struct Redirect` | `#[derive(Model)]` with `id`, `old_path: String` (unique, max 255), `new_path: String` (max 255). Backed by the `djangors_redirect` table. |
| `lookup_redirect(req, status)` | `Result<Option<Response>, DjangorsError>`; returns a redirect response (with a `Location` header) when a row matches, or `None` for clean fallthrough. |
| `redirect_handler(req, params)` | Handler form that uses `StatusCode::PERMANENT_REDIRECT` and returns `DjangorsError::NotFound` when no row matches. |
| `redirect_routes(router, paths)` | Registers explicit old paths as routes. |
| `register_admin(&AdminSite)` | Registers the model for admin editing. |

```rust,illustrative
use djangors_admin::AdminSite;
use djangors_core::{DjangorsError, Request, Response, Router, StatusCode};
use djangors_contrib_redirects::{lookup_redirect, redirect_routes, register_admin};

// Handler-based mounting for explicitly registered old paths.
let router = redirect_routes(Router::new(), ["/old-path/", "/old-news/campaign/"]);

// For true pre-routing fallthrough, call the lookup in an outer service:
async fn fallback(req: Request) -> Result<Response, DjangorsError> {
    match lookup_redirect(&req, StatusCode::MOVED_PERMANENTLY).await? {
        Some(response) => Ok(response),                       // row matched: follow the redirect
        None => Ok(Response::text(StatusCode::NOT_FOUND, "Not found")),
    }
}

// Make the model editable from the admin site.
let admin = AdminSite::new();
register_admin(&admin);
```

---

## Admin Registration

Both crates expose a `register_admin(&AdminSite)` that registers their model with an existing `djangors-admin` site:

```rust,compile
# fn main() {
use djangors_admin::AdminSite;
use djangors_core::Router;
use djangors_contrib_flatpages::{flatpage_routes, register_admin as register_flatpages_admin};
use djangors_contrib_redirects::{redirect_routes, register_admin as register_redirects_admin};

let router = flatpage_routes(Router::new(), ["/about/", "/terms/"]);
let router = redirect_routes(router, ["/old-path/", "/old-news/campaign/"]);

let admin = AdminSite::new();
register_flatpages_admin(&admin);
register_redirects_admin(&admin);
# }
```

With the models registered, page editors manage the `djangors_flatpage` and `djangors_redirect` tables from the admin interface instead of asking developers for changes.

> [!NOTE]
> Both crates are **fully DB-driven**: the model tables must exist before the handlers run. Run your schema migration for `djangors_contrib_flatpages` (table `djangors_flatpage`) and `djangors_contrib_redirects` (table `djangors_redirect`) — see [Schema Migrations](migrations.md).

> [!NOTE]
> Admin editing requires `djangors-admin` in your dependency tree and a staff superuser account, created with `dj createsuperuser`. See the [Admin Site](admin.md) guide.
