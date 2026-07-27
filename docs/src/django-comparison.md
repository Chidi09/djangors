# Djangors for Django Developers

This guide provides a direct side-by-side translation reference for experienced Django developers learning Djangors. While Djangors preserves Django's familiar mental model (models, views, routers, settings, admin, authentication, migrations), Rust's type system and asynchronous runtime introduce distinct structural and safety guarantees.

---

## 1. Models and Schema Definition

In Django, models inherit from `models.Model` and use Python field descriptors. In Djangors, models are plain Rust `struct`s deriving `#[derive(Model)]` with `#[djangors(...)]` attributes.

### Django (`models.py`)

```python
from django.db import models

class Book(models.Model):
    title = models.CharField(max_length=200)
    author = models.ForeignKey('Author', on_delete=models.CASCADE)
    is_published = models.BooleanField(default=True)
    created_at = models.DateTimeField(auto_now_add=True)

    class Meta:
        db_table = "library_book"
        ordering = ["-created_at"]
```

### Djangors

```rust,compile
# use djangors_orm::Model;
# #[derive(djangors_macros::Model, Debug, Clone)]
# #[djangors(app = "library", table_name = "library_author")]
# pub struct Author { #[djangors(primary_key, auto)] pub id: i64, #[djangors(max_length = 200)] pub name: String }
use djangors_macros::Model;
use djangors_orm::ForeignKey;
use chrono::{DateTime, Utc};

#[derive(Model, Debug, Clone)]
#[djangors(app = "library", table_name = "library_book", ordering = "-created_at")]
pub struct Book {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 200)]
    pub title: String,

    #[djangors(foreign_key(on_delete = "cascade", related_name = "books"))]
    pub author: ForeignKey<Author>,

    #[djangors(default = true)]
    pub is_published: bool,

    pub created_at: DateTime<Utc>,
}
```

### Key Differences & Guarantees
- **Explicit Primary Key Required**: In Djangors, primary key fields (such as `id: i64` with `#[djangors(primary_key, auto)]`) must be explicitly declared on every model.
- **Compile-Time Metadata**: `#[derive(Model)]` generates model metadata (`Model::meta()`) at compile time. Field types and relation targets are checked by `rustc`.

---

## 2. URL Routing

Django uses list-based `urlpatterns` with `path()` or `re_path()`. Djangors uses builder-pattern `Router` instances with method-chained route registrations (`.get()`, `.post()`, `.route()`).

### Django (`urls.py`)

```python
from django.urls import path
from . import views

urlpatterns = [
    path('articles/', views.list_articles, name='article-list'),
    path('articles/<int:id>/', views.article_detail, name='article-detail'),
]
```

### Djangors

```rust,compile
# fn main() {
# use djangors_core::{Request, PathParams, Response, DjangorsError, StatusCode};
# async fn list_articles(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::text(StatusCode::OK, "")) }
# async fn article_detail(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::text(StatusCode::OK, "")) }
use djangors_core::Router;

let router = Router::new()
    .get("/articles/", list_articles)
    .get("/articles/{id}", article_detail);
# }
```

### Key Differences & Guarantees
- **Path Parameters**: Djangors routes use `{id}` syntax for path parameters rather than `<int:id>`.
- **Method Scope**: Routes are bound directly to HTTP verbs (`.get()`, `.post()`, `.put()`, `.delete()`).

---

## 3. View Handlers

Django views receive an `HttpRequest` object and return an `HttpResponse`. Djangors view handlers are `async fn` functions receiving `Request` and `PathParams`, returning `Result<Response, DjangorsError>`.

### Django (`views.py`)

```python
from django.http import HttpResponse, Http404
from django.shortcuts import render
from .models import Article

def article_detail(request, id):
    try:
        article = Article.objects.get(pk=id)
    except Article.DoesNotExist:
        raise Http404("Article not found")
    return render(request, "articles/detail.html", {"article": article})
```

### Djangors

```rust,compile
# #[derive(djangors_macros::Model, Debug, Clone, serde::Serialize)]
# #[djangors(app = "library", table_name = "library_article")]
# pub struct Article { #[djangors(primary_key, auto)] pub id: i64, pub title: String }
use djangors_orm::Model;
use djangors_core::{Request, Response, PathParams, DjangorsError, StatusCode};

// A process-wide template engine, mirroring djangors-admin's own `ADMIN_TEMPLATES` pattern.
static TEMPLATES: std::sync::LazyLock<djangors_template::TemplateEngine> =
    std::sync::LazyLock::new(|| {
        djangors_template::TemplateEngine::new(vec!["templates".into()]).unwrap()
    });

pub async fn article_detail(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let article_id: i64 = params.get_as("id")?;
    let db = req.state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database state missing".into()))?;

    let article = Article::objects()
            .filter(djangors_orm::q!(id = article_id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|_| DjangorsError::NotFound)?;

    djangors_template::render(&TEMPLATES, "articles/detail.html", minijinja::context! { article })
}
```

