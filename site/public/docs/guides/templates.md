# Templates

`djangors-template` provides Django-style HTML template rendering powered by MiniJinja.

## `TemplateEngine`

The core rendering engine is `TemplateEngine`.

### Filesystem Loading (`TemplateEngine::new`)
Applications load templates from disk by providing search directories:

```rust,compile
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use djangors_template::TemplateEngine;
use std::path::PathBuf;

let engine = TemplateEngine::new(vec![
    PathBuf::from("templates"),
    PathBuf::from("apps/polls/templates"),
])?;
# Ok(())
# }
```

Search directories are checked in the specified order. The first directory containing the requested template name wins, supporting Django's template override pattern (project-level templates override app-level templates).

### Embedded Loading (`TemplateEngine::from_embedded`)
Library crates (such as `djangors-admin`) compile templates directly into the binary using `include_str!`:

```rust,compile
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let engine = djangors_template::TemplateEngine::from_embedded(&[
    ("admin/index.html", "<h1>Index</h1>"),
    ("admin/base.html", "<h1>Base</h1>"),
])?;
# Ok(())
# }
```

---

## Rendering Templates

### Direct Rendering
Render a template into a `String` with any serializable context (`Serialize`):

```rust,compile
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let engine = djangors_template::TemplateEngine::from_embedded(&[("index.html", "Hello {{ name }}")]).unwrap();
#[derive(serde::Serialize)]
struct Context {
    name: String,
}

let html: String = engine.render("index.html", &Context { name: "Alice".into() })?;
# Ok(())
# }
```

### HTTP Response Helper (`render`)
Constructs an HTTP `Response` object (`200 OK`, `text/html` content type):

```rust,compile
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let engine = djangors_template::TemplateEngine::from_embedded(&[("polls/index.html", "Hello")]).unwrap();
# let context = ();
use djangors_template::render;
use djangors_core::Response;

let response: Response = render(&engine, "polls/index.html", &context)?;
# Ok(())
# }
```

---

## Auto-escaping Behavior

Auto-escaping is context-aware based on template filenames:
- Files ending in **`.html`** or **`.htm`** automatically enable HTML escaping (`AutoEscape::Html`). HTML characters like `<`, `>`, `&`, `"`, and `'` are escaped.
- Other extensions (e.g. **`.txt`**) default to no escaping (`AutoEscape::None`).

```jinja
<!-- Input: "<script>alert(1)</script>" -->
<!-- In unsafe.html -->
{{ value }} <!-- Output: &lt;script&gt;alert(1)&lt;&#x2f;script&gt; -->

<!-- In unsafe.txt -->
{{ value }} <!-- Output: <script>alert(1)</script> -->
```

---

## Registered Template Filters

`djangors-template` includes built-in Django filters:

| Filter | Example Usage | Description / Output |
|---|---|---|
| `date` | `{{ val\|date('Y-m-d H:i:s') }}` | Formats dates using strftime syntax |
| `floatformat` | `{{ val\|floatformat(2) }}` | Formats floating point numbers (e.g. `34.232` -> `34.23`). Negative argument (e.g. `-2`) omits decimal zeroes if whole. |
| `pluralize` | `{{ count\|pluralize }}` or `{{ count\|pluralize('y,ies') }}` | Returns `'s'` (or custom suffix) if `count != 1` |
| `truncatewords` | `{{ val\|truncatewords(3) }}` | Truncates text after N words, adding `…` |
| `intcomma` | `{{ 1234567\|intcomma }}` | Formats integers with comma separators (`"1,234,567"`) |
| `filesizeformat` | `{{ 1048576\|filesizeformat }}` | Formats bytes into human-readable size (`"1.0 MB"`, `"500 bytes"`, `"1.5 KB"`) |
| `naturaltime` | `{{ val\|naturaltime }}` | Formats ISO-8601 datetimes as relative strings (`"just now"`, `"5 minutes ago"`, `"in 5 minutes"`) |
| `trans` | `{{ "Welcome"\|trans }}` | Internationalization translation filter (via `djangors-i18n`) |
| `default` | `{{ val\|default('N/A', true) }}` | Built-in fallback value if `val` is undefined or falsy |

---

## Template Functions

In `djangors-template`, template helpers are invoked as **functions** using `{{ ... }}` variable expression syntax rather than statement tags `{% ... %}`:

| Function | Example Usage | Description / Output |
|---|---|---|
| `url` | `{{ url('poll-detail', id=1) }}` | Resolves a named route to an absolute URL path. Requires `set_url_resolver` on `TemplateEngine`. |
| `static` | `{{ static('css/app.css') }}` | Joins configured static URL prefix (default `/static/`) and path with exactly one slash. |
| `csrf_token` | `{{ csrf_token() }}` | Renders safe `<input type="hidden" name="csrfmiddlewaretoken" value="...">` with HTML-attribute-escaped token value. |
| `now` | `{{ now('%Y-%m-%d') }}` | Formats current UTC time with a `chrono` strftime format string. |

> [!NOTE]
> Django uses statement tags such as `{% static "..." %}` and `{% csrf_token %}`. Because MiniJinja 2.21 uses function expressions, Djangors uses function call syntax (`{{ static('...') }}` and `{{ csrf_token() }}`).

### Wiring Named Routes with `set_url_resolver`

To support `{{ url(...) }}`, wire your router to the `TemplateEngine` using `set_url_resolver`:

```rust,compile
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use djangors_core::Router;
use djangors_template::TemplateEngine;

let mut engine = TemplateEngine::new(vec!["templates".into()])?;

let router = Router::new()
    .get("/polls/{id}", |_, _| async { Ok(djangors_core::Response::text(djangors_core::StatusCode::OK, "ok")) })
    .name("poll-detail");

let router_clone = router.clone();
engine.set_url_resolver(move |name, params| {
    let params_refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    router_clone.reverse(name, &params_refs).ok()
});
# Ok(())
# }
```

