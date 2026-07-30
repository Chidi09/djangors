# Changelog

All notable changes to Djangors are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/); this project has not yet made a tagged
release, so everything below is grouped by the `PLAN.md` phase it shipped under instead of a
version number.

## [Unreleased]

Everything to date, through Phase 12. Workspace version is `0.2.0`; most crates were first
published to crates.io as `0.1.0`, but that snapshot predates every fix and feature in Phase 12
below. See Phase 12's own publish-status note.

### Phase 0-1: Bootstrap and core request/response layer
- Workspace scaffold: `djangors`, `djangors-core`, `djangors-cli` (`dj` binary), and every
  placeholder crate.
- `djangors-core`: `Request`/`Response`/`Router`/`Handler`, tower middleware layer, a real TCP
  server (`Djangors` app struct), typed extractors (`Json`, `Query`, `Form`), panic isolation with
  Django-style debug error pages, a typed async signals bus (`request_started`/`request_finished`),
  and a dev/production tracing subscriber.

### Phase 2: ORM and migrations v1
- `djangors-db`: sqlx-backed Postgres pool, config, transactions with explicit isolation levels.
- `djangors-orm`: `ModelMeta`/`FieldMeta`/`RelationMeta`, `#[derive(Model)]` (via
  `djangors-macros`), the `Expr` tree and `q!()` query macro, `QuerySet<T>` execution, model
  lifecycle (`save`/`update`/`delete`), aggregation (`Count`/`Sum`/`Avg`/`Min`/`Max`), `F()`
  expressions and race-safe bulk `update()`, and `select_related()` (a 2-query batched fetch, not
  a JOIN).
- `djangors-migrations`: a CreateTable-only v1 engine, wired to `dj migrate`.

### Phase 3: Templates, forms, static files
- `djangors-template`: MiniJinja-backed engine with loader precedence.
- `djangors-forms`: `FormField` trait, `CharField`/`IntegerField`/`BooleanField`/`EmailField`,
  `#[derive(Form)]`.
- `djangors-staticfiles`: dev serving, `collectstatic`, content-hashed manifest.

### Phase 4: Sessions, CSRF, and auth
- `djangors-sessions`: signed-cookie session engine.
- CSRF middleware, `ALLOWED_HOSTS`, HSTS, and `Secure` cookie flag support.
- `djangors-auth`: `User` model with Argon2id password hashing, `AuthBackend`, login/logout, the
  `Auth<U>` extractor, login rate limiting, and audit signals.
- `djangors-mail` (console backend) plus a password-reset flow.
- A fuzz/threat-model/OWASP security review pass, and `examples/polls` as the first real
  end-to-end app (login-gated voting).

### Phase 5: The admin site
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

### Phase 6: Developer experience / CLI
- `dj new <name>` real project scaffolding, `dj new-app`, `dj run`, `dj check` (`--deploy`),
  `dj makemigrations` preview, `dj dbshell`, `dj test`, `dj shell`.
- `djangors-test`: a real `TestClient`/`TestDatabase`.
- Graceful shutdown for `Djangors::run`/`run_service`.

### Phase 7: Batteries
- `djangors-cache` (in-memory/database/Redis-backed `Cache` trait + `CacheLayer` middleware) and a
  complete `djangors-mail` (console/SMTP/file/in-memory backends).
- `djangors-contrib-messages`, a shared pagination utility, humanize template filters.
- `djangors-contrib-sitemaps`, `djangors-contrib-syndication` (RSS/Atom), `djangors-contrib-flatpages`,
  `djangors-contrib-redirects`.
- Real field-level audit diffing and `djangors-contrib-guardian` (object-level permissions).
- `djangors-i18n` (Fluent-based `.ftl` catalogs, `Accept-Language` parsing, locale-aware
  formatting) and `djangors-contrib-otp`.

### Phase 8: REST framework and background jobs
- `djangors-rest`: serializers, `ViewSet`s, router mounting, session/token/JWT auth and permission
  classes, filtering/ordering, and OpenAPI 3.1 generation.
- `djangors-core`: Server-Sent Events streaming with in-process broadcast groups.
- `djangors-tasks`: the `#[task]` macro, a `SKIP LOCKED`-based DB-backed queue, a worker loop, and
  admin integration.

### Phase 9: Documentation
- An 8-part polls tutorial mirroring Django's own.
- The mdBook docs site (`docs/src/`) with 8 topic guides, a Django-comparison guide, and 6
  how-tos.
- A real `evcxr`-backed `dj shell` REPL.
- 100% public-API rustdoc coverage across every crate (`#![deny(missing_docs)]` everywhere), and
  every documentation code block compiled as a real workspace test.