### Key Differences & Guarantees
- **Async by Default**: All Djangors view handlers are non-blocking `async fn` operations executing on the Tokio runtime.
- **Explicit Database State**: Request state (like `Database` connection pools) is explicitly retrieved via `req.state::<Database>()`.

---

## 4. Command-Line Interface (`manage.py` vs `dj`)

Django uses `python manage.py <command>`. Djangors provides the `dj` command-line utility.

| Django Command | Djangors Command | Description |
| :--- | :--- | :--- |
| `python manage.py runserver` | `dj run` | Starts dev server with live-reloading file watch loop |
| `python manage.py migrate` | `dj migrate` | Applies pending database migrations |
| `python manage.py makemigrations` | `dj makemigrations` | Introspects the project binary; v1 detects new models and new fields |
| `python manage.py createsuperuser` | `dj createsuperuser` | Prompts for superuser credentials and creates User |
| `python manage.py test` | `dj test` | Runs workspace unit and integration test suite (`cargo test`) |
| `python manage.py shell` | `dj shell` | Launches interactive Rust REPL via `evcxr` |
| `python manage.py dbshell` | `dj dbshell` | Connects directly to configured database CLI |

### `dj makemigrations` scope
`dj` runs the project's own binary in a hidden model-introspection mode, so registrations from application crates are visible. It stores the last model state in `migrations/.schema_snapshot.json` and generates numbered SQL migrations. v1 covers new models and new fields; field-type changes, removals, renames, indexes, and relation alterations are deferred.

Database migrations in Djangors are authored as raw SQL files or generated via programmatic schema utilities.

### ℹ️ `dj shell` (evcxr Rust REPL)
`dj shell` launches an interactive Rust REPL via `evcxr` (installed via `cargo install evcxr_repl`).

Because `dj` is a separate binary process from your application, target project models cannot be auto-imported across process boundaries automatically. To import your project's models into the REPL session, use `:dep` with a path dependency:
```rust,illustrative
:dep my_app = { path = "." }
use my_app::models::*;
```

---

## 5. Admin Site Registration

Django registers models with `admin.site.register(Model, ModelAdmin)`. Djangors uses `AdminSite::register_with::<M>(ModelAdminConfig)` for type-safe static configuration.

### Django (`admin.py`)

```python
from django.contrib import admin
from .models import Article

@admin.register(Article)
class ArticleAdmin(admin.ModelAdmin):
    list_display = ["title", "author", "is_published"]
    search_fields = ["title"]
    list_filter = ["is_published"]
```

### Djangors

```rust,compile
# #[derive(djangors_macros::Model, Debug, Clone)]
# #[djangors(app = "library", table_name = "library_article")]
# pub struct Article { #[djangors(primary_key, auto)] pub id: i64, pub title: String, pub author: String, pub is_published: bool }
# fn main() {
use djangors_admin::{AdminSite, ModelAdminConfig};

let site = AdminSite::new().with_site_header("Admin Console");
site.register_with::<Article>(ModelAdminConfig {
    list_display: Some(&["title", "author", "is_published"]),
    search_fields: Some(&["title"]),
    list_filter: Some(&["is_published"]),
    ..Default::default()
});
# }
```

### Key Differences & Guarantees
- **Compile-Time Field Validation**: Djangors validates that fields listed in `list_display`, `search_fields`, and `list_filter` actually exist on the target model struct at registration.
- **Search Type Safety**: `search_fields` enforces text-compatible fields, and `list_filter` enforces boolean fields.

---

## 6. Settings and Configuration

Django uses a Python file (`settings.py`). Djangors uses `DjangorsSettings`, which loads from `djangors.toml` and environment variable overrides (`DJANGORS_*`).

### Django (`settings.py`)

```python
DEBUG = True
ALLOWED_HOSTS = ["localhost", "127.0.0.1"]
SECRET_KEY = "django-insecure-secret"
```

### Djangors (`djangors.toml` & `DjangorsSettings`)

```toml
# djangors.toml
debug = true
allowed_hosts = ["localhost", "127.0.0.1"]
secret_key = "djangors-insecure-secret"
host = "127.0.0.1"
port = 8000
```

Environment variable overrides:
```bash
export DJANGORS_DEBUG=true
export DJANGORS_ALLOWED_HOSTS="localhost,127.0.0.1"
export DJANGORS_SECRET_KEY="supersecret"
export DJANGORS_PORT=8000
```

Loading in Rust:
```rust,compile
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use djangors_core::DjangorsSettings;

let (settings, warnings) = DjangorsSettings::load()?;
# Ok(())
# }
```

---

## 7. CSRF Protection

