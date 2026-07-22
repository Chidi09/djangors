# Djangors — The Django of Rust

**One-line pitch:** Everything that makes Django pleasant — the ORM, migrations, the admin, forms, auth, the batteries, the docs, the tutorial — with Rust's speed, safety, and single-binary deploys. Built for the workloads Django gets nervous about: banking backends, school management systems, high-traffic e-commerce.

**North star test:** A developer who knows Django should be able to read the Djangors tutorial and feel *at home* within an hour. A CTO at a bank should be able to answer "why Djangors?" with: memory safety, no runtime type errors in production, 10–50x throughput per server, and a compile step that catches what Django catches only in production.

---

## Part 0 — Non-negotiable principles

These decide every argument later. Write them down now, never violate them.

1. **Batteries included.** If Django ships it, Djangors ships it. No "just wire up these 9 crates yourself."
2. **Convention over configuration, configuration always possible.** Sensible defaults everywhere; every default overridable.
3. **The admin is a first-class product, not a demo.** It's the killer feature. It must be good enough that a school secretary or bank ops person uses it daily.
4. **Compile-time where it helps, runtime where Django's flexibility matters.** Models and queries: checked at compile time. Templates and admin rendering: runtime, so they hot-reload and can be overridden without recompiling.
5. **Async-first, sync-feeling.** Tokio underneath; the developer writes `.await` and nothing else async-flavored unless they want to.
6. **One binary out.** `cargo build --release` produces a single deployable artifact containing templates, static files, and migrations. Deployment story: copy one file.
7. **Boring, stable, documented.** Django won by being boring in the best way. Semantic versioning, deprecation policies from day one, docs written alongside code — never after.
8. **Regulated-industry-grade by default.** Audit logging, strong password hashing, CSRF/XSS/SQLi protection, decimal money types, explicit transaction isolation — defaults, not add-ons.

---

## Part 1 — Study the field first (1–2 weeks of reading, do not skip)

You are not the first to attempt pieces of this. Steal every good idea, note every mistake.

| Project | What to study | What to take / avoid |
|---|---|---|
| **Django itself** | App registry, migration autodetector, admin internals (`ModelAdmin`, `ChangeList`), `QuerySet` lazy evaluation, middleware contract, `settings` object | The blueprint. Read `django/db/migrations/autodetector.py` and `django/contrib/admin/options.py` line by line. |
| **cot.rs** (formerly Flareon) | Explicitly "Django for Rust." Study their model macro, admin, and what stalled or feels un-Django | Closest prior art. Learn why it hasn't taken over. |
| **Runique** | Newer Django-inspired framework (v2.x, solo author) built on Axum + SeaORM + Tera: auto admin, forms, migrations, auth, CSRF/CSP | Validates the demand. Study its admin generation and where gluing SeaORM/Tera (vs. owning the stack) limits Django-parity. |
| **loco.rs** | "Rails for Rust." Scaffolding CLI, generators, project layout, their SeaORM integration | Best-in-class Rust DX for full-stack. Their `cargo loco` CLI is the bar to beat. |
| **SeaORM / Diesel / sqlx** | SeaORM's runtime entity metadata; Diesel's compile-time DSL and its ergonomic ceiling; sqlx's driver layer and compile-checked queries | Build Djangors's ORM *on sqlx's driver layer*; do not write postgres wire protocol yourself. Avoid Diesel's trait-error ergonomics. |
| **axum / actix-web / hyper / tower** | Router design, extractors, `tower::Service`/`Layer` middleware model | Build the HTTP kernel on **hyper + tower**. Tower's `Layer` is the middleware contract; don't invent one. |
| **MiniJinja / Tera / Askama** | MiniJinja: runtime Jinja2, embeddable, by the Flask author. Askama: compile-time templates | Use **MiniJinja** as the engine core (runtime = hot reload + admin overrides), add Django-flavored filters/tags on top. |
| **Django REST Framework** | Serializers, viewsets, routers, permissions, throttling | The Phase-8 blueprint. |
| **django-simple-history, django-guardian, django-allauth, django-otp** | Audit trails, object-level permissions, social auth, 2FA | These become `djangors-contrib` crates — they're what banking/school buyers actually need. |
| **evcxr, bacon, cargo-watch, subsecond/dioxus hot-reload** | Rust dev-loop speed tooling | Feeds the `djangors run` watch/rebuild design. |

**Deliverable:** a `docs/prior-art.md` with notes, and a list of ideas explicitly adopted/rejected with reasons.

### 1.5 — Competitive reality check (as of July 2026) and how Djangors stays the most refined

Verified state of the field — nobody has shipped a *complete* Django yet; the gap is real but closing, so refinement is a moving target you must actively defend:

| Rival | Where they are | Their gap = Djangors's edge |
|---|---|---|
| **Cot** | Django-inspired, own ORM + auto-migrations + admin. Publicly early-stage: by its own admission the ORM is limited, migration autodetection covers a small subset of operations, the admin lacks pagination/filtering/search, and there's no permission system, background tasks, or WebSockets. Getting ecosystem attention (covered by JetBrains' Rust blog and InfoWorld). | Djangors's migration engine (full autodetector with state replay), full permission/auth stack, and an admin with changelist filters/search/inlines/actions is precisely the 80% Cot hasn't built. Their ORM depth is the bar to clear *first*. |
| **Runique** | v2.x, solo-author, Django-inspired glue over Axum + SeaORM + Tera: auto admin, forms/validation, migrations, auth, CSRF/CSP. | Gluing third-party ORM/templating caps Django-parity: no `field__lookup` querysets, no app registry, no `makemigrations`-style autodetection against its own metadata, thin docs, bus-factor 1. Djangors owning the ORM metadata layer (ModelMeta) is what unlocks the admin/forms/migrations trinity Runique can't fully deliver on SeaORM. |
| **loco.rs** | The most polished Rust full-stack DX today (Rails-flavored): great CLI/generators/workers/mailers on SeaORM. | Rails-shaped, not Django-shaped: no real admin, no Django-style migrations-from-models, weaker forms story. Its CLI polish is the standard Djangors's `djangors` binary must match. |
| **rustango** | Discovered during name research (crates.io, repo `ujeenet/rustango`): describes itself as Django-shaped batteries-included — ORM, migrations, auto-admin, multi-tenancy, audit log, auth (sessions/JWT/OAuth2), ViewSets + OpenAPI, jobs, email, S3 media, CSRF/CSP/rate-limit/idempotency middleware. First published April 2026, already at v0.46 after 41 releases in ~10 weeks, but ~680 total downloads and near-zero visibility. | The most plan-overlapping rival on paper — but 41 releases in 10 weeks with no adoption suggests breadth-first/possibly generated code over depth and community. **Do a proper teardown in week 1** (clone it, run its admin, try its migrations on a 20-edit model history): steal what works, and let its weaknesses (docs, tests, API stability, trust) define where Djangors out-executes. Also a warning: the space is being actively land-grabbed *right now* — reserve names and ship visibly early. |

