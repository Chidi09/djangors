---
name: djangors-django-migration
description: Use when migrating a Django codebase to Djangors, understanding the file/folder conventions, or translating Django patterns into their Djangors equivalents.
---

# Migrating from Django to Djangors

A complete, codebase-verified migration guide. Every pattern here was used in a real 25-app school
management SaaS migration (~12k lines of Rust, 50+ database tables).

## Philosophy

Djangors is Django reimagined in Rust — not a thin HTTP toolkit. The ORM, admin, forms, auth,
migrations, REST framework, background tasks, payments, multi-tenancy, PDF generation, and the
`dj` CLI are all one framework, tested together. The migration strategy is:

1. **One app at a time**, topologically sorted by FK dependency
2. **Keep the same file names** — `models.rs`, `views.rs`, `urls.rs`, etc.
3. **Keep the same API wire contract** — the frontend should not know the backend changed
4. **Use framework-native patterns** — don't hand-roll what Djangors already provides

---

## File convention mapping

### Standard Django app → Djangors app

| Django file | Djangors file | Notes |
|---|---|---|
| `models.py` | `models.rs` | `#[derive(Model)]` struct, one per table |
| `views.py` | `views.rs` | Free functions or ViewSet references |
| `urls.py` | `urls.rs` | `Router::new().mount(...)` chains |
| `admin.py` | `admin.rs` | `site.register_with::<M>(ModelAdminConfig{...})` |
| `serializers.py` | `serializers.rs` | Custom `Serializer<M>` impls |
| `forms.py` | *(not needed)* | `#[derive(Model)]` auto-generates `ModelForm` |
| `filters.py` | *(not needed)* | Built into `ViewSetOptions::with_filter_backend()` |
| `permissions.py` | `permissions.rs` | `djangors_rest::Permission` impls |
| `services.py` | `services.rs` | Pure business logic (no HTTP, no DB) |
| `errors.py` | `errors.rs` | Domain error enum → `From<BillingError> for DjangorsError` |
| `tasks.py` | `tasks.rs` | `#[task]` attribute macros |
| `signals.py` | *(not needed)* | Built into `#[derive(Model)]` — `pre_save_signal()`, etc. |
| `selectors.py` | *(not needed)* | `QuerySet` chains inline — the ORM query builder IS the selector |
| `factories.py` | *(not needed)* | Struct literals are type-checked by the compiler |
| `tests.py` | *(tests/ dir)* | `TestClient` (in-process) or real-socket tests |
| `managers.py` | *(not needed)* | `Model::objects()` is the default manager |
| `apps.py` | `mod.rs` | Module declarations, app-level re-exports |

### Why some Django files don't exist

- **`selectors.py`**: Django uses selectors for CQRS-lite read queries because ORM chaining across
  apps is painful. In Djangors, `Model::objects().filter(q!(...)).all(&db).await` is typed, composable,
  and works inline — no separate read-model layer.

- **`factories.py`**: Python's `factory_boy` exists because dicts aren't type-checked. Rust struct
  constructors ARE factories: `Student { name: "test".into(), age: 12, .. }.save(db)` — the compiler
  verifies every field.

- **`filters.py`**: Django needs `django-filter` as a third-party add-on. Djangors ships
  `FieldFilter`, `SearchFilter`, and `OrderingFilter` in `djangors-rest`. Apply them via
  `ViewSetOptions::with_filter_backend()`.

- **`signals.py`**: Every `#[derive(Model)]` generates `pre_save_signal()`, `post_save_signal()`,
  `pre_delete_signal()`, `post_delete_signal()`. `djangors_auth` defines `LOGIN_SUCCEEDED`,
  `LOGIN_FAILED`, `LOGGED_OUT`.

---

## Models: from Django ORM to Djangors ORM

### Django
```python
class Student(models.Model):
    admission_number = models.CharField(max_length=50, unique=True)
    first_name = models.CharField(max_length=150)
    school = models.ForeignKey(School, on_delete=models.CASCADE)
    date_of_birth = models.DateField(null=True)
    is_active = models.BooleanField(default=True)
    created_at = models.DateTimeField(auto_now_add=True)

    class Meta:
        db_table = "people_student"
        ordering = ["admission_number"]
```

