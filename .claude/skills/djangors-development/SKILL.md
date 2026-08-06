---
name: djangors-development
description: Use when building, editing, or reviewing code in a Djangors project — a Django-inspired, batteries-included Rust web framework (ORM, migrations, admin, forms, auth, REST framework, background tasks, typed settings, payments, multi-tenancy, PDF generation, i18n, static files). Trigger on `#[derive(Model)]`, `djangors-core`/`djangors-orm`/`djangors-rest`/`djangors-admin`/`djangors-tasks`/`djangors-contrib-*` in Cargo.toml, the `dj` CLI, or any Rust web project whose structure matches Django's.
---

# Djangors development

Djangors mirrors Django's shape (models, migrations, admin, forms, auth, a DRF-equivalent REST
framework) but is Rust: a typo in a field name or a wrong type is a compile error, not a 2am page.

**Go deeper**: `docs/src/tutorial/` (8-part walkthrough), `docs/src/guides/*.md` (one per subsystem),
`docs/src/django-comparison.md` (Django-to-Djangors mapping), `docs/src/api-stability.md`.

---

## CRITICAL: Trait import map

Many methods require a trait in scope. Forgetting these is the #1 compile error.

| Method/function | Trait to import |
|---|---|
| `model.objects()` | `use djangors_orm::Model;` |
| `serializer.to_representation()` | `use djangors_rest::Serializer;` |
| `perm.has_permission(&req)` | `use djangors_rest::Permission;` |
| `Json::from_request(&req)` / `Form::from_request(&req)` | `use djangors_core::extract::FromRequest;` |
| `provider.initiate(&req)` / `provider.verify(ref)` | `use djangors_contrib_payments::PaymentProvider;` |
| `current_user(&req)` | `use djangors_rest::permissions::current_user;` |
| `user(&req)` (returns `Result<User, Unauthorized>`) | `use djangors_rest::permissions::user;` |

---

## Defining a model

```rust
use djangors_macros::Model;

#[derive(Model, Debug, Clone)]
#[djangors(app = "blog", table_name = "blog_post")]
pub struct Post {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 200, unique, db_index)]
    pub title: String,

    pub body: String,                 // no max_length -> TEXT
    pub view_count: i64,             // BigInt
    pub published: bool,
    pub published_at: chrono::DateTime<chrono::Utc>,

    #[djangors(default = 0)]         // bare literal for non-Text defaults
    pub priority: i32,

    #[djangors(foreign_key(on_delete = "cascade", related_name = "posts"))]
    pub author: djangors_orm::ForeignKey<User>,
}
```

**Supported special field types** (all persisted through text/string serialization):
`uuid::Uuid`, `chrono::NaiveDate`, `chrono::NaiveTime`, `std::time::Duration`, and
`rust_decimal::Decimal`. `Decimal` requires `#[djangors(max_digits = N, decimal_places = N)]` or
`#[derive(Model)]` errors at compile time. `NaiveDate` maps to a `Date` column kind.

**`Option<ForeignKey<T>>` is not supported** — the macro only matches bare `ForeignKey<T>`. Use
`Option<i64>` for nullable FK columns.

**Auto-timestamps** — `#[djangors(auto_now_add = true)]` stamps `chrono::Utc::now()` on `save()`;
`#[djangors(auto_now = true)]` stamps it on both `save()` and `update()`. No manual
`created_at`/`updated_at` assignment needed.

```rust
pub created_at: chrono::DateTime<chrono::Utc>,   // plain field
#[djangors(auto_now_add = true)]
pub created_add: chrono::DateTime<chrono::Utc>,  // INSERT only
#[djangors(auto_now = true)]
pub updated_at: chrono::DateTime<chrono::Utc>,   // INSERT + UPDATE
```

**Field `choices`** — `#[djangors(choices = ["draft", "published"])]` on a `String` field. The
macro derives `FieldMeta.choices`, migrations emit a DB `CHECK (col IN (...))` constraint, and the
admin renders it as a `<select>` filter (see Admin).