**Standing discipline to stay #1 in refinement (make it a process, not a one-time check):**
1. **Quarterly competitive teardown** — re-run the Cot/Runique/loco feature matrix every release cycle; file a Djangors issue for any feature where a rival is ahead. Keep the matrix public in `docs/comparison.md` (honest comparisons build trust and rank on search).
2. **Refinement is depth, not feature count.** The differentiators no rival has *together*: full migration autodetection, an ops-team-usable admin, object-level permissions + audit + 2FA (the regulated-industry stack, Part 6), a DRF-equivalent with generated OpenAPI, background tasks, and Django-tutorial-quality docs. Rivals each have 2–3 of these at partial depth; Djangors's bar is all of them at production depth.
3. **Watch the watchers:** subscribe to Cot's and Runique's releases, This Week in Rust, and r/rust — respond to their launches with Djangors demo posts, not arguments.

---

## Part 2 — Foundational decisions (make them now, in writing)

| Decision | Choice | Rationale |
|---|---|---|
| Language edition / MSRV | Latest stable edition; MSRV = stable-minus-2, documented and CI-enforced | Serious orgs need MSRV guarantees. |
| License | MIT OR Apache-2.0 (dual) | Rust ecosystem standard; enterprise-safe. |
| Async runtime | Tokio, non-optional | Fighting runtime-genericity costs years. Ship on Tokio. |
| HTTP layer | hyper + tower + tower-http | Battle-tested; middleware = `tower::Layer`. |
| DB driver layer | sqlx (Postgres first, then SQLite, then MySQL/MariaDB) | Async, pure Rust, connection pooling included. Postgres first because banks/e-commerce run it; SQLite second because the tutorial and tests need zero-setup. |
| Template engine | MiniJinja core + Djangors filter/tag pack + template-dir loader with override precedence (app dirs → project dir), embedded via `include_dir!` in release | Runtime rendering is what makes the admin skinnable and dev loop instant. |
| Serialization | serde everywhere | Obvious. |
| Time | `chrono` + first-class timezone support (`USE_TZ = true` equivalent, store UTC) | Django got time handling right; copy it. |
| Money / decimals | `rust_decimal` as the default `DecimalField` backing type | Banking requirement. Floats for money = disqualified. |
| Password hashing | argon2id default; bcrypt/pbkdf2 verifiers for import-migration from Django apps | Also enables *migrating a real Django app to Djangors without password resets* — a killer adoption feature. |
| IDs | i64 autoincrement default, UUIDv7 opt-in per model or per project | Match Django default; offer the modern option. |
| Error model | `djangors::Error` with typed kinds; every error page in dev shows the Django-style yellow debug page (request info, template context, SQL log, backtrace) | The Django debug page is beloved. Clone it unapologetically. |
| Config | `DjangorsSettings` struct built from: defaults ← `djangors.toml` ← environment profile (`[profile.production]`) ← env vars ← code overrides | 12-factor-friendly, still one obvious place to look. |
| Name | **Djangors** — crate `djangors`, verified available on crates.io as of 2026-07-17 (`rango` and `rjango` are both taken by others). **Publish a 0.0.1 placeholder of `djangors` + `djangors-orm`, `djangors-admin`, `djangors-cli`, `djangors-macros` etc. TODAY** — name-squatting on Django-for-Rust names is demonstrably happening. Grab the GitHub org and a domain (djangors.rs / djangors.dev). CLI binary ships as `dj` (typed constantly: `dj new`, `dj run`) with `djangors` as an alias. **Considered and rejected: `django` / `django.rs`** — the `django` crate name is technically unclaimed, but "Django" is a registered trademark of the Django Software Foundation; naming a competing framework literally "Django" invites a takedown request and permanently brands the project as a clone rather than its own thing. If the DSF-adjacent branding matters, email the DSF trademark committee for written permission before using it — otherwise "Djangors" signals the lineage while remaining a distinct mark | An unclear name story at launch is a self-inflicted wound; settle it in week 1 and never revisit. |

---

## Part 3 — Architecture: the workspace

A single Cargo workspace. Users depend on the `djangors` facade crate and get everything via feature flags (all-on by default — batteries included).

```
djangors/
├── Cargo.toml                  # workspace
├── crates/
│   ├── djangors/                  # facade: re-exports, prelude, feature flags
│   ├── djangors-core/             # HTTP kernel: request/response, routing, middleware,
│   │                           # handlers, extractors, error pages, signals bus
│   ├── djangors-macros/           # ALL proc macros: #[derive(Model)], #[djangors::main],
│   │                           # q!(), urls!, #[handler], form derives
│   ├── djangors-db/               # backend abstraction over sqlx: connections, pools,
│   │                           # transactions, SQL dialect layer
│   ├── djangors-orm/              # Model trait, ModelMeta, QuerySet, managers,
│   │                           # relations, aggregation, expressions (F/Q objects)
│   ├── djangors-migrations/       # migration engine: state graph, autodetector,
│   │                           # operations, executor, squashing
│   ├── djangors-forms/            # Form/ModelForm, fields, widgets, validation, rendering
│   ├── djangors-template/         # MiniJinja integration, Django-flavored tags/filters,
│   │                           # loader precedence, {% url %}, {% static %}, {% csrf_token %}
│   ├── djangors-auth/             # User model, groups, permissions, backends, hashing,
│   │                           # login/logout views, decorators/guards
│   ├── djangors-sessions/         # session engine: signed-cookie, db, cache backends
│   ├── djangors-admin/            # THE admin: registry, changelist, forms, inlines,
│   │                           # actions, filters, search, theming
│   ├── djangors-staticfiles/      # collectstatic, hashed filenames, embed-in-binary,
│   │                           # dev serving
│   ├── djangors-cache/            # cache framework: in-memory, redis, db backends;
│   │                           # per-view + template-fragment caching
│   ├── djangors-mail/             # email backends: SMTP, console, file, in-memory (tests)
│   ├── djangors-i18n/             # translation catalogs (Fluent or gettext), locale
│   │                           # middleware, formats, {% trans %}
│   ├── djangors-test/             # test client, TestDatabase (per-test txn rollback),
│   │                           # fixtures, assertion helpers
│   ├── djangors-cli/              # the `djangors` binary: new/generate/run/etc.
│   ├── djangors-rest/             # DRF equivalent: serializers, viewsets, routers,
│   │                           # OpenAPI generation (Phase 8)
│   ├── djangors-channels/         # WebSockets/SSE, background workers, task queue (Phase 8)
│   └── contrib/
│       ├── djangors-contrib-messages/     # flash messages
│       ├── djangors-contrib-audit/        # model history / audit log (simple-history)
│       ├── djangors-contrib-guardian/     # object-level permissions
│       ├── djangors-contrib-otp/          # TOTP/WebAuthn 2FA
│       ├── djangors-contrib-sitemaps/
│       ├── djangors-contrib-syndication/  # RSS/Atom
│       ├── djangors-contrib-humanize/
│       └── djangors-contrib-flatpages/
├── examples/
│   ├── polls/                  # the tutorial app, always compiling in CI
│   ├── ecommerce/              # showcase: catalog, cart, orders, admin-heavy
│   └── school/                 # showcase: students, grades, RBAC, audit
├── docs/                       # mdBook or Astro Starlight site source
├── benchmarks/                 # criterion micro + TechEmpower-style macro benches
└── tools/                      # xtask: release automation, MSRV check, docs deploy
```