### Djangors
```rust
#[derive(Model, Debug, Clone)]
#[djangors(app = "people", table_name = "people_student", ordering = ["admission_number"])]
pub struct StudentProfile {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 50, unique, db_index)]
    pub admission_number: String,

    #[djangors(max_length = 150)]
    pub first_name: String,

    #[djangors(foreign_key(on_delete = "cascade"))]
    pub school: ForeignKey<School>,

    pub date_of_birth: Option<chrono::NaiveDate>,

    #[djangors(default = true)]
    pub is_active: bool,

    #[djangors(auto_now_add = true)]
    pub created_at: chrono::DateTime<chrono::Utc>,

    #[djangors(auto_now = true)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

### Key differences

| Django | Djangors |
|---|---|
| `models.CharField(max_length=N)` | `#[djangors(max_length = N)] pub field: String` |
| `models.TextField()` | `pub field: String` (no `max_length`) |
| `models.IntegerField()` | `pub field: i32` |
| `models.BigIntegerField()` | `pub field: i64` |
| `models.BooleanField()` | `pub field: bool` |
| `models.DateTimeField()` | `pub field: chrono::DateTime<chrono::Utc>` |
| `models.ForeignKey(..., on_delete=CASCADE)` | `pub field: ForeignKey<Target>` (on_delete in `#[djangors]`) |
| `models.UUIDField()` | `pub field: uuid::Uuid` |
| `models.DateField(null=True)` | `pub field: Option<chrono::NaiveDate>` |
| `models.DecimalField()` | `pub field: rust_decimal::Decimal` (`#[djangors(max_digits = 10, decimal_places = 2)]`) |
| `class Meta: db_table = "x"` | `#[djangors(table_name = "x")]` |
| `class Meta: ordering = ["field"]` | `#[djangors(ordering = ["field"])]` |
| `class Meta: unique_together = [...]` | `#[djangors(unique_together = [["a","b"]])]` |
| `obj.save()` | `obj.save(&db).await?` then `obj.update(&db).await?` |
| `Student.objects.filter(...)` | `StudentProfile::objects().filter(q!(...))?` |
| `obj.pk` / `obj.id` | `obj.id` (always `i64` with `#[djangors(primary_key, auto)]`) |

---

## Views: from Django views to Djangors ViewSets

### Pattern A: Hand-written views → ViewSets

The biggest single win in migration. A Django view like:

```python
@api_view(["GET"])
def list_students(request):
    queryset = Student.objects.filter(school=request.school).order_by("admission_number")
    page = paginate(queryset, request)
    serializer = StudentSerializer(page, many=True)
    return Response({"count": queryset.count(), "results": serializer.data})
```

Becomes:

```rust
// In urls.rs — ONE LINE:
viewset_routes_with_config::<StudentProfile>(router, "/students", ViewSetConfig {
    filterable_fields: &["admission_number", "is_active"],
    orderable_fields: &["admission_number", "created_at"],
    page_size: Some(25),
    ..ViewSetConfig::default()
})
```

No serializer, no pagination, no count — the framework handles all of it.

### Pattern B: Hand-written views → ScopedViewSet (multi-tenant)

Django pattern:
```python
class StudentViewSet(ModelViewSet):
    def get_queryset(self):
        return Student.objects.filter(school=self.request.school)
```

Rust pattern:
```rust
impl Scoped for StudentProfile {
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
        tenant_scope(req, qs, "school_id")
    }
}
// Then in urls.rs:
scoped_viewset_routes::<StudentProfile>(router, "/students")
```

### Pattern C: Custom actions kept as free functions

Not every endpoint maps to CRUD. Keep complex business logic as custom handlers.
Note handlers are always `Fn(Request, PathParams)` — there is no axum-style
argument-position extraction, so extract from `req` inside the body:

