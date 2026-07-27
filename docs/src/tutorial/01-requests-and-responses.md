# Tutorial Part 1: Requests and Responses

This tutorial guides you through building a Polls application in Djangors, mirroring the structure of Django's official tutorial. In Part 1, we will set up a new project, create our first HTTP view, configure routing, and launch the development server.

> [!NOTE]
> The source of truth for all code snippets in this tutorial is the working [`examples/polls`](file:///root/dev/Rango/examples/polls) project in this repository.

---

## 1. Creating a New Project

To create a new Djangors project, run the `dj new` command provided by `djangors-cli`:

```bash
dj new polls
```

This sets up a standard Rust binary crate tailored for Djangors. The generated `Cargo.toml` specifies the required framework dependencies:

```toml
[package]
name = "polls"
description = "Polls app for Djangors"
version = "0.0.1"
edition = "2021"
publish = false

[lib]
name = "polls"
path = "src/lib.rs"

[[bin]]
name = "polls"
path = "src/main.rs"

[dependencies]
djangors-core = { path = "../../crates/djangors-core" }
djangors-orm = { path = "../../crates/djangors-orm" }
djangors-macros = { path = "../../crates/djangors-macros" }
djangors-db = { path = "../../crates/djangors-db" }
djangors-auth = { path = "../../crates/djangors-auth" }
djangors-admin = { path = "../../crates/djangors-admin" }
djangors-sessions = { path = "../../crates/djangors-sessions" }
tokio = { version = "1.53.0", features = ["full"] }
tower = { version = "0.5.3", features = ["util"] }
hyper = { version = "1.10.1", features = ["full"] }
chrono = { version = "0.4.45", features = ["std", "serde"] }
serde = { version = "1.0.228", features = ["derive"] }
async-trait = "0.1.86"
```

---

## 2. Writing Your First View

In Djangors, HTTP views are asynchronous functions that accept a [`Request`](file:///root/dev/Rango/crates/djangors-core) and [`PathParams`](file:///root/dev/Rango/crates/djangors-core), returning a `Result<Response, DjangorsError>`.

Create `src/views.rs` and add an index view that returns a simple HTML string:

```rust,compile
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};

pub async fn index(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::html(
        StatusCode::OK,
        "Hello, world. You're at the polls index.".to_string(),
    ))
}
```

---

## 3. Configuring the Router

Next, map your view to a URL endpoint. Create `src/urls.rs` and define a function that constructs a [`Router`](file:///root/dev/Rango/crates/djangors-core):

```rust,compile
# mod views {
#     use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
#     pub async fn index(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
#         Ok(Response::html(StatusCode::OK, ""))
#     }
# }
use djangors_core::Router;

pub fn urls() -> Router {
    Router::new().get("/", views::index)
}
```

Expose the URL module in `src/lib.rs`:

```rust,illustrative
pub mod admin;
pub mod models;
pub mod urls;
pub mod views;
```

---

## 4. Setting Up the Server Entrypoint

In `src/main.rs`, initialize dev logging, load environment settings, construct the Tower middleware pipeline, and start the HTTP server:

```rust,compile
# mod polls {
#     pub mod urls {
#         use djangors_core::Router;
#         pub fn urls() -> Router { Router::new() }
#     }
# }
use djangors_core::{Djangors, DjangorsError, DjangorsSettings, Router};
use polls::urls;

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    djangors_core::logging::init_dev_logging();

    let (settings, warnings) = DjangorsSettings::load()?;
    for w in warnings {
        eprintln!("settings warning: {w}");
    }

    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL environment variable is not set");
            std::process::exit(1);
        }
    };

    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_db::Database::connect(&config)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let router = urls::urls().with_state(db);
    let router_service = djangors_core::router::RouterService::new(router, settings.debug);

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

    Djangors::new(settings, Router::new())
        .run_service(service)
        .await
}
```

---

## What's Real vs. What Django Has That Djangors Doesn't Yet

> [!IMPORTANT]
> **Key Architecture Differences from Django:**
> - **Async by Default**: Djangors handlers are native Rust `async fn` signatures built on Tokio and Hyper/Tower.
> - **Typed Signatures**: Views take `(Request, PathParams)` and explicitly return `Result<Response, DjangorsError>`.
> - **Explicit Router Chain**: Routes are chained on `Router::new()` using HTTP verb methods (`.get()`, `.post()`), rather than Python lists of `path()` objects.
> - **Middleware Stack**: Middleware is composed using standard Rust `tower::ServiceBuilder` layers rather than string class paths in `settings.py`.

---

## Running the Dev Server

To launch your development server, set the database connection string and run `dj run` (or `cargo run`):

```bash
DATABASE_URL="postgres://postgres:postgres@localhost/djangors_dev" dj run --port 8000
```

Navigate to `http://localhost:8000/` in your browser or curl it to confirm:

```bash
curl http://localhost:8000/
# Output: Hello, world. You're at the polls index.
```