### The app system (Django's secret weapon, ported)

A Djangors **app** is a crate (or module) implementing `AppConfig`:

```rust
pub struct PollsApp;

impl AppConfig for PollsApp {
    const LABEL: &'static str = "polls";
    fn models(&self) -> Vec<&'static ModelMeta> { djangors::models_of!(polls) }
    fn urls(&self) -> Router { urls() }
    fn migrations(&self) -> &'static [&'static dyn Migration] { migrations::ALL }
    fn templates(&self) -> Option<TemplateSource> { Some(embed_templates!("templates")) }
    fn admin(&self, site: &mut AdminSite) { admin::register(site); }
}
```

The project's settings list installed apps; the **app registry** (built at startup) is what powers the admin, `makemigrations`, `{% url %}` reversing, and management commands — exactly Django's architecture, with the registry populated by proc-macro-generated metadata instead of Python introspection.

### What developer code looks like (write this before writing the framework)

**Author `examples/polls` FIRST, as aspirational code that doesn't compile yet.** This is your API spec. Every framework feature exists to make this file compile and run.

```rust
// models.rs
use djangors::prelude::*;

#[derive(Model)]
#[djangors(app = "polls", ordering = "-pub_date")]
pub struct Question {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 200)]
    pub question_text: String,
    #[djangors(verbose_name = "date published", db_index)]
    pub pub_date: DateTime<Utc>,
}

#[derive(Model)]
#[djangors(app = "polls")]
pub struct Choice {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(to = Question, on_delete = "cascade", related_name = "choices"))]
    pub question: ForeignKey<Question>,
    #[djangors(max_length = 200)]
    pub choice_text: String,
    #[djangors(default = 0)]
    pub votes: i32,
}

// views.rs
#[handler]
async fn index(req: Request) -> Result<Response> {
    let latest = Question::objects()
        .filter(q!(pub_date__lte = Utc::now()))
        .order_by("-pub_date")
        .limit(5)
        .all(req.db())
        .await?;
    render(&req, "polls/index.html", context! { latest_question_list => latest })
}

#[handler]
async fn vote(req: Request, Path(question_id): Path<i64>) -> Result<Response> {
    let question = Question::objects().get_or_404(req.db(), question_id).await?;
    let choice_id: i64 = req.form().await?.require("choice")?;
    Choice::objects()
        .filter(q!(question = question.id, id = choice_id))
        .update(req.db(), set!(votes = F("votes") + 1))   // race-safe, like Django's F()
        .await?;
    redirect(reverse!("polls:results", question_id))
}

// urls.rs
pub fn urls() -> Router {
    Router::app("polls")
        .route("", get(index), "index")
        .route("{question_id}/", get(detail), "detail")
        .route("{question_id}/vote/", post(vote), "vote")
}

// admin.rs
pub fn register(site: &mut AdminSite) {
    site.register::<Question>(
        ModelAdmin::new()
            .list_display(&["question_text", "pub_date", "was_published_recently"])
            .list_filter(&["pub_date"])
            .search_fields(&["question_text"])
            .inlines(&[Inline::<Choice>::tabular().extra(3)]),
    );
}

// main.rs
#[djangors::main]
async fn main() -> Result<()> {
    Djangors::new(settings())
        .app(PollsApp)
        .run()          // also dispatches CLI subcommands: migrate, createsuperuser…
        .await
}
```

---

## Part 4 — The five hard problems (solve on paper before coding)

These are where "Django for Rust" attempts die. Each gets a design doc in `docs/design/` before implementation.

### 4.1 Model metadata without reflection
Python's admin/migrations/forms run on runtime introspection. Rust has none.
**Solution:** `#[derive(Model)]` generates, per model: (a) typed field accessors and a `Fields` struct for query building; (b) a `&'static ModelMeta` — field names, column types, verbose names, validators, widget hints, relations — registered into the app registry. `ModelMeta` is the single source of truth consumed by migrations, admin, forms, and serializers. This is the keystone crate; over-invest here.

### 4.2 QuerySet ergonomics
Django's `filter(pub_date__lte=now, author__name__icontains="x")` is stringly-typed magic.
**Solution:** the `q!()` proc macro — Django's exact lookup syntax (`field__lookup`, `relation__field`), but *validated at compile time* against the generated `Fields` metadata: typo'd field or wrong value type = compile error. Under the hood it builds the same expression tree as the fluent typed API (`Question::fields().pub_date().lte(now)`), which also exists for people who want pure-Rust style and IDE completion. Support: `Q` combinators (`q!(a) | q!(b)`, `!q!(c)`), `F` expressions, `annotate`/`aggregate` (Count/Sum/Avg/Min/Max), `select_related` (JOIN) / `prefetch_related` (second query), `values()`/`values_list()` (row structs), slicing, `exists`, `count`, `update`, `delete`, `bulk_create`, `get_or_create`, `iterator` (streaming). Lazy: a `QuerySet` is a pure value until `.all()/.get()/.first()` executes it — chainable and reusable exactly like Django's.