```rust
pub async fn approve_application(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let db = req.require_state::<Database>()?;
    let pk: i64 = extract_path_param(&params, "pk")?;
    let Json(body) = Json::<ApprovalPayload>::from_request(&req).await?;
    let app = repositories::by_id(db, pk).await?;
    services::approve(db, &app, &body).await?;
    Response::json(StatusCode::OK, &djangors_rest::serialize(&app))
}

// Mount on top of ViewSet routes:
let mut router = viewset_routes_with_config::<Application>(Router::new(), "/apps", config);
router = router.post("/apps/{pk:i64}/approve", approve_application);
```

---

## URL routing: from Django URLconf to Djangors Router

### Django
```python
urlpatterns = [
    path("students/", views.list_students),
    path("students/<uuid:pk>/", views.student_detail),
    path("students/create/", views.create_student),
]

# With ViewSets:
router = DefaultRouter()
router.register(r"students", StudentViewSet)
urlpatterns += router.urls
```

### Djangors
```rust
pub fn urls() -> Router {
    let config = ViewSetConfig {
        filterable_fields: &["admission_number"],
        orderable_fields: &["admission_number"],
        page_size: Some(25),
        ..ViewSetConfig::default()
    };
    Router::new()
        .mount("/students", viewset_routes_with_config::<StudentProfile>(Router::new(), "/", config))
        .post("/students/{pk:i64}/approve", views::approve_student)
}
```

Path params use Rust format strings: `{pk:i64}`; the type annotation is optional (bare `{pk}`
captures an untyped `String`) but recommended, since it makes the router itself reject a
malformed segment instead of your handler's `.parse()`.

**Do not write Django/Express-style `<uuid:pk>` or `:pk` segments in `.rs` files.** Muscle memory
from the Django URLconf on the left routinely produces `.post("/students/:pk/approve", ...)`
instead of `.post("/students/{pk:i64}/approve", ...)` during a mechanical migration pass — a real
25-app migration shipped 79 routes with this exact mistake across ~20 apps (clearance/payment
approval, result-sheet submit/approve/release, invitation acceptance, LMS course provisioning,
device management, timetable publishing, and more) before it was caught. `:pk` is accepted as an
alias for `{pk}` as of Djangors 0.6.3+ specifically so an existing mistake like that degrades to
"works, but untyped" instead of "route is silently unreachable" — but treat that as a safety net
for legacy code, not permission to write `:name` in a fresh migration. Grep your `urls.rs` files
for `"[^"]*:[a-zA-Z_]` after a Django→Djangors pass to catch any that slipped through.

---

## Serializers: from DRF to Djangors REST

### Django
```python
class StudentSerializer(serializers.ModelSerializer):
    class Meta:
        model = Student
        fields = ["id", "admission_number", "first_name", "school"]
        read_only_fields = ["id"]
```

### Djangors
```rust
use djangors_rest::{FieldSet, ModelSerializer, Serializer};

let serializer = ModelSerializer::<StudentProfile>::new(
    FieldSet::all()
        .read_only(&["id", "created_at"])
        .write_only(&["password"]),
);

// Use:
let json_val = serializer.to_representation(&student);
// Parse + validate a body into write column values (used by ViewSet create/update):
let values = serializer.to_internal_value(json_body, /*partial=*/false)?;
```

`to_internal_value(data, partial)` returns `Result<Vec<(&'static str, Value)>, ValidationErrors>`
ready for `insert_raw`/`update`. (There is no `parse_and_validate` method.)

For complex serializers (nested, custom computations), implement the `Serializer<M>` trait directly.
`NestedSerializer` handles the common "embed a related object" pattern when combined with
`select_related`.

---

## Permissions: from DRF permissions to Djangors permissions

### Django
```python
class IsFinanceStaff(permissions.BasePermission):
    def has_permission(self, request, view):
        return request.user.groups.filter(name="finance").exists()
```

### Djangors
```rust
use djangors_rest::Permission;

struct IsFinanceStaff;

#[async_trait::async_trait]
impl Permission for IsFinanceStaff {
    async fn has_permission(&self, req: &Request) -> bool {
        current_user(req).await.is_some_and(|u| {
            // Check role via your own logic
        })
    }
}
```

