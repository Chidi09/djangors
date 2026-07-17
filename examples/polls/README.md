# `examples/polls` — the Djangors API spec

This is Django's tutorial app, ported to Djangors.

## What's real right now vs. aspirational

- **HTTP layer**: 100% real — `Router`, `Handler`, `Request`, `Response`, `PathParams`, `DjangorsError`, `DjangorsSettings`, `Djangors::run_service()`, and the `Form` extractor.
- **ORM**: 100% real — `#[derive(Model)]`, `ModelMeta`, `ForeignKey<T>`, `QuerySet`, the `q!()` query macro, `set!()` macro, and atomic updates via `F()`.
- **Auth and login-gating**: 100% real — `AuthBackend`, `ModelBackend`, `Auth<U>` extractor for login-gated voting, and `login`/`logout` session helpers.
- **Route-name reversal (`reverse!()`)**: Still aspirational. The views use explicit route string formatting (`format!()`) instead.
- **Migrations**: Still raw-SQL-in-tests (not real migration files).
- **Admin**: Stays fully aspirational (Phase 5, not started).
