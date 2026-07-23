# Testing

Djangors supports two testing paradigms: **in-process route testing** via `djangors-test` (`TestClient`) and **real-socket integration testing** over localhost TCP sockets.

---

## 1. In-Process Testing (`TestClient` & `TestDatabase`)

`djangors-test` provides an in-process client that executes requests directly through `Router::handle()` without binding network sockets.

### In-Process Route Tests
```rust
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
```rust
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
```rust
use djangors_test::TestDatabase;

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
```rust
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceBuilder;
use djangors_core::{Djangors, DjangorsSettings, Router};
use djangors_core::router::RouterService;
use djangors_core::middleware::{csrf_layer, security_headers_layer};
use djangors_sessions::{SessionLayer, SignedCookieStore};

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