Built-in permissions: `AllowAny`, `IsAuthenticated`, `IsStaff`, `IsSuperuser`, `IsReadOnly`.
Combine them: `IsStaff.or(IsReadOnly)`, `IsStaff.and(IsAdmin)`.

---

## Admin: from Django admin to Djangors admin

### Django
```python
@admin.register(Student)
class StudentAdmin(admin.ModelAdmin):
    list_display = ["admission_number", "first_name", "school"]
    search_fields = ["admission_number", "first_name"]
    list_filter = ["is_active", "school"]
```

### Djangors
```rust
use djangors_admin::{AdminSite, ModelAdminConfig};

pub fn register(site: &AdminSite) {
    site.register_with::<StudentProfile>(ModelAdminConfig {
        list_display: Some(&["admission_number", "first_name", "school"]),
        list_filter: Some(&["is_active"]) // bool OR choices-declared fields, else panics
        search_fields: Some(&["admission_number", "first_name"]),
        ..Default::default()
    });
}

// In urls.rs:
let admin = AdminSite::new()
    .with_site_header("School Management")
    .with_site_title("Admin");
app::register(&admin);
Router::new().mount("/admin", admin.urls())
```

**IMPORTANT**: `list_filter` accepts **Boolean** fields and fields declared with
`#[djangors(choices = [...])]` only; `search_fields` accepts text-like fields only. Setting
`list_filter`/`search_fields` to a disallowed field type, or naming a non-existent field anywhere
in the config, **panics at startup** in 0.6.1 (the `AdminSite::register_with` validation uses
`assert!`/`panic!`, so it is a runtime crash, not a compile error). A `choices`-declared field also
gets a DB `CHECK` constraint in migrations and renders as a `<select>` in the admin filter.

---

## Background tasks: from Celery to Djangors tasks

### Django (Celery)
```python
@shared_task
def send_welcome_email(user_id):
    user = User.objects.get(id=user_id)
    send_mail(...)
```

### Djangors
```rust
#[task]
async fn send_welcome_email(payload: WelcomePayload) -> Result<(), TaskError> {
    let db = crate::runtime::db();  // process-global DB handle
    let user = User::objects().filter(q!(id = payload.user_id))?.get(db).await?;
    // ... send email ...
}

djangors_tasks::enqueue(&db, "send_welcome_email", &payload).await?;
djangors_tasks::register_recurring(&db, "cleanup", &(), "0 */12 * * *").await?;
```

---

## Payments: from hand-rolled Paystack to djangors-contrib-payments

### Before (hand-rolled, 127 lines)
```rust
// paystack.rs — reqwest::Client, manual HMAC-SHA512 verify, PaystackError, etc.
pub async fn initialize_transaction(...) -> Result<String, PaystackError> { ... }
pub async fn verify_transaction(...) -> Result<Value, PaystackError> { ... }
pub fn verify_webhook_signature(...) -> bool { ... }
```

### After (framework-native, 0 new lines)
```rust
use djangors_contrib_payments::{PaystackProvider, PaymentProvider, handle_paystack_webhook};

let provider = PaystackProvider::new(secret_key);

// Initialize:
let resp = provider.initiate(&InitiateChargeRequest {
    email: "user@example.com".into(),
    amount_minor: 50_000,  // always integer minor units, never float
    currency: "NGN".into(),
    reference: "inv-123".into(),
    callback_url: None,
}).await?;

// Webhook (HMAC-SHA512 verify + event/status check + idempotent recording — all handled):
handle_paystack_webhook(&provider, db, body_bytes, signature).await?;
```

---

## Multi-tenancy: from hand-rolled scoping to djangors-contrib-tenancy

### Before (hand-rolled, ~5 lines per handler)
```rust
let principal = current_principal(&req).await?;
let school_id = scope_school_id(&principal)?;
let qs = StudentProfile::objects().filter(q!(school_id = school_id))?;
```

