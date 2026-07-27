# Tutorial Part 6: Static Files, Favicon, and Middleware Stack

In Part 6, we examine Djangors' middleware architecture (`security_headers_layer`, `SessionLayer`, `csrf_layer`), static asset management (`favicon_routes`), and static file collection (`collectstatic`).

> [!NOTE]
> All middleware configuration code matches [`examples/polls/src/main.rs`](file:///root/dev/Rango/examples/polls/src/main.rs) and [`examples/polls/src/urls.rs`](file:///root/dev/Rango/examples/polls/src/urls.rs).

---

## 1. Configuring the Tower Middleware Stack

In Djangors, middleware is built using standard Rust Tower layers in `src/main.rs`:

```rust,compile
# fn main() {
# let settings = djangors_core::DjangorsSettings::default();
# let router_service = djangors_core::router::RouterService::new(djangors_core::Router::new(), settings.debug);
let secret_key = if settings.secret_key.is_empty() {
    "dev-only-secret-key-at-least-32-bytes-long-for-signing-cookies".to_string()
} else {
    settings.secret_key.clone()
};

let service = tower::ServiceBuilder::new()
    .layer(djangors_core::middleware::security_headers_layer())
    .layer(djangors_sessions::SessionLayer::new(
        djangors_sessions::SignedCookieStore::new(secret_key.as_bytes()),
    ))
    .layer(djangors_core::middleware::csrf_layer())
    .service(router_service);
# }
```

### Layer Breakdown
1. `security_headers_layer()`: Injects HTTP security headers (`X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy`, etc.).
2. `SessionLayer`: Manages signed, encrypted cookie sessions (`SignedCookieStore`).
3. `csrf_layer()`: Enforces Cross-Site Request Forgery (CSRF) protection on mutating requests (`POST`, `PUT`, `DELETE`), requiring `csrftoken` cookies and `X-CSRFToken` headers.

---

## 2. Favicon & Static File Serving

In `src/urls.rs`, `djangors_admin::favicon_routes` wraps the main application router to automatically handle `favicon.ico` requests:

```rust,compile
# mod views {
#     use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
#     pub async fn index(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn detail(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn results(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn vote(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn login_view(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn logout_view(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
# }
# mod admin {
#     use djangors_admin::AdminSite;
#     pub fn admin_site() -> AdminSite { AdminSite::new() }
# }
use djangors_core::Router;

pub fn urls() -> Router {
    djangors_admin::favicon_routes(
        Router::new()
            .get("/", views::index)
            .get("/{question_id:i64}/", views::detail)
            .get("/{question_id:i64}/results/", views::results)
            .post("/{question_id:i64}/vote/", views::vote)
            .post("/accounts/login/", views::login_view)
            .post("/accounts/logout/", views::logout_view)
            .mount("/admin", self::admin::admin_site().urls()),
    )
}
```

---

## 3. Collecting Static Assets for Production

Djangors provides `dj collectstatic` to discover static assets across source directories, generate cache-busting content hashes, and output them along with a `manifest.json`:

```bash
# Collect static files into the deployment output directory
dj collectstatic --source static --output staticfiles
```

---

## What's Real vs. What Django Has That Djangors Doesn't Yet

> [!IMPORTANT]
> **Key Architecture Differences from Django:**
> - **Tower Middleware**: Instead of string lists in `MIDDLEWARE = [...]`, middleware is constructed at compile-time using Rust's `tower::ServiceBuilder`.
> - **Session Store**: `SignedCookieStore` stores cryptographically signed session state directly in browser cookies without requiring a database session table by default.
> - **Static Manifests**: `dj collectstatic` produces a production manifest JSON file containing hashed asset paths for high-performance static file serving.

---

## Running and Verifying

1. Verify static file collection:

```bash
mkdir -p static
echo "body { background: #f0f0f0; }" > static/style.css
dj collectstatic --source static --output staticfiles
```

2. Inspect the generated manifest in `staticfiles/manifest.json`.