- The root `README.md`.

### Phase 10: Hardening, architecture parity, and 1.0 prep
- Honest `oha`-driven HTTP benchmarks against Django/Gunicorn and axum.
- Real migration autogeneration (new-table + new-field diffing against a schema snapshot) and a
  fix to a confirmed `dj migrate` bug found at the time.
- Eight architecture-parity items: compile-time-enforced scoped viewsets, `prefetch_related`
  batch eager-loading with N+1 regression tooling, a pluggable/project-customizable global error
  envelope, named+scoped rate limiting per endpoint, cron/scheduled recurring background jobs,
  cursor (keyset) pagination for REST viewsets, a pluggable `Storage` trait with
  `LocalDiskStorage`, and an `S3Storage` backend plus a real `FileField`.
- Load testing `djangors-admin` under concurrency, with a connection-pool tuning guide.
- A completed `djangors` facade crate, an API freeze review (both passes, all ~29 crates) with a
  published deprecation/stability policy.
- An internal security review (`cargo-audit` plus a manual audit, see
  `docs/security-review-2026-07-27.md`) that fixed an insecure-by-default cookie gap in both
  example apps. A real third-party audit is still open, pending a budget/vendor decision.
- The Astro marketing site (`site/`), deployed on Vercel, with AI-agent-friendly markdown
  (`llms.txt`/`llms-full.txt`, raw `.md` served at every doc URL, an on-page "Copy as Markdown"
  button) and a shipped Claude Code Skill (`.claude/skills/djangors-development/`).

### Phase 11: Django-parity gap closure
- CircleCI pipeline (fmt/clippy/build/test/doc-build against a real Postgres service container,
  plus a `cargo-audit` job). While validating it, found and fixed a real pre-existing
  test-isolation race in `djangors-tasks`'s test suite.
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
- Most crates first published to crates.io as `0.1.0` (superseded by Phase 12 below; that
  snapshot predates every fix and feature listed there).
- **Not done**: the remaining two example-app deployments (only `djangors-polls` is live) and
  actually republishing the fixed/updated crates (Phase 12 does the fixing; the republish itself
  is still open, tracked in `PLAN.md`).

### Phase 12: Post-1.0 hardening
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
  distinct from the existing in-memory rate limiter. It rejects even correct credentials once
  locked, and survives process restarts.
- `djangors-pdf`: typed PDF generation (report cards, invoices, receipts) via a pure-Rust builder
  API, deliberately not an HTML/Chrome-based renderer (no viable headless-Chrome path in a
  container deployment).