---

## Saving, updating, deleting

```rust
// save() = INSERT...RETURNING — returns the row with DB-assigned PK
let post = Post { title: "Hello".into(), author: ForeignKey::<User>::new(1), .. };
let post = post.save(&db).await?;

// update() = UPDATE ... WHERE pk = ...
post.title = "edited".into();
post.update(&db).await?;  // returns Result<(), OrmError> — NotFound if no row matched

// delete() = DELETE ... WHERE pk = ...
post.delete(&db).await?;
```

**`ForeignKey::new(id)` is how you set FK values** — never visible in docs but required everywhere:

```rust
let enrollment = Enrollment {
    student: ForeignKey::<StudentProfile>::new(student_id),
    school: ForeignKey::<School>::new(school_id),
    ..
};
enrollment.save(&db).await?;
```

---

## Querying

```rust
use djangors_orm::{q, Model};

// Filter
Post::objects().filter(q!(published = true))?.order_by("-published_at")?.all(&db).await?;

// Lookup suffixes on q! fields: __gt, __gte, __lt, __lte, __ne, __contains, __icontains,
//   __startswith, __endswith, __iexact, __in (Vec), __isnull (bool), __regex, __iregex
Post::objects().filter(q!(view_count__gt = 100, published_at__gte = since))?.all(&db).await?;

// OR / NOT / AND
Choice::objects()
    .filter(q!(votes = 0i32) | q!(votes__gt = 100i32))?  // OR
    .filter(!q!(choice_text = "spam"))?                    // NOT
    .exclude(q!(votes = 0i32))?;                           // same as filter(!q!(...))

// select_related — 2 queries, batched forward FK
let posts_with_authors = Post::objects().select_related::<User, _>(&db, "author").await?;

// Get single
Post::objects().filter(q!(id = 1))?.get(&db).await?;
Post::objects().filter(q!(id = 1))?.first(&db).await?;  // Option<T>

// Count, exists, aggregate
let n = Post::objects().count(&db).await?;
let has = Post::objects().filter(q!(published = true))?.exists(&db).await?;

// values / values_list — column projection, avoid fetching unused columns
let titles = Post::objects().values_list(&db, "title").await?;

// Bulk update with F expressions (atomic SQL increment)
Choice::objects().filter(q!(id = 1))?.update(&db, set!(votes = F("votes") + 1)).await?;

// get_or_create / update_or_create — idempotent upsert (T must be Send)
let (post, created) = Post::objects()
    .filter(q!(slug = "hello"))?
    .get_or_create(&db, || vec![("title", "Hello".into())]).await?;
let (post, created) = Post::objects()
    .filter(q!(slug = "hello"))?
    .update_or_create(&db,
        || vec![("title", "Hello".into())],
        || set!(view_count = 0)).await?;
// Neither is wrapped in a transaction — pair with a UNIQUE constraint if the race matters.

// Full-text search (Postgres tsvector) — `search` chains into the query
Post::objects().search("rust djangors", &["title", "body"])?.all(&db).await?;

// DB function expressions (COALESCE / LOWER / UPPER / CONCAT / LENGTH) via annotate_funcs
use djangors_orm::aggregate::FuncExpr;
let rows = Post::objects()
    .annotate_funcs(&db, &["author"], vec![
        ("title_lower", FuncExpr::lower("title")),
        ("title_len", FuncExpr::length("title")),
    ])
    .await?;

// EXPLAIN (Postgres-only) — returns the plan as a String
let plan = Post::objects().filter(q!(published = true))?.explain(&db).await?;
```

---

## Migrations

The framework ships TWO migration strategies:

### Auto-generation from models (recommended)
```rust
// In main.rs, ONE line:
djangors_migrations::migrate(&db).await?;
```
This calls `build_create_all_plan` which reads all `#[derive(Model)]` structs, topologically sorts
them by FK dependency, generates `CREATE TABLE IF NOT EXISTS` DDL, and records `0001_initial` in
the `djangors_migrations` tracking table. On subsequent starts, it's a no-op.

