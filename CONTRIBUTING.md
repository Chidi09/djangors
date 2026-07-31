# Contributing to Djangors

Thank you for your interest in contributing to Djangors! This document outlines the development workflow, code standards, and verification requirements for this repository.

---

## Workspace & Versioning

Djangors is structured as a single Cargo workspace comprising **32 crates** that share a unified version number (`0.6.0`). All crates are versioned and released together.

---

## Design-Doc-First Process

For any non-trivial architectural change, new feature, or subsystem addition, **write a design document under `docs/design/` before writing code**.

This repository currently contains **99 design documents** under `docs/design/`. Design docs articulate the design goals, API contracts, threat model considerations, and open trade-offs. This process is our standard development convention, not an optional aspiration.

---

## Code Standards & Documentation

- **Doc Comments**: `#![deny(missing_docs)]` is enabled across every crate in the workspace. All new public modules, structs, enums, traits, methods, and functions must include descriptive doc comments (`///`).
- **Doc Code Block Tags**: Every Rust code block in documentation files (`docs/src/**/*.md`) must be explicitly tagged as either:
  - ` ```rust,compile ` for code examples that must compile clean.
  - ` ```rust,illustrative ` for conceptual or incomplete code snippets.

> [!WARNING]
> A bare ` ```rust ` fence in `docs/src/` is a hard build error enforced via `tools/doc-code-check`.

---

## Running Tests

### Running Tests Without PostgreSQL (Recommended Fast Path)

To run the test suite locally without needing a PostgreSQL database server:

```bash
env -u DATABASE_URL TEST_BACKEND=sqlite cargo test
```

Using the SQLite test backend requires no external database setup and is dramatically faster. For instance, running tests for `djangors-admin` takes **0.69s** with SQLite versus **15.8s** with PostgreSQL.

PostgreSQL-only tests are marked with `#[ignore = "requires PostgreSQL"]` and will be skipped automatically when running with the SQLite backend.

To run PostgreSQL tests, ensure a PostgreSQL instance is running, set `DATABASE_URL` (e.g. `postgres://postgres:postgres@localhost/djangors_test`), and execute:

```bash
cargo test --workspace
```

---

## Verification Commands (CI Enforced)

Every Pull Request is validated in Continuous Integration (CI) against five checks. All five must pass before a PR can be merged:

1. **Format Check**:
   ```bash
   cargo fmt --all -- --check
   ```

2. **Clippy Lints**:
   ```bash
   cargo clippy --workspace --all-targets -- -D warnings
   ```

3. **Workspace Build**:
   ```bash
   cargo build --workspace --all-targets
   ```

4. **Workspace Test Suite**:
   ```bash
   cargo test --workspace
   ```

5. **Workspace Documentation Build**:
   ```bash
   RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
   ```