### After (compile-time guaranteed, 0 lines per handler)
```rust
// ONE line per model:
impl Scoped for StudentProfile {
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
        tenant_scope(req, qs, "school_id")
    }
}
// All ScopedViewSet routes automatically enforce this — impossible to forget.
```

## Migrations: from hand-written SQL to auto-generation

### Before (hand-maintained)
```
migrations/accounts/0001_accounts.sql
migrations/people/0001_people.sql
migrations/billing/0001_billing.sql
src/migrations.rs  ← 175-line ordered registry
```

### After (auto-generation from models)
```rust
djangors_migrations::migrate(&db).await?;
// Reads all #[derive(Model)] structs, topologically sorts by FK dependency,
// generates CREATE TABLE IF NOT EXISTS DDL, records in djangors_migrations table.
// NOTE: This auto-generation is the fallback — if a `migrations/` directory
// exists in the working directory, `migrate()` delegates to `migrate_from_dir`
// instead (reading .sql files). Delete the migrations/ dir (and the
// hand-written registry) to switch to model-based DDL generation.
```

---

## Error handling pattern

### Django
```python
from rest_framework.exceptions import ValidationError, PermissionDenied

raise ValidationError({"field": "Invalid value"})
```

### Djangors
```rust
use djangors_core::DjangorsError;
use hyper::StatusCode;

// Application errors → Djangors errors:
impl From<BillingError> for DjangorsError {
    fn from(e: BillingError) -> Self {
        match e {
            BillingError::InvalidAmount => DjangorsError::api(
                StatusCode::BAD_REQUEST, "invalid_amount", "Amount must be > 0"
            ),
            BillingError::PaymentNotFound => DjangorsError::api(
                StatusCode::NOT_FOUND, "not_found", "Payment not found"
            ),
            BillingError::Persistence(msg) => DjangorsError::Internal(msg),
        }
    }
}

// Or construct directly:
Err(DjangorsError::api(StatusCode::FORBIDDEN, "permission_denied", "Access denied"))
Err(DjangorsError::Unauthorized("not authenticated".into()))
```

---

## App structure: the pattern we used across 25 apps

Every app follows this structure:

```
src/apps/<name>/
├── mod.rs          — module declarations, top-level re-exports
├── models.rs       — #[derive(Model)] structs, impl Scoped blocks
├── views.rs        — custom handlers (CRUD handled by ViewSets)
├── urls.rs         — Router::new().mount(...) chains
├── services.rs     — pure business logic (no HTTP, no DB)
├── repositories.rs — DB queries (may disappear as ORM improves)
├── serializers.rs  — custom Serializer<M> impls (if needed)
├── contracts.rs    — request/response DTOs (serde Deserialize/Serialize)
├── permissions.rs  — custom Permission impls
├── errors.rs       — domain error enum + From<XError> for DjangorsError
├── admin.rs        — site.register_with::<M>(ModelAdminConfig{...})
└── <extras>.rs     — e.g. paystack.rs → DELETE (use djangors-contrib-payments)
```

### What we deleted across 25 apps

| Removed | Replaced by | Lines saved |
|---|---|---|
| `serde_json::from_slice(req.body_bytes())` (106 sites) | `Json(body): Json<T>` extractor | ~300 |
| `parse_query_string()` + manual pagination (25 apps) | `ViewSetConfig` + ViewSet built-ins | ~500 |
| `scope_school_id(principal)` (all apps) | `Scoped` trait + `tenant_scope()` | ~200 |
| `QuerySet::insert_raw(db, vec![...])` (75 sites) | `.save(&db)` instance method | ~300 |
| `AdminResource` const stubs (16 files) | `AdminSite::register_with()` | ~100 |
| `paystack.rs` (127 lines) | `djangors_contrib_payments` | ~127 |
| `src/migrations.rs` (175 lines) | `djangors_migrations::migrate(&db)` | ~175 |
| **Total** | | **~1,702 lines** |

---

## Quick reference: Django → Djangors glossary