`CREATE TABLE` DDL also includes `CHECK` constraints for any field declared with
`#[djangors(choices = [...])]` (see Defining a model).

For tables NOT backed by `#[derive(Model)]` (e.g. framework bootstrap tables), call their own
setup functions: `djangors_tasks::create_task_table(&db)`, etc.

### Hand-written SQL (for custom DDL)
Put numbered `.sql` files in a flat `migrations/` directory:
```
migrations/0001_initial.sql
migrations/0002_add_index.sql
```
Each file uses `-- up` and `-- down` markers. Then call:
```rust
djangors_migrations::migrate_from_dir(&db, Path::new("migrations")).await?;
```
Or use the CLI: `dj migrate` / `dj migrate --rollback 1`.

---

## REST API — ViewSets (full API)

### Basic ViewSet
```rust
use djangors_rest::{ViewSetConfig, viewset_routes_with_config};

let config = ViewSetConfig {
    filterable_fields: &["status", "author_id"],
    orderable_fields: &["published_at", "title"],
    page_size: Some(25),
    max_page_size: Some(100),         // allows ?page_size= up to this cap
    cursor_pagination: false,          // opt-in cursor-based pagination
    ..ViewSetConfig::default()
};

// Default permission is IsAuthenticated
viewset_routes_with_config::<Post>(router, "/api/posts", config)
```

### All ViewSet mounting functions:
| Function | What it does |
|---|---|
| `viewset_routes::<M>(router, path)` | Default config + IsAuthenticated |
| `viewset_routes_with_config::<M>(router, path, config)` | Custom filtering/ordering |
| `viewset_routes_with_permission::<M, P>(router, path, perm)` | Custom permission (e.g. `AllowAny`) |
| `viewset_routes_with_config_and_permission::<M, P>(router, path, config, perm)` | Both |
| `viewset_routes_with_options::<M, P>(router, path, options, perm)` | Full `ViewSetOptions` |
| `scoped_viewset_routes::<M>(router, path)` | Requires `M: Scoped`, default config only |
| `scoped_viewset_routes_with_config::<M>(router, path, config)` | Requires `M: Scoped` + custom config |

### Scoped ViewSet (multi-tenant / row ownership)
```rust
use djangors_rest::Scoped;
use djangors_core::{Request, error::DjangorsError};

impl Scoped for Post {
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
        let owner_id = req.ext::<CurrentOwner>().ok_or(DjangorsError::Unauthorized("...".into()))?;
        // qs.filter(...) returns Result<_, OrmError> — convert to DjangorsError:
        qs.filter(q!(author = owner_id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))
    }
}

// Scoped routes with custom config — use the ready-made function:
djangors_rest::scoped_viewset_routes_with_config::<Post>(router, "/api/posts", ViewSetConfig {
    filterable_fields: &["author"],
    orderable_fields: &["published_at"],
    page_size: Some(25),
    ..ViewSetConfig::default()
});
// Like scoped_viewset_routes, this wraps each handler in IsAuthenticated and requires
// `M: Scoped` (compile-time), then applies `config` on top.
```

### Filter backends (FieldFilter, SearchFilter, OrderingFilter)
```rust
use djangors_rest::{
    FieldFilter, OrderingFilter, SearchFilter,
    ViewSetConfig, ViewSetOptions,
};

let options = ViewSetOptions::<Post>::new(ViewSetConfig::default())
    .with_filter_backend(FieldFilter::new(&["status", "author_id"]))
    .with_filter_backend(SearchFilter::new(&["title", "body"]))
    .with_filter_backend(OrderingFilter::new(&["published_at", "title"]));
```
Every backend is allowlist-driven — unlisted fields are silently ignored.

