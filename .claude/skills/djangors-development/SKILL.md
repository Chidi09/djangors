---
name: djangors-development
description: Use when building, editing, or reviewing code in a Djangors project — a Django-inspired, batteries-included Rust web framework (ORM, migrations, admin, forms, auth, REST framework, background tasks, typed settings, payments, multi-tenancy, PDF generation, i18n, static files). Trigger on `#[derive(Model)]`, `djangors-core`/`djangors-orm`/`djangors-rest`/`djangors-admin`/`djangors-tasks`/`djangors-contrib-*` in Cargo.toml, the `dj` CLI, or any Rust web project whose structure matches Django's (settings.py-style config, migrations, an admin site, ViewSets).
---

# Djangors development

Djangors mirrors Django's shape (models, migrations, admin, forms, auth, a DRF-equivalent REST
framework) but is Rust: a typo in a field name or a wrong type is a compile error, not a 2am page.
This skill is a quick-start map for an AI agent writing Djangors code — it inlines the handful of
idioms you need immediately and points at the real docs for depth. **Always verify against the
actual installed crate versions/source in the project you're working in** — this skill describes
the framework as of its own writing; a specific project may pin an older or newer version.

**Go deeper**: `docs/src/tutorial/` (8-part walkthrough), `docs/src/guides/*.md` (one per subsystem
— orm, forms, auth, admin, testing, deployment, security), `docs/src/django-comparison.md` (a
direct Django-to-Djangors API mapping table), `docs/src/api-stability.md` (what's frozen).

## Defining a model

```rust
use djangors_macros::Model;

#[derive(Model, Debug, Clone)]
#[djangors(app = "blog", table_name = "blog_post")]  // table_name optional, defaults from app+struct name
pub struct Post {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 200, unique, db_index)]
    pub title: String,

    pub body: String,                 // no max_length -> TEXT column
    pub view_count: i64,               // BigInt
    pub published: bool,
    pub published_at: chrono::DateTime<chrono::Utc>,

    #[djangors(default = 0)]           // bare literal, NOT a quoted string for non-Text defaults
    pub priority: i32,

    #[djangors(file_field)]            // String/Option<String> only — stores a Storage-relative path
    pub attachment: Option<String>,

    #[djangors(foreign_key(on_delete = "cascade", related_name = "posts"))]
    pub author: djangors_orm::ForeignKey<User>,
}
```

Common attributes: `primary_key`, `auto` (auto-increment), `max_length = N` (Char, else Text),
`unique`, `db_index`, `default = <bare literal>`, `file_field` (String-only), `foreign_key(on_delete
= "cascade"|"protect"|"set_null"|"restrict"|"do_nothing", related_name = "...")`. `default` on a
non-string field must be a bare literal (`default = 0`, `default = true`), not a quoted string —
that parses as text and silently produces the wrong `DefaultValue` variant.

## Saving vs updating

`save()` is INSERT-only — it always creates a new row (`auto` PK columns are DB-generated, so this
is correct for a fresh instance). Any row you already fetched or previously saved must go through
`update()` instead, which issues an `UPDATE ... WHERE pk = ...` and returns `OrmError::NotFound` if
no row matched. There's no automatic new-vs-persisted detection — track it yourself:

```rust
let post = post.save(&db).await?;   // first time — INSERT, returns the row with its DB-assigned pk
post.title = "edited".into();
post.update(&db).await?;            // already exists — UPDATE, calling save() here inserts a duplicate
```

## Querying

```rust
Post::objects().filter(djangors_orm::q!(published = true))?.order_by("-published_at")?.all(&db).await?;
Post::objects().select_related::<User>(&db, "author").await?;               // forward FK, 2 queries total
djangors_orm::prefetch_related::<User, Post>(&db, &authors, "posts").await?;  // reverse FK batch load, 1 query
```

`q!(field = value, other = value)` builds an equality filter (AND-combined); it does not support
`>`/`<`/ranges directly — for range/comparison filters use `filter_datetime_range` or read
`crates/djangors-orm/src/queryset.rs` for the `Expr`/`CompareOp` pattern used internally.

## Migrations

```
dj makemigrations   # diffs registered models against migrations/.schema_snapshot.json
dj migrate          # applies pending migrations against DATABASE_URL
```

v1 diffing covers new models (CREATE TABLE) and new fields (ALTER TABLE ADD COLUMN) only — field
type changes, removals, renames, and relation alterations are not yet auto-detected.

## REST API — ViewSet vs ScopedViewSet

```rust
// Unscoped — anyone authenticated sees everything:
djangors_rest::viewset_routes::<Post>(router, "/api/posts")

// Mandatory scoping (tenant isolation, row ownership, soft-delete) — a model that doesn't
// implement Scoped will not COMPILE against ScopedViewSet, a stronger guarantee than a runtime
// NotImplementedError check:
impl djangors_rest::Scoped for Post {
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
        let owner_id = /* extract from req state/session */;
        qs.filter(djangors_orm::q!(author = owner_id))
    }
}
djangors_rest::scoped_viewset_routes::<Post>(router, "/api/posts")
```

Reach for `ScopedViewSet` any time "every query through this endpoint must apply this filter, and
forgetting to isn't an option" — multi-tenancy, row-level ownership, soft-delete exclusion.

## Admin site

```rust
let site = djangors_admin::AdminSite::new();
site.register_with::<Post>(djangors_admin::ModelAdminConfig {
    list_display: &["title", "published", "view_count"],
    search_fields: &["title"],
    ..Default::default()
});
router.mount("/admin", site.urls())
```

## Background tasks

```rust
#[task]
async fn send_welcome_email(payload: WelcomePayload) -> Result<(), TaskError> { /* ... */ }

djangors_tasks::enqueue(&db, "send_welcome_email", &WelcomePayload { user_id }).await?;
djangors_tasks::enqueue_scheduled(&db, "send_welcome_email", &payload, run_at).await?; // future DateTime<Utc>

djangors_tasks::register_recurring(&db, "cleanup_sessions", &payload, "*/5 * * * *").await?; // standard 5-field cron
```

`tick_recurring_tasks(&db)` (call it from your own scheduler loop) advances a recurring task by
**exactly one** occurrence per call, not all the way to "now" — a schedule that's missed several
runs is caught up one occurrence at a time, with concurrent/successive callers each claiming one via
`SELECT ... FOR UPDATE SKIP LOCKED`. It never dumps a whole backlog of catch-up tasks in one call.

## Typed settings

```rust
#[derive(djangors_macros::Settings, Debug)]
#[djangors(prefix = "APP")]
struct AppSettings {
    api_key: String,                                          // required; env var APP_API_KEY
    #[djangors(default = "https://api.example.com".to_string())]
    base_url: String,                                         // APP_BASE_URL
    #[djangors(default = 30)]
    timeout_secs: u64,
    feature_flag: Option<bool>,                                // unset env var -> None, never an error
}
let settings = AppSettings::load()?;
```

Env vars are `{PREFIX}_{FIELD_NAME}` uppercased (or just `{FIELD_NAME}` with no `prefix`). A
non-`Option` field with no `#[djangors(default = ...)]` is required — a missing env var is
`SettingsError::MissingRequired`, not a silent default.

## Other contrib crates

`djangors-contrib-payments` — `PaymentProvider` trait + `PaystackProvider`, an idempotent
`Transaction` model, `handle_paystack_webhook`; amounts are always integer minor units, never a
float. `djangors-contrib-tenancy` — `Tenant`/`TenantMembership` models, `TenantResolutionLayer`
middleware, `tenant_scope()` for `Scoped` impls (see the REST section above). `djangors-pdf` —
`PdfDocument::new(title)` builder (`.heading()`/`.text()`/`.table()`/`.render()`). `djangors-deploy`
exists (`DeployProvider`, `RenderProvider`, an SSH provider) but is not yet wired to a `dj deploy`
CLI subcommand — use the crate directly.

## Rate limiting, cursor pagination, storage — opt-in, per-endpoint

```rust
// Named + scoped rate limiter (cache keys are prefixed by `name`, so two limiters never interfere):
let limiter = Arc::new(djangors_core::RateLimiter::new("login", ByIp, 5, Duration::from_secs(900), cache_backend));
djangors_core::rate_limited(limiter, my_handler)

// Cursor pagination: opt-in per ViewSetConfig, offset pagination is the unaffected default.
ViewSetConfig { cursor_pagination: true, orderable_fields: &["created_at"], ..Default::default() }

// Storage: LocalDiskStorage for dev, S3Storage for prod — same trait, swap the backend.
```

## The `dj` CLI (mirrors `manage.py`)

`new`, `new-app`, `run`, `check [--deploy]`, `migrate`, `makemigrations`, `createsuperuser`,
`createpermissions`, `dbshell`, `runworker` (starts the background-task worker loop), `shell` (a
real `evcxr` Rust REPL), `test`, `collectstatic`.

## Security defaults to know

CSRF (`csrf_layer()`) and session cookies (`SignedCookieStore`) both default `Secure` to `false`
(so local HTTP dev works out of the box) — call `.with_secure(!settings.debug)` on both in any
real deployment; see `docs/src/guides/security.md`. Password hashing is Argon2id. The `ByIp` rate-
limit key strategy is header-based and spoofable without a trusted reverse proxy in front of it —
its own doc comment says so.
