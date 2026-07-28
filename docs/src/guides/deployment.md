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

```rust,compile
# use djangors_core::{Request, PathParams, Response, DjangorsError, StatusCode};
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

## Provider-Managed Deployment (`djangors-deploy`)

`djangors-deploy` provides a `DeployProvider` trait (`provision`/`deploy`/`status`/`logs`/
`destroy`), implemented so far by `RenderProvider` (drives Render's REST API directly) and
`SshProvider` (a raw VPS reachable over SSH). This is a first slice: no `dj deploy` CLI
subcommand is wired in yet, and Railway/GCP/AWS providers aren't implemented. Using it today
means calling the trait directly from your own tooling.

```rust,compile
use djangors_deploy::DeploySpec;

fn spec_example() -> DeploySpec {
    DeploySpec {
        project_name: "my-app".to_string(),
        repo_url: Some("https://github.com/me/my-app.git".to_string()),
        branch: "main".to_string(),
        dockerfile_path: "Dockerfile".to_string(),
        docker_context: ".".to_string(),
        health_check_path: "/healthz".to_string(),
        env_vars: vec![("RUST_LOG".to_string(), "info".to_string())],
        region: None,
        plan: None,
        needs_database: true,
    }
}
```

### `RenderProvider`

```rust,illustrative
use djangors_deploy::{DeployProvider, render::RenderProvider};

async fn deploy_to_render(spec: &djangors_deploy::DeploySpec) -> Result<(), djangors_deploy::DeployError> {
    let owner_id = RenderProvider::discover_owner_id("rnd_your_api_key").await?;
    let provider = RenderProvider::new("rnd_your_api_key", owner_id);

    let info = provider.provision(spec).await?;
    provider.deploy(&info, spec).await?; // polls until live or failed
    Ok(())
}
```

### `SshProvider`

`SshProvider` shells out to the system `ssh` binary (via `tokio::process::Command`) rather than
adding a native SSH library dependency, the same class of native-dependency build friction this
project hit once already deploying to a container platform. It clones/hard-resets your repo on
the remote host and runs the same Dockerfile-based `docker build`/`docker run` flow
`RenderProvider` uses. Every value interpolated into a remote shell command is POSIX-escaped and
`project_name` is validated against a safe alphanumeric/-/_ pattern before use, as defense in
depth.

```rust,illustrative
use djangors_deploy::{ssh::SshProvider, DeployProvider};

async fn deploy_over_ssh(spec: &djangors_deploy::DeploySpec) -> Result<(), djangors_deploy::DeployError> {
    let provider = SshProvider::new("203.0.113.10", 22, "deploy", "/home/deploy/.ssh/id_ed25519");
    let info = provider.provision(spec).await?;
    provider.deploy(&info, spec).await?;
    Ok(())
}
```

---

## Error Tracking (optional Sentry integration)

`djangors-core`'s optional `sentry` Cargo feature (off by default, so zero cost/deps unless
enabled) wires Sentry into the same `tracing` setup `init_production_logging()` already uses,
rather than requiring a second, separate logging setup:

```rust,illustrative
use djangors_core::logging::init_production_logging_with_sentry;

fn main() {
    // Empty/invalid DSN produces a disabled client (Sentry's own cross-language convention) -
    // always safe to call unconditionally even with a DSN that may be empty in development.
    let _guard = init_production_logging_with_sentry("https://key@o0.ingest.sentry.io/0");
    // ... start the app; the guard must stay alive for the process lifetime.
}
```

`ERROR`-level `tracing` spans become Sentry events automatically; everything else becomes
breadcrumbs, and panics are captured via Sentry's built-in panic integration.

---

## Graceful Shutdown

`djangors-core` handles process termination signals (`SIGINT` and `SIGTERM`) gracefully via `run_with_shutdown` and `run_service_with_shutdown`:

```rust,compile
# async fn run_app() -> Result<(), Box<dyn std::error::Error>> {
# let app = djangors_core::Djangors::new(djangors_core::DjangorsSettings::default(), djangors_core::Router::new());
app.run_with_shutdown(async {
    tokio::signal::ctrl_c().await.ok();
}).await?;
# Ok(())
# }
```

### Shutdown Sequence
1. Upon receiving `SIGINT` or `SIGTERM`, the TCP accept loop stops listening for new connections.
2. An active connection drain phase initiates, allowing in-flight HTTP requests to complete.
3. If in-flight requests do not complete within 30 seconds, remaining connections are aborted to ensure timely process termination.