### Serializers
```rust
use djangors_rest::{FieldSet, ModelSerializer, NestedSerializer, Serializer};

// Read/write field separation
let serializer = ModelSerializer::<Post>::new(
    FieldSet::all()
        .excluding(&["internal_notes"])
        .read_only(&["id", "published_at"])
        .write_only(&["password"]),
);

// Nested serializer (join with select_related first)
let nested = NestedSerializer::new(
    ModelSerializer::<Comment>::default(),
    "author",
    ModelSerializer::<User>::default(),
);
```

### Throttling on ViewSets
```rust
use djangors_rest::Throttle;
let throttle = Throttle::new("posts_endpoint", "100/hour", store);
let options = ViewSetOptions::<Post>::default().with_throttle(throttle);
// Throttle::new(scope, rate_string, cache_store) — NOT (rate, scope)
```

### Permissions
```rust
use djangors_rest::{AllowAny, IsAuthenticated, IsStaff, IsSuperuser, IsReadOnly, PermissionExt};

// Staff may write, everyone else read-only:
let perm = IsStaff.or(IsReadOnly);

// Public endpoint:
viewset_routes_with_permission::<Post, AllowAny>(router, "/api/public-posts", AllowAny)
```

### `current_user()` — resolves authenticated user from session/JWT/token
```rust
use djangors_rest::permissions::current_user;

pub async fn my_handler(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
    let user = current_user(&req).await.ok_or(DjangorsError::Unauthorized("...".into()))?;
    // user.id(), user.is_staff, user.is_superuser(), etc.
}
```
This is the single most useful auth utility. It checks session auth, then API token auth, then JWT
(in that order).

### `serialize()` convenience function
`djangors_rest::serialize(&model)` returns a `serde_json::Value` for one model.
There is **no** `serialize_many` free function (that name does not exist in
0.6.1) — map `serialize` over a collection, or use
`Serializer::to_representation_many`:
```rust
use djangors_rest::{serialize, Serializer};
Response::json(StatusCode::OK, &djangors_rest::serialize(&model))
Response::json(StatusCode::OK, &models.iter().map(serialize).collect::<Vec<_>>())
```

### Error response envelope
```json
{ "error": { "code": "validation_error", "message": "...", "details": {...} } }
```
Build with `DjangorsError::api(status, code, message)` and `.with_details(json)`.

---

## Multi-tenancy (djangors-contrib-tenancy)

```rust
// main.rs — add middleware (requires SessionLayer to run first):
use djangors_contrib_tenancy::TenantResolutionLayer;
use djangors_auth::SESSION_USER_ID_KEY;

let service = ServiceBuilder::new()
    .layer(SessionLayer::new(store))
    .layer(TenantResolutionLayer::new(db.clone(), |req: &hyper::Request<_>| {
        req.extensions().get::<Session>()?.get::<i64>(SESSION_USER_ID_KEY)
    }))
    .service(router);

// Model scoping — one line per model:
use djangors_contrib_tenancy::tenant_scope;
impl Scoped for SchoolClass {
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
        tenant_scope(req, qs, "school_id")  // FK column on this model
    }
}
```

`TenantResolutionLayer` reads `X-Tenant-Id` header, verifies the authenticated user has a real
`TenantMembership` row for that tenant — never trusts the header alone.

---

## Admin site

```rust
use djangors_admin::{AdminSite, ModelAdminConfig};

let site = AdminSite::new()
    .with_site_header("My App")
    .with_site_title("Admin");
site.register::<Post>();  // all defaults
site.register_with::<Post>(ModelAdminConfig {
    list_display: Some(&["title", "author", "published"]),
    list_filter: Some(&["published"]),     // Boolean or choices-declared fields only
    search_fields: Some(&["title"]),        // text-like fields only
    ..Default::default()
});
router.mount("/admin", site.urls());

// Inlines — render a child model's rows inside the parent's change form:
site.register_with::<Post>(ModelAdminConfig {
    inlines: Some(&[InlineConfig {
        struct_name: "Comment",            // child's struct name (from all_registered_models)
        relation_field: "post",            // FK field on the child pointing at the parent
        fields: &["body", "author"],
    }]),
    ..Default::default()
});
```