| Django concept | Djangors equivalent |
|---|---|
| `./manage.py runserver` | `dj run` or `cargo run` |
| `./manage.py migrate` | `dj migrate` or `migrate(&db)` in `main.rs` |
| `./manage.py makemigrations` | `dj makemigrations` (diff-based) |
| `./manage.py createsuperuser` | `dj createsuperuser` |
| `./manage.py shell` | `dj shell` (evcxr Rust REPL) |
| `./manage.py test` | `dj test` or `cargo test` |
| `./manage.py collectstatic` | `dj collectstatic` |
| `./manage.py dbshell` | `dj dbshell` |
| `./manage.py check --deploy` | `dj check --deploy` |
| `settings.py` | `#[derive(Settings)]` + `djangors.toml` |
| `SECRET_KEY` | `djangors.toml` → `DJANGORS_SECRET_KEY` env |
| `DATABASE_URL` | Same env var, read via `DatabaseConfig::new(url)` |
| `redis://` for cache/sessions | Same URL format |
| DRF `ModelViewSet` | `viewset_routes::<M>()` |
| DRF `ModelSerializer` | `ModelSerializer::<M>::new(FieldSet::...)` |
| DRF `FilterBackend` | `FieldFilter::new()`, `SearchFilter::new()`, `OrderingFilter::new()` |
| DRF `PageNumberPagination` | Default — `?page=2` |
| DRF `LimitOffsetPagination` | `.with_pagination(LimitOffsetPagination::default())` |
| DRF `CursorPagination` | `ViewSetConfig { cursor_pagination: true, .. }` |
| DRF `Throttle` | `Throttle::new(scope, rate, store)` |
| DRF `IsAuthenticated`, `AllowAny` | Same names in `djangors_rest` |
| Django `ModelAdmin` | `ModelAdminConfig { .. }` |
| Django `admin.site.register()` | `site.register::<M>()` or `site.register_with::<M>(config)` |
| `Q(...)` filters | `q!(field = value)` macro |
| `F("field")` expressions | `F("field")` in `set!` macro |
| `select_related("author")` | `.select_related::<User>(&db, "author").await?` |
| `prefetch_related("posts")` | `prefetch_related::<User, Post>(&db, &users, "posts").await?` |
| `@shared_task` / Celery | `#[task]` attribute |
| `Celery beat` | `register_recurring(&db, name, &payload, cron).await?` |
| `request.user` | `current_user(&req).await` |
| `User.is_authenticated` | `current_user(&req).await.is_some()` |
| `LoginRequiredMixin` | `IsAuthenticated` permission |

---

## Django-specific gotchas (things that surprise Django devs)

1. **`request.user` is `current_user(&req).await`** — it's async because auth resolution may need a DB lookup (session → user_id → user). Django caches this on the request object; Djangors resolves it fresh each call. If you call it multiple times, store the result.

2. **`.save()` returns the row, don't discard it** — Django's `obj.save()` mutates `obj.pk` in place. Djangors' `obj.save(&db).await?` returns a fresh `Self` with the DB-assigned PK. Discarding the return means you don't have the ID.

3. **There is no `request.META`** — headers are `req.header("content-type")`, query string is `req.query("key")`, raw query is `req.raw_query()`, path is `req.path()`. No nested dict.

4. **There is no `request.GET`/`request.POST` dict** — use `Query<T>` extractor for typed query params, `Form<T>` for form bodies, or call `req.query("key")` for individual values.

5. **There is no Django middleware** (string path in `MIDDLEWARE`) — Tower's `ServiceBuilder::new().layer(...)` is ordered, explicit, and type-checked at compile time.

6. **There is no Django `settings` singleton** — use `#[derive(Settings)]` for typed config, then pass it via `router.with_state(settings)`. Handlers read it with `req.state::<AppSettings>()`.

7. **Foreign keys are `ForeignKey<T>` not `T`** — Django gives you the related object on `obj.author` (lazy). Djangors gives you `obj.author.id` (eager, i64). To get the related object, query it: `User::objects().filter(q!(id = post.author.id))?.first(&db).await?`

