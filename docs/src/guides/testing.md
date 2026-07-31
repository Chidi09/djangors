# Testing

Djangors supports two testing paradigms: **in-process route testing** via `djangors-test` (`TestClient`) and **real-socket integration testing** over localhost TCP sockets.

---

## 1. In-Process Testing (`TestClient` & `TestDatabase`)

`djangors-test` provides an in-process client that executes requests directly through `Router::handle()` without binding network sockets.

### In-Process Route Tests
```rust,compile
use djangors_test::TestClient;
use djangors_core::{Router, Response, StatusCode, Request, PathParams, DjangorsError};
use djangors_sessions::Session;

async fn hello_handler(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "hello from test"))
}

#[tokio::test]
async fn test_hello_route() {
    let router = Router::new().get("/hello", hello_handler);
    let client = TestClient::new(router);

    client.get("/hello")
        .send()
        .await
        .assert_status(StatusCode::OK)
        .assert_contains("hello from test");
}
```

### Form & Session Testing
```rust,compile
# async fn submit_handler(_: djangors_core::Request, _: djangors_core::PathParams) -> Result<djangors_core::Response, djangors_core::DjangorsError> { Ok(djangors_core::Response::text(djangors_core::StatusCode::CREATED, "")) }
# use djangors_sessions::Session;
# use djangors_test::TestClient;
# use djangors_core::{Router, StatusCode};
#[tokio::test]
async fn test_authenticated_form_submit() {
    let router = Router::new().post("/submit", submit_handler);
    let client = TestClient::new(router);

    let session = Session::new_empty();
    session.set("user_id", 42i64);

    client.post_form("/submit", &[("title", "Test Item")])
        .with_session(session)
        .send()
        .await
        .assert_status(StatusCode::CREATED);
}
```

### Database Fixture (`TestDatabase`)
```rust,compile
use djangors_test::TestDatabase;

# #[ignore]
#[tokio::test]
async fn test_database_queries() {
    let test_db = TestDatabase::connect().await.unwrap();
    let db = test_db.database();

    test_db.create_table("CREATE TABLE test_table (id BIGSERIAL PRIMARY KEY)").await.unwrap();
    // ... run queries against db ...
    test_db.drop_table("test_table").await.unwrap();
}
```

---

## 2. Real-Socket Integration Testing

For full-stack integration testing (including Tower middleware layers, real HTTP headers, cookie handling, and graceful shutdown), test suites bind a local TCP socket on port `0`.

### Real-Socket Pattern (As Used in `examples/polls/tests/voting.rs`)
```rust,compile
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceBuilder;
use djangors_core::{Djangors, DjangorsSettings, Router};
use djangors_core::router::RouterService;
use djangors_core::middleware::{csrf_layer, security_headers_layer};
use djangors_sessions::{SessionLayer, SignedCookieStore};

# #[ignore]
#[tokio::test]
async fn test_full_stack_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let settings = DjangorsSettings {
        host: "127.0.0.1".to_string(),
        port: addr.port(),
        debug: true,
        ..Default::default()
    };

    let router = Router::new();
    let router_service = RouterService::new(router, settings.debug);

    let service = ServiceBuilder::new()
        .layer(security_headers_layer())
        .layer(SessionLayer::new(SignedCookieStore::new(b"test-secret-key-32-bytes-minimum!")))
        .layer(csrf_layer())
        .service(router_service);

    let app = Djangors::new(settings, Router::new());
    tokio::spawn(async move {
        app.serve_service(listener, service).await.unwrap();
    });

    // Send HTTP requests over real TCP connection
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf);

    assert!(response.contains("200 OK"));
}
```

---

## Comparison Summary

| Metric | In-Process (`TestClient`) | Real-Socket Integration |
|---|---|---|
| **Speed** | Extremely fast (microseconds) | Fast (milliseconds) |
| **Network Overhead** | Zero (in-memory execution) | Real local TCP loopback socket |
| **Middleware Coverage** | Handler-level extensions & state | Full Tower layer stack & HTTP protocol |
| **Use Case** | Unit tests & route handler assertions | Full-stack end-to-end flow validation |

---

## Choosing a Database Backend

The test suite runs against either PostgreSQL or in-memory SQLite. Which one is used is decided by
`TEST_BACKEND`, falling back to whether `DATABASE_URL` is set:

| Condition | Backend |
|---|---|
| `TEST_BACKEND=sqlite` | SQLite (in-memory) |
| `TEST_BACKEND=postgres` | PostgreSQL |
| neither set, `DATABASE_URL` present | PostgreSQL |
| neither set, no `DATABASE_URL` | SQLite (in-memory) |

SQLite needs no server and no setup:

```bash
# Fast path - no PostgreSQL required. Unset DATABASE_URL so it cannot be picked up.
env -u DATABASE_URL TEST_BACKEND=sqlite cargo test
```

```bash
# PostgreSQL - required before changing any dialect-specific behaviour
export DATABASE_URL="postgres://postgres:postgres@localhost/djangors_test"
cargo test
```

Each SQLite test gets a fresh `sqlite::memory:` database, so isolation is automatic — there is no
shared state to clean up and no advisory lock to arbitrate. The pool is deliberately pinned to a
single connection: with a plain `sqlite::memory:` URL every pooled connection would otherwise be a
*separate* database, and setup DDL run on one connection would be invisible to the next query.

### Why both

SQLite is dramatically faster. Measured on a development machine, `djangors-admin`'s 32 tests take
**0.69s** on SQLite against **15.8s** on PostgreSQL.

It is not a full substitute. A handful of tests cover behaviour SQLite cannot express — transaction
isolation levels, `pg_advisory_lock`, `SKIP LOCKED` queue claiming, and PostgreSQL's width-strict
integer decoding. Those tests return early when `DATABASE_URL` is absent, each with a comment
naming the feature they need. **They report as passing on SQLite without executing**, so a green
SQLite run alone does not prove those paths still work.

Use SQLite for the fast inner development loop, and run against PostgreSQL before changing anything
dialect-specific and in CI.