### CRITICAL PITFALLS — runtime panics:
- **`list_filter` accepts Boolean fields and fields declared with `#[djangors(choices = [...])]`**
  only. Passing a plain String/FK/int field panics at startup. Set to `None` or only use `bool` /
  `choices` fields.
- **`search_fields` only accepts Char/Text/Email/Url/Slug fields.** Passing a ForeignKey field
  panics at startup. Set to `None` or only use text-like fields.

### Tenant-scoped admin
`AdminSite::with_tenant_scoping(tenant_field, extract_tenant_id)` declares the FK column every
model uses to point at a tenant plus a function that returns the "current" tenant id from a
request (typically reads `CurrentTenant` from request extensions). Once set, every registered
model's changelist/add/change/delete queries are filtered by that tenant, so a tenant-backed model
can't leak rows across tenants in the admin either. Inlines inherit the parent's tenant scoping.

```rust
use djangors_contrib_tenancy::CurrentTenant;
let site = AdminSite::new().with_tenant_scoping(
    "school_id",
    |req| req.extensions().get::<CurrentTenant>().map(|t| t.id),
);
```

---

## Payments (djangors-contrib-payments)

```rust
use djangors_contrib_payments::{
    PaystackProvider, PaymentProvider,
    handle_paystack_webhook, record_charge_initiated,
};

let provider = PaystackProvider::new(secret_key);

// Initiate
let resp = provider.initiate(&InitiateChargeRequest {
    email: "user@example.com".into(),
    amount_minor: 50_000,  // NGN 500.00 — always integer minor units
    currency: "NGN".into(),
    reference: "order-123".into(),
    callback_url: None,
}).await?;
// resp.authorization_url

// Webhook — verifies HMAC-SHA512, checks event==charge.success AND data.status==success,
// idempotently records the transaction (database-level UNIQUE constraint on reference):
handle_paystack_webhook(&provider, db, body_bytes, signature).await?;
```

---

## Background tasks

```rust
#[task]
async fn send_welcome_email(payload: WelcomePayload) -> Result<(), TaskError> { /* ... */ }

djangors_tasks::enqueue(&db, "send_welcome_email", &payload).await?;
djangors_tasks::register_recurring(&db, "cleanup", &(), "0 2 * * *").await?;
```

---

## Typed extractors: Json<T>, Form<T>, Query<T>

```rust
use djangors_core::extract::{FromRequest, Json, Form, Query};

// WARNING: Djangors handlers are ALWAYS `Fn(Request, PathParams)`.
// There is NO axum-style argument-position extraction like
// `pub async fn h(Json(body): Json<T>, req: Request, ...)` — the Handler trait
// matches `Fn(Request, PathParams)`, so extractors are called in the body:
pub async fn create(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
    let Json(body) = Json::<CreatePayload>::from_request(&req).await?;
    // ...
}

// Query params:
pub async fn search(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
    let Query(params) = Query::<SearchParams>::from_request(&req).await?;
    // ...
}

// Path params:
let id: i64 = djangors_core::extract::extract_path_param(&params, "pk")?;
```

---

## Typed settings

```rust
#[derive(djangors_macros::Settings, Debug)]
struct AppSettings {
    api_key: String,                                    // required
    #[djangors(default = "localhost".to_string())]
    host: String,
    #[djangors(default = 8000)]
    port: u16,
}
let settings = AppSettings::load()?;
```

---

## `dj` CLI (Django's `manage.py`)

`new`, `new-app`, `run [--port]`, `check [--deploy]`, `migrate [--rollback N] [--plan]`,
`createsuperuser`, `makemigrations`, `createpermissions`, `dbshell`, `runworker`, `shell` (evcxr REPL),
`test`, `collectstatic`.

---

## Signals

Every `#[derive(Model)]` generates signal emitters:
- `pre_save_signal()` / `post_save_signal()` — emitted during `.save()`
- `pre_delete_signal()` / `post_delete_signal()` — emitted during `.delete()`

