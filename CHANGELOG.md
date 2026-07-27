# Changelog

All notable changes to Djangors are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/); this project has not yet made a tagged
release, so everything below is grouped by the `PLAN.md` phase it shipped under instead of a
version number.

## [Unreleased]

Everything to date. Workspace version is still `0.0.1` — nothing here has been published to
crates.io yet (tracked in `PLAN.md`, Phase 11).

### Phase 0-1 — Bootstrap and core request/response layer
- Workspace scaffold: `djangors`, `djangors-core`, `djangors-cli` (`dj` binary), and every
  placeholder crate.
- `djangors-core`: `Request`/`Response`/`Router`/`Handler`, tower middleware layer, a real TCP
  server (`Djangors` app struct), typed extractors (`Json`, `Query`, `Form`), panic isolation with
  Django-style debug error pages, a typed async signals bus (`request_started`/`request_finished`),
  and a dev/production tracing subscriber.

### Phase 2 — ORM and migrations v1
- `djangors-db`: sqlx-backed Postgres pool, config, transactions with explicit isolation levels.
- `djangors-orm`: `ModelMeta`/`FieldMeta`/`RelationMeta`, `#[derive(Model)]` (via
  `djangors-macros`), the `Expr` tree and `q!()` query macro, `QuerySet<T>` execution, model
  lifecycle (`save`/`update`/`delete`), aggregation (`Count`/`Sum`/`Avg`/`Min`/`Max`), `F()`
  expressions and race-safe bulk `update()`, and `select_related()` (a 2-query batched fetch, not
  a JOIN).
- `djangors-migrations`: a CreateTable-only v1 engine, wired to `dj migrate`.

### Phase 3 — Templates, forms, static files
- `djangors-template`: MiniJinja-backed engine with loader precedence.
- `djangors-forms`: `FormField` trait, `CharField`/`IntegerField`/`BooleanField`/`EmailField`,
  `#[derive(Form)]`.
- `djangors-staticfiles`: dev serving, `collectstatic`, content-hashed manifest.

### Phase 4 — Sessions, CSRF, and auth
- `djangors-sessions`: signed-cookie session engine.
- CSRF middleware, `ALLOWED_HOSTS`, HSTS, and `Secure` cookie flag support.
- `djangors-auth`: `User` model with Argon2id password hashing, `AuthBackend`, login/logout, the
  `Auth<U>` extractor, login rate limiting, and audit signals.
- `djangors-mail` (console backend) + a password-reset flow.
- A fuzz/threat-model/OWASP security review pass, and `examples/polls` as the first real
  end-to-end app (login-gated voting).

### Phase 5 — The admin site
- `djangors-admin`: `AdminSite` registry, `is_staff`-gated login, a real `createsuperuser`.
- Changelist: generic field rendering, sorting, pagination, `list_display`/`search_fields`,
  `list_filter`, bulk delete, `date_hierarchy`, `list_editable`, CSV export.
- Add/edit forms and delete confirmation (GET confirm + POST delete).
- A full permissions data model (`has_perm`) enforced across every admin view.
- Converted every admin page from hand-built HTML to the real template engine, added a proper
  HTML5 page shell with dark-mode CSS custom properties, per-site branding
  (`site_header`/`site_title`, favicons, logo/accent-color overrides), and CSRF-safe form wiring.
- Computed columns, bulk actions, fieldsets, readonly/`raw_id` fields, a transitive `PROTECT`
  walk on delete, a per-object history page, and extension points.
- A full audit log (`LogEntry`) with a "Recent actions" panel.
- `examples/school` as the Phase 5 Definition-of-Done app.

### Phase 6 — Developer experience / CLI
- `dj new <name>` real project scaffolding, `dj new-app`, `dj run`, `dj check` (`--deploy`),
  `dj makemigrations` preview, `dj dbshell`, `dj test`, `dj shell`.