- Optional malware/AV scanning of uploaded bytes via `clamd`'s real `INSTREAM` wire protocol
  (`djangors-staticfiles`'s `clamav` feature), scanning in-memory before anything touches disk.
- `djangors-deploy`: a `DeployProvider` trait for `dj deploy`, shipping a `RenderProvider` (real
  REST API) and an `SshProvider` (shells out to the system `ssh`, avoiding a native SSH library
  dependency). A first slice, deliberately left to grow (Railway/GCP/AWS not yet implemented, no
  `dj deploy` CLI subcommand wired in yet).
- `djangors-contrib-payments`: a `PaymentProvider` trait and `PaystackProvider`, with an
  idempotency-key-first `Transaction` model (a real DB-level UNIQUE constraint on `reference`, not
  an application-level check-then-insert) and webhook handling that verifies the HMAC-SHA512
  signature against the raw body before ever parsing JSON. Real API shapes validated against a
  working production Paystack integration, not guessed.
- `djangors-contrib-tenancy`: multi-tenancy support, including a `Tenant` model, a
  `TenantMembership` join model, membership-verified per-request tenant resolution
  (`TenantResolutionLayer`, never trusts a client-supplied tenant header alone), and a one-line
  `tenant_scope()` helper reusing `djangors-rest`'s existing `Scoped`/`ScopedViewSet` enforcement
  mechanism. Design doc: `docs/design/12.1-multi-tenancy.md`.
- Workspace version bumped `0.1.0` to `0.2.0` and all 32 crates published for real, including
  `djangors-pdf`/`djangors-deploy`/`djangors-contrib-payments`/`djangors-contrib-tenancy` for the
  first time. Independently verified live on crates.io, not just attempted. Found and fixed a
  real bug along the way: 13 crates had internal *dev*-dependencies pinned with both `path` and
  `version` (a leftover from the version-bump script not distinguishing `[dependencies]` from
  `[dev-dependencies]`), which made publishing circular for crates with a mutual dev-dependency on
  each other. Fixed by keeping only `path` on internal dev-dependencies, which cargo correctly
  drops from the published manifest entirely.
- Also covered every Phase 12 feature in the mdBook doc site for the first time (new guides:
  settings, PDF, payments, multi-tenancy; extended auth/security/deployment guides). The doc site
  previously had zero mention of anything in this phase. Every code example genuinely compiles,
  verified via `doc-code-check`.

### Phase 12.1: dj new fix, 0.2.1 republish, and site polish
- Found and fixed a real bug in `dj new`: it located the sibling `djangors-core` crate via
  `env!("CARGO_MANIFEST_DIR")`, which only resolves correctly when `djangors-cli` is built from
  inside the cloned monorepo. Anyone who installed via `cargo install djangors-cli` from crates.io
  got a generated project with an unresolvable path dependency, and `cargo build` failed
  immediately. Fixed with a runtime check: use a path dependency when a sibling checkout exists,
  otherwise fall back to a real version dependency on the published crate.
- Workspace version bumped `0.2.0` to `0.2.1` and all 32 crates republished to ship the fix.
  Independently verified live on crates.io per crate, then re-verified end to end against the real
  registry (not the local `cargo package` simulation used during development): fresh
  `cargo install djangors-cli --version 0.2.1`, `dj new`, `cargo build`, and real HTTP requests to
  the generated project's `/` and `/healthz`, all green.
- mdBook docs pages gained the Copy-as-Markdown button (it previously only worked on the marketing
  site's own pages), and the button's contrast was fixed on both to meet WCAG AA (it was
  effectively invisible in dark mode due to an inherited low-contrast color).
- Marketing site SEO: real per-page meta descriptions (previously one generic line reused
  site-wide), Open Graph/Twitter Card tags, JSON-LD structured data, a sitemap, and `robots.txt`.
  The `site` canonical was also pointed at the framework's actual live URL, `djangors.vercel.app`
  (it had been set to `djangors.dev`, a domain that was never registered).
- Homepage gained four new content sections (pillars, get-started, features grid, a real 4-column
  footer with every link verified to resolve), and an em-dash reduction pass across the site copy,
  README, and every doc guide.
- Corrected an earlier (Phase 11) conclusion that had closed `tick_recurring_tasks`'s dual-claim
  race as "not a real bug, misattributed to cross-crate test load." It really is a genuine,
  deterministic bug: whenever a concurrent tick runs within roughly the first 60 seconds after a
  cron boundary, a single-step schedule advance can land on an occurrence that's still due,
  wrongly making the earlier fix's 520 clean verification runs a matter of wall-clock luck rather
  than proof. Captured the tick's `now` after acquiring its advisory lock instead of before, and
  fixed the regression test's setup to seed a realistic, boundary-aligned overdue value instead of
  an arbitrary offset. Deliberately preserved the intentional one-occurrence-per-tick catch-up
  design (multiple concurrent ticks are meant to share backlog work, not each grab everything at
  once).

### Phase 12.2: Error handling, ORM transactions, and REST polish

Driven by an external review of a real backend built on 0.2.2, which rated error handling 4.5/10
and REST 5.5/10 as the framework's weakest areas.

**Error handling.** `DjangorsError` was a closed 7-variant enum whose payload was a bare `String`,
with `code()` and `message()` private — so an application could not attach its own status code,
stable domain code, or structured details, and even a custom `ErrorRenderer` could not read them
back out.
- Added `DjangorsError::Api(ApiError)`, carrying an explicit `StatusCode`, a stable domain `code`,
  a `message`, and an optional `serde_json::Value` of `details`. Constructed via
  `DjangorsError::api(status, code, message)` and `.with_details(json)`; `.with_details()` on a
  built-in variant promotes it, preserving status/code/message.
- Made `code()` and `message()` public and added `details()`, so custom renderers can build their
  own envelope without re-matching on the variant.
- Added the `ApiResultExt` trait (`.api_err(status, code)` / `.api_err_msg(...)`) so foreign errors
  convert at the `?` site without a `map_err` closure.
- Error responses now content-negotiate: a project `ErrorRenderer` wins, then the JSON envelope
  when the caller sent `Accept: application/json` or the error is an `ApiError`, then the debug or
  production page. This applies to every route, not just REST. The six duplicated
  render-the-error branches in `router.rs` collapsed onto one `DjangorsError::render` method.
- The JSON envelope now omits `details` entirely when absent rather than emitting `null`.

**ORM transactions.** `Database::transaction` existed, but `QuerySet` only ever executed against
`Database::pool()`, so no ORM call could join a transaction — atomic work had to drop to raw sqlx
and give up the `QuerySet` API.
- Added `djangors_db::DbExecutor`, implemented for `&Database` (the pool) and `&mut PgConnection`
  (an open transaction, as handed to the `transaction` closure), plus the `Conn` handle that
  centralises the pool-versus-transaction dispatch.
- Every `QuerySet` method is now generic over `DbExecutor`: `all`, `get`, `first`, `exists`,
  `count`, `aggregate`, `update`, `insert_raw`, `bulk_create`, `delete_by_pk`, `select_related`,
  and the free `prefetch_related`. Existing `&Database` call sites are unchanged.
- Added `impl From<OrmError> for DbError` (and a `DbError::Orm` variant). Without it the ORM still
  could not be used inside `transaction`, whose closure must return an error convertible to
  `DbError` — the trait bound, not the executor, was the actual blocker.
- **Breaking:** `select_related` and `prefetch_related` gained a trailing inferred type parameter,
  so turbofished calls become `select_related::<Related, _>(..)` and
  `prefetch_related::<Parent, Child, _>(..)`.

**REST.**
- Page size is configurable per endpoint via `ViewSetConfig::page_size`; it was a hardcoded
  `REST_PER_PAGE = 100`. Setting `max_page_size` additionally opts the endpoint into a
  client-supplied `?page_size=`, clamped to that cap. Unparseable or out-of-range values fall back
  to the default rather than failing an otherwise valid request.
- Added the `IsStaff`, `IsSuperuser`, and `IsReadOnly` permission policies, the `And`/`Or`/`Not`
  combinators, and the `PermissionExt` trait (`.and()`, `.or()`, `.negate()`) — previously the only
  policies were `AllowAny` and `IsAuthenticated`, with no way to compose them.
- Added `current_user()`, which resolves the request's user across session, token, and JWT auth
  using the same precedence as `IsAuthenticated`.

**REST, part two — the serializer layer.** `djangors-rest` was a single 3,257-line `lib.rs` whose
`serialize`/`deserialize` were direct `Model`-to-JSON with no field control and no validation hook.
- Split into `auth`, `permissions`, `serializers`, `validation`, `pagination`, `viewsets`, and
  `openapi` modules. Every name is re-exported from the crate root, so existing imports are
  unaffected.
- Added the `Serializer<M>` trait (`to_representation` / `to_internal_value` / `validate` / `parse`)
  and `ModelSerializer<M>`, the metadata-driven default. `ModelSerializer` with `FieldSet::all()`
  reproduces the previous behaviour exactly.
- Added `FieldSet`: `only` / `excluding` / `read_only` / `write_only`, giving the read/write split
  the review found missing. A write to a read-only field is now *rejected* rather than silently
  dropped — a client that thinks it set `id` is told it did not.
- Added `ValidationErrors`: a `{field: [messages]}` map with `non_field_errors`, which renders as a
  `422` `DjangorsError::Api` carrying the whole map in `details`. Validation failures used to
  collapse into one `BadRequest` string, so a client could not tell which field was wrong.
- Added the `Validator<T>` trait and `ModelSerializer::with_validator` for object-level rules. Every
  registered validator runs even if an earlier one failed, so the client sees the full set at once.
- Added the `Pagination` trait with `PageNumberPagination` (the default), `LimitOffsetPagination`,
  and `CursorPagination`. The strategy owns both the row window and the response envelope.
- Added `ViewSetOptions<M>` (serializer + pagination + config), `*_with_options` handlers for
  `list`/`retrieve`/`create`/`update`, and `viewset_routes_with_options` so the options reach every
  handler rather than just `list`.

**Two real bugs found while building the above, both caught by new tests.**
- `PATCH` was not partial. `deserialize` walks every column and defaults the absent ones, so a
  `PATCH` of one field would persist a `false` for every omitted boolean and `NULL` for every
  omitted nullable — silently resetting columns the client never mentioned. `deserialize` is now
  partial-aware (`deserialize_partial`), and absent keys are skipped rather than defaulted.
- A missing non-nullable boolean was silently accepted as `false` on a full write. That is right for
  an HTML checkbox and wrong for a JSON API, where `POST {}` would quietly clear a flag.
  `ModelSerializer` now reports it as required; `deserialize` keeps the form semantics the admin
  depends on.

**Router state and middleware ergonomics.**
- Added `Request::require_state<T>()`, returning a descriptive error naming the missing type instead
  of the `.ok_or_else(|| ...Internal("Database connection not found"))?` that appeared 30 times
  across the framework. All 30 call sites now use it.
- `Router::mount` now inherits state from the sub-router for types the parent has not set (the
  parent still wins on conflict). Previously a mounted sub-router's state was dropped, so a handler
  that worked standalone would fail with "state absent" once mounted.
- Added `AppState::merge`, `contains`, `len`, and `is_empty`.