8. **No lazy relationships** — Django defers FK queries. Djangors requires explicit queries. Use `select_related` to batch-load.

9. **Auto-timestamps are declared, not manual** — Django's `auto_now_add=True`/`auto_now=True`
   map to `#[djangors(auto_now_add = true)]` / `#[djangors(auto_now = true)]` on a
   `chrono::DateTime<chrono::Utc>` field. `auto_now_add` stamps the INSERT (`save()`);
   `auto_now` stamps both `save()` and `update()` with `chrono::Utc::now()`. You no longer need
   to set timestamps by hand on every write.

10. **No `models.Manager`** — every `#[derive(Model)]` gets one `QuerySet` via `.objects()`. No custom managers. Compose query functions instead.

11. **No `blank=True` vs `null=True` distinction** — `Option<T>` maps to nullable DB column. Form-level blank handling is in your validation, not the model.

12. **No `HttpResponse` subclasses** — `Response::json()`, `Response::html()`, `Response::text()`, `Response::redirect()` are constructors on a single `Response` type.

13. **No Django template tags/filters** — Djangors uses minijinja which has its own filter syntax. See the templates guide.

14. **Admin inlines exist** — register FK-related models as inline editors on a parent via
    `ModelAdminConfig { inlines: Some(&[djangors_admin::InlineConfig { struct_name: "Choice", relation_field: "question", fields: &["choice_text", "votes"] }]), ..Default::default() }`.
    Inlines render in the parent's change form (one set of rows per related record), not as a
    separate list page, and are covered by the same tenant scoping as the parent model.

15. **Content types ARE available for generic foreign keys** — `djangors-contrib-contenttypes`
    (`ContentType`, `GenericForeignKey`, `sync_content_types`, `generic_key_for`,
    `resolve_content_type`) is published at 0.6.1. Use it when one table must reference "any
    model" (e.g. an activity feed or a generic audit row). Pair it with
    `djangors-contrib-guardian` for object-level permissions if you need per-row grants.

16. **No Django sites framework** — multi-tenancy replaces the single-site model.

17. **Flatpages/redirects/messages/OTP/sitemaps/feeds are workspace-only (not yet published at
    0.6.1)** — `djangors-contrib-flatpages`, `-redirects`, `-messages`, `-otp`, `-sitemaps`, and
    `-syndication` exist in the workspace but are not on crates.io yet. If you need them, add a
    path dependency from the workspace rather than `version = "0.6.1"`.

18. **WARNING: `String` without `max_length` is `TEXT`** — in Django, `CharField` always needs `max_length`. In Djangors, `String` without `#[djangors(max_length = N)]` maps to `TEXT` (unbounded). This is often what you want but check your schema.

---

## Real-world migration patterns from a 25-app SaaS

### Pattern: Two-phase insert (create + lookup)
```python
# Django: save, then obj.id is populated
obj = Student.objects.create(admission_number="STU-001", ...)
return Response({"id": obj.id, ...})

# DON'T do this in Djangors:
let id = repositories::create_student(&db, &body).await?;     // raw insert, returns id
let student = Student::objects().filter(q!(id = id))?.first(&db).await?;  // re-query
return Response::json(201, &serialize(&student));

# DO this — save() returns the full row:
let student = Student { admission_number: "...", .. }.save(&db).await?;
return Response::json(201, &serialize(&student));
```

### Pattern: Scope-aware query
```python
# Django: request object carries school implicitly
students = Student.objects.filter(school=request.school)

# Djangors BEFORE Scoped trait:
let principal = current_principal(&req).await?;
let qs = Student::objects().filter(q!(school_id = principal.school_id.ok_or(...)?))?;

# Djangors AFTER Scoped trait:
impl Scoped for Student { fn scope(..) { tenant_scope(req, qs, "school_id") } }
// All ScopedViewSet::<Student> queries are automatically scoped — no code per handler.
```

