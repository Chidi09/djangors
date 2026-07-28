# Changelog

All notable changes to Djangors are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/); this project has not yet made a tagged
release, so everything below is grouped by the `PLAN.md` phase it shipped under instead of a
version number.

## [Unreleased]

Everything to date, through Phase 12. Workspace version is `0.2.0`; most crates were first
published to crates.io as `0.1.0`, but that snapshot predates every fix and feature in Phase 12
below — see Phase 12's own publish-status note.

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

### Phase 11 — Django-parity gap closure
- CircleCI pipeline (fmt/clippy/build/test/doc-build against a real Postgres service container,
  plus a `cargo-audit` job) — and, while validating it, a real pre-existing test-isolation race
  found and fixed in `djangors-tasks`'s test suite.
- Migration rollback with typed `Operation` variants (`AddColumn`/`DropColumn`/`AlterColumn`/
  `RenameColumn`) and real down-migrations.
- Model-level signals (`post_save`/`pre_save`/`post_delete`/`pre_delete`).
- `bulk_create` and a `ModelForm` equivalent auto-derived from `#[derive(Model)]`.
- Real `multipart/form-data` file upload parsing (not a stub).
- Server-rendered generic class-based views (`ListView`/`DetailView`/`CreateView`/`UpdateView`/
  `DeleteView`).
- A custom management-command plugin mechanism (`#[management_command]`, `dj <custom-command>`).
- A contenttypes/`GenericForeignKey` framework.
- `djangors-test`: transactional rollback-per-test and a JSON fixtures loader.
- A genuine cross-crate test DDL race (colliding global table names like `auth_user` across
  `djangors-admin`/`djangors-auth`'s own test suites) found and fixed via a real Postgres
  session-level advisory lock primitive (`djangors_test::acquire_cross_process_lock`), verified by
  re-reproducing the original collision as concurrent OS processes before and after the fix.
- First real production deployment (`djangors-polls` to Render), surfacing and fixing three real
  bugs along the way: unquoted reserved-keyword SQL identifiers in `derive(Model)`'s generated
  INSERT/UPDATE, a `dj makemigrations` bug where new models with foreign keys to sibling new
  models couldn't resolve the relation (planned one model at a time instead of together), and a
  Docker build missing TLS/`pkg-config`/`libssl-dev`.
- Most crates first published to crates.io as `0.1.0` (superseded by Phase 12 below — that
  snapshot predates every fix and feature listed there).
- **Not done**: the remaining two example-app deployments (only `djangors-polls` is live) and
  actually republishing the fixed/updated crates (Phase 12 does the fixing; the republish itself
  is still open, tracked in `PLAN.md`).

### Phase 12 — Post-1.0 hardening
Compiled after the first real deployment surfaced genuine bugs, and after comparing Djangors
directly against a real production Django SaaS backend to find concrete gaps. Sequenced easiest
to hardest.
- Quoted every remaining raw SQL identifier in `QuerySet` (`.filter()`/`.order_by()`/
  `bulk_create`/`prefetch_related`/etc.), closing a gap the Phase 11 INSERT/UPDATE fix didn't
  cover.
- `#[derive(Settings)]`: a typed, validated app-config macro (the `pydantic-settings`/
  `django-environ` equivalent) reading `{PREFIX}_{FIELD}` env vars with defaults and
  `Option<T>` support.
- A `CspBuilder`/`CspLayer` middleware (the `django-csp` equivalent).
- Optional Sentry error tracking (`djangors-core`'s `sentry` feature) alongside the existing
  `tracing`-based logging.
- A `django-axes`-style persistent, database-backed account lockout (`PersistentLockoutBackend`),
  distinct from the existing in-memory rate limiter — rejects even correct credentials once
  locked, and survives process restarts.
- `djangors-pdf`: typed PDF generation (report cards, invoices, receipts) via a pure-Rust builder
  API, deliberately not an HTML/Chrome-based renderer (no viable headless-Chrome path in a
  container deployment).
- Optional malware/AV scanning of uploaded bytes via `clamd`'s real `INSTREAM` wire protocol
  (`djangors-staticfiles`'s `clamav` feature), scanning in-memory before anything touches disk.
- `djangors-deploy`: a `DeployProvider` trait for `dj deploy`, shipping a `RenderProvider` (real
  REST API) and an `SshProvider` (shells out to the system `ssh`, avoiding a native SSH library
  dependency) — a first slice, deliberately left to grow (Railway/GCP/AWS not yet implemented, no
  `dj deploy` CLI subcommand wired in yet).
- `djangors-contrib-payments`: a `PaymentProvider` trait and `PaystackProvider`, with an
  idempotency-key-first `Transaction` model (a real DB-level UNIQUE constraint on `reference`, not
  an application-level check-then-insert) and webhook handling that verifies the HMAC-SHA512
  signature against the raw body before ever parsing JSON. Real API shapes validated against a
  working production Paystack integration, not guessed.
- `djangors-contrib-tenancy`: multi-tenancy support — a `Tenant` model, a `TenantMembership` join
  model, membership-verified per-request tenant resolution (`TenantResolutionLayer`, never trusts
  a client-supplied tenant header alone), and a one-line `tenant_scope()` helper reusing
  `djangors-rest`'s existing `Scoped`/`ScopedViewSet` enforcement mechanism. Design doc:
  `docs/design/12.1-multi-tenancy.md`.
- Workspace version bumped `0.1.0` → `0.2.0` and all 32 crates published for real, including
  `djangors-pdf`/`djangors-deploy`/`djangors-contrib-payments`/`djangors-contrib-tenancy` for the
  first time — independently verified live on crates.io, not just attempted. Found and fixed a
  real bug along the way: 13 crates had internal *dev*-dependencies pinned with both `path` and
  `version` (a leftover from the version-bump script not distinguishing `[dependencies]` from
  `[dev-dependencies]`), which made publishing circular for crates with a mutual dev-dependency on
  each other — fixed by keeping only `path` on internal dev-dependencies, which cargo correctly
  drops from the published manifest entirely.
- Also covered every Phase 12 feature in the mdBook doc site for the first time (new guides:
  settings, PDF, payments, multi-tenancy; extended auth/security/deployment guides) — the doc site
  previously had zero mention of anything in this phase. Every code example genuinely compiles,
  verified via `doc-code-check`.