Django uses `{% csrf_token %}` tag and `CsrfViewMiddleware` to check both request headers and POST form bodies. Djangors provides `csrf_layer()`.

### Header-First with Form-Body Fallback
1. **Cookie**: CSRF token is stored in the `csrftoken` cookie.
2. **Header Check First**: `CsrfLayer` checks for the token in the `X-CSRFToken` HTTP header on unsafe requests (POST, PUT, PATCH, DELETE).
3. **Form Body Fallback**: If the HTTP header is absent, Djangors checks `application/x-www-form-urlencoded` POST request bodies for a `csrfmiddlewaretoken` field.

In HTML forms:
```html
<form method="post">
  <input type="hidden" name="csrfmiddlewaretoken" value="{{ csrf_token }}">
  <button type="submit">Submit</button>
</form>
```

Adding the CSRF layer — like every Djangors middleware, it's composed via `tower::ServiceBuilder`
around a `RouterService`, not a method on `Router` itself:
```rust,compile
# fn main() {
# let router = djangors_core::Router::new();
# let settings = djangors_core::DjangorsSettings::default();
use djangors_core::middleware::csrf_layer;
use djangors_core::router::RouterService;
use tower::ServiceBuilder;

let router_service = RouterService::new(router, settings.debug);
let service = ServiceBuilder::new()
    .layer(csrf_layer())
    .service(router_service);
# }
```

---

## 8. Form Handling & Extraction

Django provides `forms.Form` and `forms.ModelForm` for rendering and parsing HTML forms. Djangors uses the `Form<T>` extractor to deserialize form bodies directly into Rust structs.

### Django (`forms.py` / `views.py`)

```python
class ContactForm(forms.Form):
    name = forms.CharField(max_length=100)
    email = forms.EmailField()

def contact_view(request):
    form = ContactForm(request.POST)
    if form.is_valid():
        name = form.cleaned_data['name']
```

### Djangors

```rust,compile
use serde::Deserialize;
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{Request, Response, PathParams, DjangorsError, StatusCode};

#[derive(Debug, Deserialize)]
pub struct ContactForm {
    pub name: String,
    pub email: String,
}

pub async fn contact_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let Form(form) = Form::<ContactForm>::from_request(&req).await?;
    println!("Submitted name: {}", form.name);
    Ok(Response::text(StatusCode::OK, "OK"))
}
```

### ⚠️ Note on `ModelForm` Parity
Djangors does **not** feature automatic `ModelForm` generation from ORM models. Form payload structures are explicitly defined using Rust `struct`s with Serde deserialization and optional `djangors-forms` validation attributes.

---

## 9. REST Framework APIs

Django relies on Django REST Framework (DRF) `ModelSerializer` and `ViewSet`. Djangors provides `djangors-rest` with `serialize`, `deserialize`, and `ViewSet<M>`.

### Django REST Framework

```python
from rest_framework import serializers, viewsets
from .models import Article

class ArticleSerializer(serializers.ModelSerializer):
    class Meta:
        model = Article
        fields = '__all__'

class ArticleViewSet(viewsets.ModelViewSet):
    queryset = Article.objects.all()
    serializer_class = ArticleSerializer
```

### Djangors (`djangors-rest`)

```rust,compile
# #[derive(djangors_macros::Model, Debug, Clone, serde::Serialize)]
# #[djangors(app = "library", table_name = "library_article")]
# pub struct Article { #[djangors(primary_key, auto)] pub id: i64 }
# fn main() {
# let article_instance = Article { id: 1 };
use djangors_rest::{serialize, viewset_routes};
use djangors_core::Router;

// Low-level model serialization
let json_val = serialize::<Article>(&article_instance);

// High-level ViewSet registration — `ViewSet<M>` has no instance to construct;
// its CRUD handlers are mounted directly as a free function, IsAuthenticated by default.
let router = viewset_routes::<Article>(Router::new(), "/articles");
# }
```

---

## 10. Conceptual Architecture Comparison

| Architectural Aspect | Django (Python) | Djangors (Rust) |
| :--- | :--- | :--- |
| **Execution Model** | Dynamic, interpreted (CPython WSGI/ASGI) | Compiled native binary (Tokio async loop) |
| **Type Checking** | Runtime duck typing, optional type hints | Strict compile-time static type system |
| **Concurrency** | Thread pool (WSGI) or async event loop (ASGI) | Native async task channels and multithreaded Tokio runtime |
| **Deployment Model** | Python interpreter + virtualenv + Gunicorn/Uvicorn + Nginx static serving | Single self-contained binary containing logic, assets, and HTTP server |
| **Development Reloading** | Instant Python module re-import in dev server | File watcher trigger followed by `cargo` incremental binary re-compilation |
| **Memory Management** | Automatic garbage collection (ref counting + GC) | Ownership, borrowing, and RAII without garbage collection overhead |