Receivers connect via `.connect(handler)`. `djangors_auth` defines `LOGIN_SUCCEEDED`,
`LOGIN_FAILED`, and `LOGGED_OUT` signals.

---

## Module privacy — items re-exported at crate root

These modules are **private** — import from the crate root, not the submodule:

| ❌ Wrong | ✅ Correct |
|---|---|
| `djangors_contrib_tenancy::scope::tenant_scope` | `djangors_contrib_tenancy::tenant_scope` |
| `djangors_contrib_tenancy::middleware::CurrentTenant` | `djangors_contrib_tenancy::CurrentTenant` |
| `djangors_contrib_payments::transaction::handle_paystack_webhook` | `djangors_contrib_payments::handle_paystack_webhook` |

---

## Common pitfalls

1. **`Json::from_request` needs `FromRequest` trait in scope** — `use djangors_core::extract::FromRequest;`
2. **`.objects()` needs `Model` trait in scope** — `use djangors_orm::Model;`
3. **`.to_representation()` needs `Serializer` trait in scope** — `use djangors_rest::Serializer;`
4. **`.has_permission()` needs `Permission` trait in scope** — `use djangors_rest::Permission;`
5. **`ForeignKey::new(id)` is required** — struct literal FK fields need this
 6. **`scoped_viewset_routes` only takes the default config** — use
    `scoped_viewset_routes_with_config` for custom config on scoped routes
 7. **Admin `list_filter` panics on non-Boolean fields** — only `bool` fields or fields with
    `#[djangors(choices = [...])]` allowed
