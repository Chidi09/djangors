# Deployment

`djangors` applications compile into standalone native binaries, making production deployment simple and lightweight.

## Production Containerization (Dockerfile)

`dj new` generates a multi-stage Dockerfile optimized for minimal image size and container health checking:

```dockerfile
FROM rust:1-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/my_app /app/server
EXPOSE 8000
ENV DJANGORS_PORT=8000
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:8000/healthz || exit 1
CMD ["/app/server"]
```

### `/healthz` Route
Projects include a dedicated `/healthz` endpoint returning HTTP `200 OK` for container orchestrators (Docker, Kubernetes) and load balancers:

```rust
pub async fn healthz(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "ok"))
}
```

---

## systemd Service Scaffolding

For deployments directly on Linux VMs, `dj new` scaffolds `deploy/djangors.service`:

```ini
[Unit]
Description=Djangors Web Application (my_app)
After=network.target postgresql.service

[Service]
Type=simple
User=www-data
Group=www-data
WorkingDirectory=/var/www/my_app
ExecStart=/var/www/my_app/my_app
Restart=on-failure
RestartSec=5s
EnvironmentFile=/etc/my_app/djangors.env

[Install]
WantedBy=multi-user.target
```

---

## Production Pre-flight Check (`dj check --deploy`)

Before deploying to production, execute `dj check --deploy` to verify critical security and performance configurations:

```bash
dj check --deploy
```

### Verification Rules Enforced
1. **`DEBUG` mode**: Must be `false` (fails if `debug = true`).
2. **`SECRET_KEY`**: Must be at least 32 characters long.
3. **`ALLOWED_HOSTS`**: Must be configured explicitly (fails if set to default `["127.0.0.1", "localhost"]` when `debug` is `false`).

---

## Graceful Shutdown

`djangors-core` handles process termination signals (`SIGINT` and `SIGTERM`) gracefully via `run_with_shutdown` and `run_service_with_shutdown`:

```rust
app.run_with_shutdown(async {
    tokio::signal::ctrl_c().await.ok();
}).await?;
```

### Shutdown Sequence
1. Upon receiving `SIGINT` or `SIGTERM`, the TCP accept loop stops listening for new connections.
2. An active connection drain phase initiates, allowing in-flight HTTP requests to complete.
3. If in-flight requests do not complete within 30 seconds, remaining connections are aborted to ensure timely process termination.