- `djangors-test`: a real `TestClient`/`TestDatabase`.
- Graceful shutdown for `Djangors::run`/`run_service`.

### Phase 7 — Batteries
- `djangors-cache` (in-memory/database/Redis-backed `Cache` trait + `CacheLayer` middleware) and a
  complete `djangors-mail` (console/SMTP/file/in-memory backends).
- `djangors-contrib-messages`, a shared pagination utility, humanize template filters.
- `djangors-contrib-sitemaps`, `djangors-contrib-syndication` (RSS/Atom), `djangors-contrib-flatpages`,
  `djangors-contrib-redirects`.
- Real field-level audit diffing and `djangors-contrib-guardian` (object-level permissions).
- `djangors-i18n` (Fluent-based `.ftl` catalogs, `Accept-Language` parsing, locale-aware
  formatting) and `djangors-contrib-otp`.

### Phase 8 — REST framework and background jobs
- `djangors-rest`: serializers, `ViewSet`s, router mounting, session/token/JWT auth and permission
  classes, filtering/ordering, and OpenAPI 3.1 generation.
- `djangors-core`: Server-Sent Events streaming with in-process broadcast groups.
- `djangors-tasks`: the `#[task]` macro, a `SKIP LOCKED`-based DB-backed queue, a worker loop, and
  admin integration.

### Phase 9 — Documentation
- An 8-part polls tutorial mirroring Django's own.
- The mdBook docs site (`docs/src/`) with 8 topic guides, a Django-comparison guide, and 6
  how-tos.
- A real `evcxr`-backed `dj shell` REPL.
- 100% public-API rustdoc coverage across every crate (`#![deny(missing_docs)]` everywhere), and
  every documentation code block compiled as a real workspace test.
- The root `README.md`.

### Phase 10 — Hardening, architecture parity, and 1.0 prep
- Honest `oha`-driven HTTP benchmarks against Django/Gunicorn and axum.
- Real migration autogeneration (new-table + new-field diffing against a schema snapshot) and a
  fix to a confirmed `dj migrate` bug found at the time.
- Eight architecture-parity items: compile-time-enforced scoped viewsets, `prefetch_related`
  batch eager-loading with N+1 regression tooling, a pluggable/project-customizable global error
  envelope, named+scoped rate limiting per endpoint, cron/scheduled recurring background jobs,
  cursor (keyset) pagination for REST viewsets, a pluggable `Storage` trait with
  `LocalDiskStorage`, and an `S3Storage` backend + real `FileField`.
- Load testing `djangors-admin` under concurrency, with a connection-pool tuning guide.
- A completed `djangors` facade crate, an API freeze review (both passes, all ~29 crates) with a
  published deprecation/stability policy.
- An internal security review (`cargo-audit` + manual audit) — see
  `docs/security-review-2026-07-27.md` — that fixed an insecure-by-default cookie gap in both
  example apps. A real third-party audit is still open, pending a budget/vendor decision.
- The Astro marketing site (`site/`), deployed on Vercel, with AI-agent-friendly markdown
  (`llms.txt`/`llms-full.txt`, raw `.md` served at every doc URL, an on-page "Copy as Markdown"
  button) and a shipped Claude Code Skill (`.claude/skills/djangors-development/`).

### Phase 11 — Django-parity gap closure (in progress)
- CircleCI pipeline (fmt/clippy/build/test/doc-build against a real Postgres service container,
  plus a `cargo-audit` job) — and, while validating it, a real pre-existing test-isolation race
  found and fixed in `djangors-tasks`'s test suite.
- Remaining items (migration rollback, model signals, `bulk_create`, a `ModelForm` equivalent,
  real multipart file uploads, server-rendered generic class-based views, a custom management-
  command plugin mechanism, a contenttypes/`GenericForeignKey` framework, transactional
  rollback-per-test in `djangors-test`, crates.io publishing, and three live example-app
  deployments) are tracked in `PLAN.md`.