### Pattern: N+1 elimination
```python
# Django: select_related / prefetch_related
posts = Post.objects.select_related("author").all()

# Djangors:
let rows: Vec<(Post, Option<User>)> = Post::objects()
    .select_related::<User>(&db, "author").await?;
// Returns Vec<(T, Option<R>)> — the related row is Option (None if FK dangling).
// Batched in 2 queries total (per the 0.6.1 signature + generic args: pass `<User, _>`).

// Reverse FK (prefetch_related):
prefetch_related::<User, Post>(&db, &users, "posts").await?;
// Populates a HashMap<i64, Vec<Post>> in 1 query.
```

### Pattern: Maintaining the API wire contract
```python
# Django response (DRF):
{ "count": 10, "next": "...", "previous": null, "results": [...] }

# Djangors: ViewSet generates an envelope, but NOT the DRF `next`/`previous`
# shape. The 0.6.1 built-in envelopes are:
# PageNumberPagination (default): {"count": N, "page": 1, "total_pages": 2, "results": [...]}
# LimitOffsetPagination:          {"count": N, "limit": 20, "offset": 0, "results": [...]}
# CursorPagination:               {"next_cursor": "...", "previous_cursor": "...", "results": [...]}
# If your frontend depends on DRF's `next`/`previous`, implement the `Pagination`
# trait and supply your own envelope via `ViewSetOptions::with_pagination`.
```

### Pattern: Idempotent webhook handling
```python
# Django: HMAC verify + check status + idempotent insert
@csrf_exempt
def paystack_webhook(request):
    if not verify_signature(request.body, request.headers["x-paystack-signature"]):
        return Response(status=401)
    data = json.loads(request.body)
    if data["event"] != "charge.success" or data["data"]["status"] != "success":
        return Response(status=200)
    Payment.objects.get_or_create(reference=data["data"]["reference"], defaults={...})
    return Response(status=200)

# Djangors: one call
handle_paystack_webhook(&provider, db, body_bytes, signature).await?;
// HMAC-SHA512 verify, event/status double-check, idempotent DB-level UNIQUE constraint on
// reference — all handled. Returns Transaction. Just call it and return 200.
```

For your own idempotent upserts, `QuerySet::get_or_create` / `update_or_create` are now native
(no hand-rolled check-then-insert):

```rust
// get_or_create: returns the row + whether it was created
let (payment, created) = Payment::objects()
    .filter(djangors_orm::q!(reference = "inv-123"))?
    .get_or_create(&db, || vec![("amount_minor", djangors_orm::expr::Value::I64(50_000))])
    .await?;

// update_or_create: applies `updates` to an existing match instead of returning it unchanged
let (payment, created) = Payment::objects()
    .filter(djangors_orm::q!(reference = "inv-123"))?
    .update_or_create(&db,
        || vec![("amount_minor", djangors_orm::expr::Value::I64(50_000))],
        || djangors_orm::set!(status = "success"))
    .await?;
// Both require `T: Send`. Neither is wrapped in a transaction yet — pair them with a DB
// UNIQUE constraint (or a transaction) if the check-then-insert race matters.
```

### Pattern: Periodic background jobs
```python
# Celery beat: schedule in settings.py
CELERY_BEAT_SCHEDULE = {
    "lock-expired-trials": {"task": "tasks.lock_expired_trials", "schedule": crontab(minute=0)},
}

# Djangors: register at startup in main.rs
djangors_tasks::register_recurring(&db, "lock_expired_trials", &(), "0 * * * *").await?;
// Worker started via tokio::spawn, reads process-global DB handle.
```

### Pattern: Readiness probe (healthz + readyz)
```rust
// healthz — always 200 if process is up
router.get("/healthz", |_, _| async { Response::text(200, "ok") })

// readyz — checks PostgreSQL + Redis
router.get("/readyz", |req, _| async move {
    let db = req.require_state::<Database>()?;
    let redis = req.require_state::<redis::Client>()?;
    let db_ok = db.conn().execute("SELECT 1", &[]).await.is_ok();
    let cache_ok = redis.get_connection().await.is_ok();
    Response::json(200, &json!({"database": db_ok, "cache": cache_ok}))
})
```