8. **Admin `search_fields` panics on FK fields** — only text-like fields allowed
9. **`build_create_all_plan` needs all registered models** — models must use `#[derive(Model)]`
10. **`save()` is INSERT-only, `update()` is UPDATE-only** — track persisted state yourself
11. **Never mount `ViewSet::<M>`/`ScopedViewSet::<M>`'s associated functions (`list`,
    `list_with_config`, `retrieve`, `create`, `update`, `destroy`) as bare route handlers** —
    e.g. `router.post("/x", ScopedViewSet::<M>::create)`. They perform **no** authentication
    check themselves; `Scoped::scope` only restricts *which rows* are visible (typically "this
    tenant"), not *who's allowed to write*. A hand-rolled mount silently gives every
    authenticated tenant member — any role — full read/write on the model. Always mount
    through `viewset_routes*` / `scoped_viewset_routes*`, which wrap every handler in
    `IsAuthenticated` for you. If you need a custom `ViewSetConfig` on a scoped endpoint, that's
    exactly what `scoped_viewset_routes_with_config` is for — there is no case where hand-mounting
    the raw associated functions is the right call.
12. **`:name`-style path params work but are a compatibility alias, not the recommended
    syntax** — write `{name}` / `{name:i64}` / `{name:slug}` in new code so the router validates
    the segment before your handler runs, instead of leaving that to a `.parse()` inside it.

---

## Testing

### TestClient (in-process, no network socket)
```rust
use djangors_test::TestClient;

TestClient::new(router)
    .get("/hello").send().await
    .assert_status(StatusCode::OK)
    .assert_contains("hello");

// Form + session:
let session = Session::new_empty();
session.set("user_id", 42i64);
client.post_form("/submit", &[("title", "Test")])
    .with_session(session).send().await
    .assert_status(StatusCode::CREATED);
```

### TestDatabase
```rust
use djangors_test::TestDatabase;

let test_db = TestDatabase::connect().await.unwrap();
let db = test_db.database();
// SQLite: fresh in-memory DB per test, no cleanup. 0.69s for 32 admin tests.
// Postgres: 15.8s. Use SQLite for dev loop, Postgres in CI.
```

### Real-socket integration test
```rust
let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
tokio::spawn(async move { Djangors::new(s, Router::new()).serve_service(listener, svc).await });
// Send raw HTTP over TcpStream
```

## Email (djangors-mail)
```rust
use djangors_mail::{Message, MailBackend, SmtpBackend, ConsoleBackend};

// Message is a struct literal — there is NO EmailMessage builder.
let msg = Message {
    to: vec!["u@example.com".into()],
    from: "no@example.com".into(),
    subject: "Welcome".into(),
    body: "Hello!".into(),
    html_body: None,
};
backend.send(&msg).await?;
// SmtpBackend for prod, ConsoleBackend for dev, FileBackend for .eml files,
// InMemoryBackend (sent_messages()) for tests. Requires `use MailBackend;` for .send().
```

## Templates (djangors-template, minijinja-backed)
```rust
use djangors_template::TemplateEngine;
let engine = TemplateEngine::from_embedded(&[
    ("base.html", include_str!("../templates/base.html")),
])?;
let html = engine.render("base.html", &context! { "title" => "Home" })?;
```

## Caching (djangors-cache)
```rust
use djangors_cache::{Cache, CacheExt, InMemoryCache};
let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new(10_000));
// Raw byte get/set:
cache.set("key", b"val".to_vec(), Some(Duration::from_secs(300))).await?;
let raw: Option<Vec<u8>> = cache.get("key").await?;
// JSON auto-serialise/deserialise via CacheExt (blanket impl on all Cache):
let val: Option<MyStruct> = cache.get_or_set_json("key2", Some(Duration::from_secs(60)), || async {
    Ok(MyStruct { data: 42 })  // computed once, cached for 60s
}).await?;
```

## Production middleware stack (correct order)
```rust
ServiceBuilder::new()
    .layer(logging_layer())                          // 1. log every request
    .layer(request_id_layer())                       // 2. X-Request-Id
    .layer(compression_layer())                      // 3. gzip/brotli
    .layer(SessionLayer::new(s))                     // 4. signed sessions
    .layer(TenantResolutionLayer::new(db, user_extractor)) // 5. tenant from session
    .layer(csrf_layer())                             // 6. CSRF
    .layer(hsts_layer(31536000))                     // 7. HSTS
    .layer(security_headers_layer())                 // 8. XFO/nosniff/referrer
    .layer(HostValidationLayer::new(                 // 9. Host header check
        vec!["api.example.com".to_string()])))
    .layer(normalize_path_layer())                   // 10. strip trailing slash
    .service(router_service);
```

`host_validation` is a struct, not a function — `HostValidationLayer::new(Vec<String>)`:
```rust
.layer(HostValidationLayer::new(vec!["api.example.com".to_string()]))

## Custom error renderer (matching existing API envelope)
```rust
struct MyErrorRenderer;
impl djangors_core::error::ErrorRenderer for MyErrorRenderer {
    fn render(&self, err: &DjangorsError, _req: &Request) -> Response {
        Response::json(err.status_code(), &json!({
            "error": {"code": err.code(), "message": err.message()}
        })).unwrap()
    }
}
router.with_state(Arc::new(MyErrorRenderer) as Arc<dyn ErrorRenderer>);
```

## Request lifecycle
```
Tower layers → RouterService → Router → handler(Request, PathParams)
  Request::state<T>()     — app-wide state set via Router::with_state
  Request::ext<T>()       — per-request extensions set by tower middleware
  Request::require_state::<T>() — Result, not Option
  Response::json() / ::html() / ::text() / ::redirect()
  Error → ErrorRenderer::render() → JSON envelope
```

## What's NOT available in 0.6.1
- **No CORS layer** — use `tower_http::cors::CorsLayer` if needed
- **No outbound HTTP client** — use `reqwest` (already vendored via sqlx TLS)
- **No `Option<ForeignKey<T>>`** — use `Option<i64>` for nullable FK columns
- **No `Uuid`/`NaiveDate`/`NaiveTime`/`Duration`/`Decimal`** — use `String`/`i64`, convert at boundary
- **`djangors-test` not published** — path-dependency from workspace or in-process manual testing
- **Admin no tenant-scoping** — admin users can see all tenants' rows