### 4.3 Migrations with autodetection
`makemigrations` must diff "models as written" against "state after all existing migrations."
**Solution (Django's, transplanted):** migrations are Rust files declaring `Operation` lists (`CreateModel`, `AddField`, `AlterField`, `RunSQL`, `RunRust`), each operation able to mutate an in-memory `ProjectState`. The autodetector replays all migrations to build historical state, gets current state from the app registry (`ModelMeta`), diffs, and emits a new migration file (generated Rust source, `cargo fmt`-ed). Because migrations live *in the compiled binary*, `app migrate` works on a production box with no source tree. Include from day one: dependency graph between apps' migrations, `sqlmigrate` (print SQL), `--check` for CI, `--fake`, squashing, and data migrations via `RunRust` closures with a lightweight historical-model API (document its limits honestly — this is Django's hairiest corner; it's acceptable for v1 to give data migrations the *current* model API with loud docs).

### 4.4 The admin without a dynamic language
**Solution:** the admin is a runtime engine over `ModelMeta` + `ModelAdmin` config: it builds changelists (pagination, sorting, filters, search, date hierarchy, bulk actions), change forms (via djangors-forms, from field metadata + widget hints), inlines, delete-confirmation with related-object collection (mirror Django's `NestedObjects`), history integration, and permission checks — all rendered through overridable MiniJinja templates. Ship a clean, modern default theme (server-rendered + sprinkles of vanilla JS/htmx-style progressive enhancement; **no SPA framework dependency**). "Angular-level management" is achieved by making the admin genuinely rich: saved filters, CSV/XLSX export action built-in, column choosing, dark mode, mobile-usable. Admin URLs, templates, and forms all overridable per-model — parity with Django's extension points (`get_queryset`, `readonly_fields`, custom actions, custom admin views).

### 4.5 The dev loop (Rust compiles; Django reloads)
This is the #1 DX risk. Attack it from five directions:
1. **Runtime templates + static files** — the majority of iteration (HTML/CSS) needs zero recompile; `djangors run` watches and the browser auto-reloads (built-in livereload websocket in dev).
2. **`djangors run`** = cargo-watch equivalent: watches source, rebuilds, restarts, holds requests during restart (no connection-refused flashes), preserves sessions (signed cookies survive restarts).
3. **Fast dev profile out of the box:** generated projects ship a tuned `[profile.dev]` (opt-level 0, incremental, optional cranelift backend, optional mold/lld linker) — target: **sub-3-second rebuild** for handler-level edits on the tutorial app. Measure this in CI as a regression test.
4. **Workspace splitting guidance:** generators put apps in separate crates so touching one app recompiles one crate.
5. **Debug error page** with full context (Django's yellow page), SQL query log panel, and a debug-toolbar contrib crate later.

---

## Part 5 — Phased build plan

Phases are sequential dependencies, not calendar promises. Each phase has a **Definition of Done** (DoD) and ends with the polls example exercising every new feature. Rough effort assumes 1–2 experienced Rust devs; scale accordingly.

### Phase 0 — Bootstrap (days, not weeks)
- [ ] `git init`; workspace `Cargo.toml`; empty crates for core/macros/orm/db/cli with `#![deny(missing_docs)]` policy decided.
- [ ] Reserve crates.io names, GitHub org, domain, chat server. Check `djangors` name availability/conflicts *first*; have a backup name.
- [ ] CI (GitHub Actions): fmt, clippy (deny warnings), test matrix (linux/mac/windows × stable/MSRV), docs build, examples build. cargo-deny for licenses/advisories.
- [ ] `docs/design/` RFC template; write RFCs for the five hard problems (Part 4).
- [ ] Write the aspirational `examples/polls` (Part 3) — the API spec.
- [ ] LICENSE (MIT/Apache-2.0), CODE_OF_CONDUCT, CONTRIBUTING, SECURITY.md (with a security-report email), README with the pitch.

**DoD:** `cargo build` green in CI on empty crates; polls example exists as spec; 5 design docs drafted.

### Phase 1 — HTTP kernel (djangors-core) (~4–6 weeks)
- [ ] Request/Response types over hyper; body handling, multipart, form parsing, JSON, file uploads (streaming to temp files, size limits).
- [ ] Router: Django-style path syntax (`{id}`, converters `{id:i64}`, `{slug:slug}`), app namespacing, include/nesting, **named routes with `reverse!()`** (compile-time-checked route names where possible).
- [ ] Handler trait + extractors (Path, Query, Form, Json, State) — axum-familiar but Django-flavored.
- [ ] Middleware = tower `Layer`; ship: logging, security headers, common (slash-appending/redirects, à la Django `CommonMiddleware`), gzip/brotli, request-id.
- [ ] Settings system (Part 2 design) + `runserver` bound into a `Djangors` application object that owns the app registry.
- [ ] Error handling: `Result<Response>`, Http404/Http403/Http400 types, custom error handlers, the **dev debug page**, production error pages.
- [ ] Signals bus (typed, async): request_started/finished, plus the plumbing model signals will use later.
- [ ] Structured logging via `tracing` with pretty dev output.

**DoD:** a hello-world Djangors app with routing, middleware, templates NOT yet — plain responses; benchmarked ≥ axum-minus-10% throughput.

### Phase 2 — ORM + migrations (djangors-db, djangors-orm, djangors-migrations, djangors-macros) (~3–4 months; the long pole)
- [ ] djangors-db: sqlx integration, pool config in settings, `DATABASES`-style multi-db support with `.using("replica")`, transactions (`db.transaction(|tx| …)`), explicit isolation levels, savepoints.
- [ ] `#[derive(Model)]` + `ModelMeta` (design 4.1). Field types: all Django fields — Char/Text/Integer(s)/BigInt/Float/**Decimal**/Boolean/Date/DateTime(tz-aware)/Time/Duration/UUID/Email/URL/Slug/IP/Binary/JSON/File/Image; null/blank/default/choices/unique/db_index/validators/help_text/verbose_name; Meta: table name, ordering, unique_together, indexes, constraints (CheckConstraint!), permissions.
- [ ] Relations: ForeignKey (with on_delete: cascade/protect/set_null/restrict/do_nothing), OneToOne, ManyToMany (auto through-table + custom through), reverse accessors, related managers.
- [ ] QuerySet engine (design 4.2) incl. aggregation, expressions, subqueries (`Exists`, `OuterRef`), `select_for_update` (banking!), raw SQL escape hatch with type-checked rows via sqlx.
- [ ] Model lifecycle: `save()` (insert-or-update semantics), `delete()` with cascade collection, `full_clean()` validation, pre/post save/delete signals.
- [ ] Migration engine (design 4.3): operations, state replay, autodetector, executor, `djangors_migrations` table, all CLI verbs (`makemigrations`, `migrate`, `sqlmigrate`, `showmigrations`, `--check/--fake/--plan`).
- [ ] Backends: **Postgres** complete → **SQLite** complete (with its ALTER TABLE table-rebuild workarounds, which Django also does) → MySQL after 1.0.
- [ ] Test harness for the ORM itself: run the full ORM suite against real Postgres + SQLite in CI (containers).

**DoD:** polls models work end-to-end: makemigrations → migrate → query in views; a torture-test app with every field type and relation migrates cleanly forward from 20+ sequential model edits.

### Phase 3 — Templates, forms, static files (~6–8 weeks)
- [ ] djangors-template: MiniJinja engine; loader precedence (project `templates/` overrides app templates — this is what makes admin theming work); auto-escaping on; Django filter pack (date, floatformat, pluralize, humanize hooks, etc.); tags: `{% url %}`, `{% static %}`, `{% csrf_token %}`, `{% now %}`; template inheritance is native Jinja. `render()` shortcut with `RequestContext` (auto-injects user, messages, csrf).
- [ ] Dev: template hot reload. Release: `include_dir!` embedding.
- [ ] djangors-forms: `Form` derive + `ModelForm` (generated from ModelMeta); field validation pipeline (`clean_<field>` hooks, `clean()` cross-field), typed `cleaned_data`, error rendering, widgets (all Django widgets incl. select/radio/checkbox/date pickers), `as_div/as_table` renderers + template-based rendering, formsets (admin inlines need them), file-upload fields.
- [ ] djangors-staticfiles: dev serving, `collectstatic`, hashed-manifest storage (cache-busting), embed-in-binary option.

**DoD:** polls has real templates and a hand-written form with validation errors rendering; static CSS served in dev and embedded in release.

### Phase 4 — Sessions, auth, security (~6–8 weeks) — *the banking phase*
- [ ] djangors-sessions: engines — signed-cookie (default dev), database, cache; rotation, expiry, `session.set/get` typed API.
- [ ] CSRF middleware: token generation, header validation **(done)**, form-field (`csrfmiddlewaretoken`) fallback validation **(done, core-level — 5.8.9, commit `8a448db` — and wired into every admin `<form>` — 5.8.10, commit `b2c5d16`)**, template tag, exempt marker. On by default.
- [ ] Security middleware: HSTS, X-Content-Type-Options, Referrer-Policy, CSP helper, secure/httponly/samesite cookie defaults, host header validation (`ALLOWED_HOSTS`).
- [ ] djangors-auth: `User` model (swappable via setting, like `AUTH_USER_MODEL` — design this in *now*, Django learned the hard way it can't be retrofitted), groups, model-level permissions (auto add/change/delete/view per model), authentication backends trait, argon2id hashing + legacy verifiers, login/logout/password-change/password-reset views + email flow, `login_required` guard/extractor (`Auth<User>` extractor pattern), permission guards, session fixation protection (rotate on login).
- [ ] Rate limiting middleware (login throttling default-on), constant-time credential comparison, audit hooks for login/logout/failed-login signals.
- [ ] Security review checklist doc; threat model doc; fuzz targets for parsers (multipart, cookies, query strings).

**DoD:** polls requires login to vote; full password-reset flow works via console email backend; OWASP top-10 self-assessment written.

### Phase 5 — THE ADMIN (~3–4 months, worth every week)

> **Status (2026-07-18):** DoD met (5.9). Every top-level bullet below has now landed at least a
> "done (v1)" pass, including the full 5.8 arc (theming/history/extension points, 5.8.1–5.8.12).
> What remains across the board is optional, non-blocking follow-up work (choice-field filters,
> autocomplete/M2M widgets, inlines, full `on_delete` enforcement beyond `Protect`, generic
> `AuthUser` support) — see `docs/design/phase-5-roadmap.md` for the authoritative slice-by-slice
> status, commit references, and the full deferred-items ledger.

- [x] AdminSite + registry; auto-URLs; login gated by `is_staff`. *(5.1, commit 9edf249 — v1 constraint: gate hardcoded to the built-in `User`, not generic `AuthUser`)*
- [ ] Changelist: **done:** all-fields columns, sorting, pagination *(5.2, commit ac647f9)*; `list_display` (real-field subset/reorder) + `search_fields` (ILIKE search box) via new `AdminSite::register_with()` customization API *(5.6.1, commit a6ad28f)*; `list_filter` v1, Boolean fields only *(5.6.2, commit cba2109 — choices-based filtering blocked on `FieldMeta` gaining choices metadata, which doesn't exist yet)*; bulk delete, single hardcoded action with two-step confirm *(5.6.3, commit 47459ee)*; `date_hierarchy` v1, one `DateTime` field, year/month/day drilldown, drilldown counts not combined with search/`list_filter` state *(5.6.4, commit a744250)*; `list_editable` v1, text/numeric fields only, shares the bulk-delete `<form>` via per-button `formaction` (no JS, no nested forms) *(5.6.5, commit 27bce9e)*; CSV export v1, plain `GET` link exporting the current filtered/searched/ordered queryset (not a selected-rows action), hand-rolled RFC 4180 escaping, no new dependency *(5.6.6, commit 6f49e30)*; `list_display` computed-method columns (`ModelAdminConfig.computed_columns`, a function of the row's field values, checked before falling back to a real field) and a real bulk-actions dispatch mechanism for selected-rows actions (`AdminAction` + `POST bulk-action/` with a two-step confirm page, same `formaction`-per-button pattern 5.6.5 established) *(5.8.12, commit `8d0c683`)*. **remaining:** `list_filter` for choice fields (blocked as above), saved views.
- [ ] Change form: **done (v1):** add + edit pages, all fields, server-side validation with all-errors-at-once reporting, changelist row links *(5.4, commit 9f016fe)*; fieldsets (rows grouped under `<fieldset><legend>` in registration order), `readonly_fields` (rendered as inert text), `raw_id_fields` (a plain lookup link to the related model's changelist rather than a full `<select>`) *(5.8.12, commit `8d0c683`)*. **remaining:** autocomplete FK widgets with an async search endpoint (raw_id_fields v1 is a static link, not a live-search picker), M2M widgets, inlines (tabular/stacked) via formsets. *(CSRF header-only gap fixed at the core-middleware level, 5.8.9, commit `8a448db`, and the admin's own `<form>` elements now emit the hidden `csrfmiddlewaretoken` field, 5.8.10, commit `b2c5d16` — a raw browser form POST works end-to-end without client-side JS)*
- [ ] Delete confirmation: **done (v1):** confirm page + delete action, one-hop related-object counts via the `inventory`-based global model registry, `on_delete` shown for information only *(5.5, commit aba0ff9)*; permission enforcement — `require_perm` gates GET/POST delete and bulk-delete on the `delete_{model}` codename, superusers bypass *(5.7.2, commit b9aed08)*; transitive (multi-hop, depth-bounded + cycle-safe) related-object walk, and `on_delete: Protect` enforcement — single-object delete now 400s and bulk delete silently skips protected pks instead of deleting them *(5.8.12, commit `8d0c683`)*. **remaining:** real DB-level (or ORM-level) enforcement for the other `on_delete` variants (Cascade/SetNull/Restrict/DoNothing are still metadata-only — see roadmap ledger); only `Protect` is enforced, and only as an admin-layer pre-check, not a schema constraint.
- [x] Permissions: **done:** `Permission`/`Group`/`UserGroup`/`GroupPermission`/`UserPermission` models (plain FK join tables, no real many-to-many ORM support needed), `AuthUser::is_superuser()`, `has_perm(db, user_id, codename)`, `sync_permissions()` + `dj createpermissions` *(5.7.1, commit bb284a4)*; every admin view (changelist/export/add/change/list_editable save/delete/bulk-delete) now checks `has_perm` per action via `require_perm` instead of a blanket `is_staff` check, superusers bypass entirely, `admin_index` filters to only viewable models *(5.7.2, commit b9aed08)*; admin UI for groups/permissions/user assignments — `Permission`/`Group`/`UserGroup`/`GroupPermission`/`UserPermission` registered with the generic admin like any other model, real add/change/delete/changelist CRUD, no new admin-crate mechanism needed — and UI-level hiding of action buttons (`has_add/change/delete/view_permission` booleans threaded into every template context) for a user lacking the relevant permission *(5.8.12, commits `8d0c683`/`1305195`)*. **remaining:** `AuthUser`-generic (not hardcoded to `djangors_auth::User`) support, same standing limitation as the 5.1 staff gate.
- [x] History: **done (v1):** admin log entries table (`LogEntry`, one entry per successful add/change/delete, per-user "Recent actions" panel on the admin index) *(5.8.11, commit `01cd081`)*; per-object History page (`GET /{app}/{model}/{pk}/history/`, newest-first, joined against `auth_user` for the acting username) *(5.8.12, commit `8d0c683`)*. **remaining:** real field-level diffing/`change_message` beyond fixed action strings, IP/request-id capture; deep integration with djangors-contrib-audit for full record diffing.
- [x] Theming: clean modern default (CSS custom properties, dark mode — done, 5.8.7); every template overridable; per-site branding (title/heading — done, 5.8.8; logo, accent color — done, 5.8.10) via settings. *(Conversion mechanism complete as of 5.8.6 — every admin page renders through minijinja: `admin_index` (5.8.1, eb1a916), delete-confirm (5.8.2, dfe6ec0), bulk-delete confirm (5.8.3, 965fb24), `list_editable` save-error page (5.8.4, 8b7dce5), add/change form (5.8.5, 55e54b5), changelist (5.8.6, ab1363e). 5.8.7 (cdd64bf) added a real HTML5 page shell (`admin/base.html`), CSS custom properties, and automatic dark mode. 5.8.8 (ad23e39) added `SiteBranding` (`site_header`/`site_title`) threaded through `AdminSite::urls()`'s existing per-closure-snapshot mechanism into every template-rendering handler — `AdminSite::new()`'s signature unchanged, opt-in via `with_site_header`/`with_site_title` builders. 5.8.10 (b2c5d16) added `logo_url`/`accent_color` via `with_logo_url`/`with_accent_color`, same mechanism, plus default favicons served at the app root.)*
- [x] Extension points: **done (v1):** `AdminSite::extra_route()` lets a host app mount an arbitrary extra `Router` under the admin site (custom admin views); `ModelAdminConfig.base_filter` ANDs a fixed `UnresolvedExpr` into every changelist/queryset for a model (a scoped `get_queryset` override); custom actions with an intermediate confirmation page via `AdminAction`/`requires_confirm` (this slice's own bulk-actions mechanism, above); `ModelAdmin` is already a trait with `DefaultModelAdmin` providing every default *(5.8.12, commit `8d0c683`)*. **remaining:** a full `get_queryset` override (an arbitrary closure, not just a fixed `base_filter`) if a future use case needs one.
- [x] `createsuperuser` command. *(5.3, commit 9b2fd47 — non-interactive only: `--username`/`--email` + `DJANGORS_SUPERUSER_PASSWORD` env; interactive prompting deferred)*

**DoD: met.** `examples/school` (`Student`/`Course`/`Enrollment`) runs its entire back-office through the admin with zero custom CRUD views — only `login`/`logout` plus the mounted `AdminSite`, registered with real `list_display`/`search_fields`/`list_filter`/`date_hierarchy`/`list_editable` customization. Proven end-to-end by a real socket-level integration test (`examples/school/tests/admin_crud.rs`) exercising add/changelist/grade-edit/delete through actual HTTP against the full middleware stack, each step verified against real DB state *(5.9, commit 259c448)*.

### Phase 6 — CLI & developer experience (~6–8 weeks, parallel with 5)
- [x] `djangors` binary: **done (v1):** `dj new <project>` — generates a standalone crate (Cargo.toml/main.rs/views.rs/.gitignore/djangors.toml/README.md/Dockerfile/deploy/djangors.service), path-dependency-wired against the same checkout `dj` was built from (crates.io versions blocked on `cargo login`), `git init`-ed, builds and serves a welcome page plus a `/healthz` route with zero config *(6.1, commit `85d2b92`; deployment scaffolding added in 6.3, commit `0e894e5`)*; `dj new-app <name>` — generates an app as a Rust **module** inside the current project's `src/`, not a separate crate (a deliberate v1 scope decision — the project is a single standalone crate, not a workspace, so a real "app crate wired into the registry" needs workspace-conversion surgery not attempted here); `dj run` — hand-rolled watch/build/restart loop (no new dependency), keeps watching rather than crashing on a failed build; `dj check [--deploy]` — reuses `DjangorsSettings::validate()` plus basic project-structure sanity, deliberately cannot check models/migrations (that data only exists at runtime inside a project's own compiled binary, not introspectable by the external `dj` tool — a real version of this needs a different mechanism); `--deploy` adds production-readiness checks (DEBUG=false, SECRET_KEY present and >=32 chars, ALLOWED_HOSTS not left at the default) *(new-app/run/check: 6.2, commit `d027cee`; `--deploy`: 6.3, commit `0e894e5`)*. **remaining:** true livereload (browser auto-refresh, not just process restart), model/migration-aware checks (see `makemigrations` below for why that's structurally hard, not just undone). Graceful Ctrl-C child cleanup for `dj run`'s own spawned dev-server child is a separate, smaller known gap from the server-level graceful shutdown below (6.4) — `dj run`'s watch loop still uses a hard kill on rebuild.
- [x] Project-binary subcommands (via `Djangors::run()` dispatch): **done:** `migrate`, `createsuperuser`, `createpermissions`, `collectstatic` (all pre-6.x); `makemigrations [--check]` — investigated and confirmed the external `dj` binary structurally cannot see a target project's `inventory`-registered models (registration is per-binary-image; `dj` is a different binary than any project it operates on) — rather than a half-working feature, it returns a clear error explaining the binary-boundary limit; real incremental migration generation needs a fundamentally different mechanism, a separate future design question, not a v1 gap to fill in later the same way; `dbshell` — execs `psql` with the `DATABASE_URL` passed directly as a connection URI (confirmed empirically that libpq's URI-argument form works in this environment); `test` — thin wrapper around `cargo test`, propagating its real exit code; `shell` — connects to `DATABASE_URL` to prove connectivity, then honestly reports no real REPL exists yet (needs `evcxr`, a separate future project) rather than faking one, pointing at `dbshell` for interactive DB work today *(6.3, commit `0e894e5`)*. **remaining:** custom management commands API.
- [x] djangors-test: **done (v1):** `TestClient` (wraps a `Router`, in-process `.handle()`, ergonomic `get`/`post_form`/`with_session`/`with_state` builder, `TestResponse` with `assert_status`/`assert_contains`) and `TestDatabase` (thin `create_table`/`drop_table`/`reset` wrappers around the exact DROP/CREATE-TABLE boilerplate duplicated across every existing test suite) *(6.2, commit `d027cee`)*. **remaining, deliberately not attempted:** true per-test transactional DB rollback — blocked on `djangors-orm`'s `QuerySet` methods all taking a hardcoded `&Database`/pool rather than an injectable executor/transaction type, a real, separate, larger ORM change; fixtures (JSON/YAML loaddata + Rust factory helpers); `assertRedirects`/`assertTemplateUsed` equivalents; snapshot testing.
- [x] Deployment story doc + generators: **done (v1):** every `dj new`-generated project now ships a multi-stage `Dockerfile` (`rust:1-slim` builder + `debian:bookworm-slim` runtime, `HEALTHCHECK` against `/healthz` — a real working v1, not the ~20MB distroless image this bullet's text aspirationally names, noted explicitly rather than silently claimed), a systemd unit template (`deploy/djangors.service`), a `/healthz` route, and `dj check --deploy` as a pre-flight settings check *(6.3, commit `0e894e5`)*; **graceful shutdown is now done too** — `Djangors::serve`/`serve_service` (and therefore `run`/`run_service`, unchanged call sites) now listen for SIGINT/SIGTERM and drain in-flight connections (bounded by a 30s timeout) before exiting, via a shared internal accept-loop helper and a `tokio::task::JoinSet` replacing the previous fire-and-forget `tokio::task::spawn`; new lower-level `serve_with_shutdown`/`run_with_shutdown` (+ `_service` variants) take an arbitrary shutdown future for testability *(6.4, commit `d1d5e0c`, deliberately isolated from 6.3 given its blast radius across every app's request-serving loop — verified via a real timing-based test proving an in-flight request survives shutdown, plus both example apps' full integration tests explicitly re-run)*. **remaining:** a distroless (vs. slim-Debian) runtime image; a proper deployment story *doc* (the generators exist now, the accompanying prose doc doesn't yet).

**DoD:** `djangors new mysite && cd mysite && djangors run` → welcome page in under 90 seconds on a clean machine (after crates cache); tutorial-app edit-to-reload under 3s. **Partially met:** `dj new mysite && cd mysite && cargo run` works today with zero config (6.1); `dj run`'s own watch/rebuild loop works end-to-end (6.2, verified via a real generated-project rebuild-and-reserve test) but hasn't been benchmarked against the literal "under 90 seconds on a clean machine" / "edit-to-reload under 3s" timing claims — do that before calling the DoD fully met. **Phase 6 is otherwise functionally complete** — every top-level bullet has at least a "done (v1)" pass; what's left is optional polish (livereload, distroless images, custom management commands, the prose deployment doc) and the literal DoD timing benchmark.

### Phase 7 — Batteries (contrib) (~2–3 months, parallelizable, community-friendly)
- [x] djangors-cache: **done (v1):** `Cache` trait (`get`/`set`/`delete`/`get_or_set`, object-safe raw-byte values + a `CacheExt` for typed JSON), `InMemoryCache` (moka), `DatabaseCache` (reuses `djangors_db::Database`, lazily created table), `RedisCache` (optional `redis` Cargo feature, not default), `CacheLayer` tower middleware (explicit opt-in via a `CacheableResponse` marker, never blanket-caches every GET — a deliberate security decision), template fragment caching via a plain `get_or_set_fragment` helper (not a new minijinja tag) *(7.1, commit `c87a76d`)*. **remaining:** a real `{% cache %}` template-tag syntax if minijinja's custom-tag support turns out to make that worthwhile later.
- [x] djangors-mail: **done (v1):** extends the Phase-4-minimal console-only crate — `SmtpBackend` (`lettre`, real TLS), `FileBackend` (one file per message), `InMemoryBackend` (test inspection), `Message.html_body` for real `multipart/alternative` HTML+text mail *(7.1, commit `c87a76d`)*.
- [x] djangors-contrib-messages: **done (v1):** per-session flash-message queue (`add`/`take`, `Level::{Debug,Info,Success,Warning,Error}`) built on `djangors_sessions::Session`'s existing get/set/remove — template integration is documented with an example, not auto-wired (no generic way to discover a project's own render call sites) *(7.2, commit `168dd5c`)*.
- [ ] djangors-i18n: message extraction from templates+code, Fluent-based catalogs, locale middleware, `{% trans %}`, localized formats; l10n of dates/numbers.
- [ ] djangors-contrib-audit: every model change recorded (who/when/what diff) — **default-on in generated projects**; admin timeline view. Banking table stakes.
- [ ] djangors-contrib-guardian: object-level permissions integrated with admin + auth guards.
- [ ] djangors-contrib-otp: TOTP enrollment + verification, admin 2FA enforcement setting; WebAuthn stretch.
- [ ] Sitemaps, syndication, flatpages, redirects app.
- [x] Humanize: **done (v1):** `intcomma`/`filesizeformat`/`naturaltime` template filters added to `djangors-template` *(7.2, commit `168dd5c`)*.
- [x] Pagination utility (shared by admin + user code): **done:** `djangors_core::Paginator` extracted from djangors-admin's previously-inline math, admin now consumes it, all 30 pre-existing admin tests pass unmodified *(7.2, commit `168dd5c`)*.

**DoD:** e-commerce example uses cache, messages, audit, and mail end-to-end.

### Phase 8 — API & real-time (~2–3 months)
- [ ] djangors-rest: serializers derived from ModelMeta (+ manual), ViewSets + routers, auth (session + token + JWT via feature), permissions classes, throttling, pagination, filtering integration with QuerySet, **OpenAPI 3.1 generation** from the type system (Rust's edge: schemas are *actually correct*), browsable API page.
- [ ] djangors-channels: WebSocket handlers with auth/session access, SSE, groups/broadcast (in-process + Redis backplane).
- [ ] Background tasks: `#[task]` functions, DB-backed queue (Postgres SKIP LOCKED) default + Redis backend, scheduled tasks (cron syntax), admin visibility into queue. (Django never shipped this and everyone needs it — ship it.)

**DoD:** e-commerce exposes a versioned JSON API with generated OpenAPI docs; order-status page updates live over WebSocket; emails send via background task.

### Phase 9 — Docs, tutorial, website (starts Phase 1, finishes never)
- [ ] Docs site (Starlight or mdBook): **Tutorial parts 1–8** (mirror Django's polls tutorial structure exactly — it's the best framework tutorial ever written), topic guides (ORM, templates, forms, auth, admin, testing, deployment, security), reference (rustdoc, 100% public-API coverage enforced), how-tos, and a **"Djangors for Django developers"** side-by-side translation guide (your single biggest adoption lever).
- [ ] Every doc code block extracted and compiled in CI (doctests / skeptic-style).
- [ ] `djangors.rs` (or chosen domain) landing page: pitch, live admin demo (deployed e-commerce example, read-only), benchmark numbers, "deploy in 5 minutes" screencast.

**DoD:** a Django dev completes the tutorial without asking a question in chat.

### Phase 10 — Hardening & 1.0 (~2–3 months)
- [ ] Benchmarks published honestly: vs Django (expect 10–50x), vs axum/actix (target: within 15–25% on full-stack paths, and say why the gap buys you the admin), TechEmpower submission.
- [ ] Load testing the admin + ORM under concurrency; connection-pool tuning guide.
- [ ] Third-party **security audit** of auth/sessions/CSRF/admin (budget for it; publish results — enormous credibility with your banking audience).
- [ ] API freeze review: go over every public item; `#[doc(hidden)]` or seal what you're unsure of. Deprecation policy + release cadence doc (time-based, like Django's).
- [ ] 1.0 launch: blog post, HN/Reddit/This Week in Rust, conference talk submissions (RustConf, EuroRust, DjangoCon — yes, DjangoCon).

**DoD:** semver 1.0 with a written stability contract; audit published; three example apps deployed live.

---

## Part 6 — What "banking / schools / e-commerce grade" concretely requires

Bake these into the phases above; this list is the acceptance criteria for the pitch:

1. **Money is `Decimal`, never float**; currency-aware field in contrib.
2. **`select_for_update`, explicit isolation levels, savepoints** — documented double-entry-ledger example in the docs.
3. **Audit trail default-on** (contrib-audit): who changed what, when, from which IP/request-id; immutable log table option.
4. **Idempotency-key middleware** for POST APIs (e-commerce checkout, payment webhooks).
5. **Object-level permissions + row scoping** (a teacher sees only their class; a branch officer only their branch) with a documented pattern.
6. **2FA for admin users**, session timeout policies, password policy settings, login throttling.
7. **PII tooling:** field-level encryption-at-rest helper, `#[djangors(sensitive)]` masking in logs/error pages/audit diffs.
8. **Compliance posture docs:** how Djangors's defaults map to OWASP ASVS / SOC2 controls (auditors love a mapping table).
9. **Zero-downtime migrations guide** (additive-first patterns, `--check` in CI, lock-timeout settings for Postgres DDL).
10. **Observability:** `tracing` spans per request/query out of the box, OpenTelemetry exporter feature, slow-query log, `/health` + `/ready` endpoints.

---

## Part 7 — Project operations (do these alongside the code)

- **Cadence:** ship a tagged 0.x release every 4–6 weeks from Phase 1 onward, each with a changelog and one demo GIF. Momentum is marketing.
- **RFC process:** any public-API change ≥ medium gets a short design doc + 48h comment window (even while it's just you — future contributors inherit the archive).
- **Issue hygiene:** `good-first-issue` labels from Phase 2 onward; contrib crates (Phase 7) are the designated community on-ramp.
- **Testing bar:** every crate ≥ 80% coverage; ORM and migrations get property-based tests (proptest: random model-edit sequences must always produce applicable migrations); fuzzing on all parsers.
- **Dogfooding:** the project website's own tiny CMS runs on Djangors as soon as the admin exists.
- **Funding path (later):** GitHub Sponsors → hosted admin/support offering → never gate core features.

## Part 8 — Top risks and their mitigations

| Risk | Mitigation |
|---|---|
| Scope drowning (this is 2+ person-years) | Phases are strictly ordered; the polls example is the scope-cutter — if polls doesn't need it before 1.0, it's post-1.0. |
| Compile times ruin the "pleasant" promise | Design 4.5 is a *tracked benchmark in CI*, not a hope. Runtime templates carry most iteration. |
| Proc-macro complexity explodes (djangors-macros) | Macros stay thin: they only *emit metadata and delegate* to runtime code in djangors-orm; all logic lives in normal, testable functions. Use `trybuild` for macro error-message tests — good compile errors are a feature. |
| ORM correctness bugs destroy trust with exactly your target users | Run the ORM test suite against real databases in CI from day one; port relevant chunks of Django's own ORM test suite (it's a free, battle-hardened spec). |
| "Yet another Rust web framework" dismissal | Never market the router. Market the **admin + migrations + batteries**. Demos, not benchmarks, lead every post. |
| Burnout | The 4–6-week release ritual + visible example apps give constant finish-lines. Recruit 1–2 co-maintainers by Phase 3. |

## Part 9 — Start-today checklist (first 7 days)

1. ✅ Name research done (2026-07-17): `rango` and `rjango` are taken; **`djangors` is free — publish placeholder crates (`djangors`, `djangors-orm`, `djangors-admin`, `djangors-cli`, `djangors-macros`, `djangors-forms`, `djangors-rest`) today.** Do NOT use `django`/`django.rs` without reading the DSF trademark policy first (see naming row).
2. Create GitHub org + repo, push this plan, add LICENSE/CoC/CONTRIBUTING/SECURITY.
3. Scaffold the workspace (Part 3 layout, empty crates) + CI green.
4. Write `examples/polls` as the aspirational API spec (Part 3 code).
5. Write design docs 4.1 (ModelMeta) and 4.2 (QuerySet) — these unblock everything.
6. Read `django/db/migrations/autodetector.py` and cot.rs's model macro; take notes into `docs/prior-art.md`.
7. Begin Phase 1: Request/Response over hyper + the router.
