# Tutorial Part 8: Packaging and Pre-flight Deployment Checks

In Part 8, we review modular application architecture (`src/lib.rs`), execute pre-flight deployment security audits using `dj check --deploy`, compile release binaries, and summarize the complete Djangors application stack.

> [!NOTE]
> All project layout examples match the structure in [`examples/polls`](file:///root/dev/Rango/examples/polls).

---

## 1. Modular Application Architecture

A well-structured Djangors project splits application logic into a library crate (`src/lib.rs`) and a server binary entrypoint (`src/main.rs`).

### `src/lib.rs`

```rust,illustrative
pub mod admin;
pub mod models;
pub mod urls;
pub mod views;
```

This structure makes all models, views, and routing logic reusable for unit tests, background worker binaries, or custom CLI tools.

---

## 2. Pre-flight Deployment Checks

Before deploying to production, run `dj check --deploy` to audit your security configuration and environment settings:

```bash
dj check --deploy
```

`dj check --deploy` verifies key production safety requirements:
- `debug`: Ensures debug mode is set to `false`.
- `secret_key`: Verifies that a cryptographically secure, non-default secret key of at least 32 bytes is configured.
- `allowed_hosts`: Checks that allowed hosts are configured explicitly rather than defaulting to wildcard `*`.
- Security Headers: Ensures security headers and CSRF protections are active in the middleware stack.

---

## 3. Building Production Release Binaries

To compile an optimized production binary with full link-time optimization (LTO):

```bash
cargo build --release --bin polls
```

The resulting binary will be located at `target/release/polls`. Run it in production by providing environment variables:

```bash
DATABASE_URL="postgres://prod_user:prod_pass@db.example.com/polls_prod" \
SECRET_KEY="a-secure-production-secret-key-that-is-at-least-32-bytes-long" \
PORT=8000 \
./target/release/polls
```

---

## 4. Complete App Summary

Congratulations! You have completed the Djangors Polls Tutorial. You have built a production-ready Web application featuring:

1. **Async HTTP Handling**: Tokio-powered async handlers with typed request extractors.
2. **ORM Models**: Strong-typed models with primary keys, foreign keys, filtering (`q!`), and atomic F-expressions (`set!`).
3. **Session Authentication**: Password hashing (`argon2`), login/logout session handling, and staff-gated endpoints (`Auth<User>`).
4. **Automated Testing**: Real-socket integration tests (`#[tokio::test]`).
5. **Security & Middleware**: CSRF protection, signed cookie sessions, and security headers via Tower middleware.
6. **Admin Dashboard**: Full CRUD admin interface with `AdminSite` and `ModelAdminConfig`.

---

## What's Real vs. What Django Has That Djangors Doesn't Yet

> [!IMPORTANT]
> **Key Architecture Differences from Django:**
> - **Binary Output**: Djangors compiles down to a single native binary with zero runtime interpreter dependencies.
> - **Pre-Flight Inspection**: Deployment audits are performed using `dj check --deploy` prior to compilation.
> - **Cargo Workspace Integration**: Djangors apps natively integrate into standard Rust Cargo workspaces.

---

## Final Verification Command

Run the test suite across your completed project:

```bash
cargo test
```
