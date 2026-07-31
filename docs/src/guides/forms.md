# Forms & Form Processing

`djangors-forms` provides field validation, HTML widgets, layout renderers, and formsets, while `djangors-core` provides typed request body extraction for URL-encoded form POST submissions.

## The `Form<T>` Extractor

The `Form<T>` struct (`djangors_core::extract::Form`) extracts and deserializes form data submitted via HTTP `POST` requests (`application/x-www-form-urlencoded`).

```rust,compile
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, Request, Response, StatusCode};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct VoteForm {
    pub choice: i64,
}

pub async fn vote_handler(req: Request) -> Result<Response, DjangorsError> {
    // Deserialize URL-encoded request body bytes into VoteForm
    let Form(form) = Form::<VoteForm>::from_request(&req).await?;

    println!("Voted for choice ID: {}", form.choice);

    Ok(Response::text(StatusCode::OK, "Vote recorded"))
}
```

---

## Form Widgets & HTML Escaping

`djangors-forms` ships a suite of HTML form widgets implementing the [`Widget`](file:///root/dev/Rango/crates/djangors-forms/src/widgets.rs) trait. All widget output, including field names, values, option labels, and HTML attributes, is escaped against cross-site scripting (XSS).

Available widgets:
- `TextInput`: `<input type="text">`
- `Textarea`: `<textarea>`
- `NumberInput`: `<input type="number">`
- `CheckboxInput`: `<input type="checkbox">` (renders `checked` for boolean truthy values like `"true"`, `"on"`, `"1"`)
- `Select`: `<select>` with `<option>` choices
- `RadioSelect`: Radio inputs with label containers
- `DateInput`: `<input type="date">`
- `EmailInput`: `<input type="email">`
- `PasswordInput`: `<input type="password">` (by default, `render_value = false` to prevent echo of cleartext passwords)
- `HiddenInput`: `<input type="hidden">`

```rust,illustrative
use djangors_forms::{TextInput, PasswordInput, Select, Widget, WidgetAttrs};

let text_widget = TextInput;
let html = text_widget.render("username", Some("alice"), &WidgetAttrs::from([("class", "form-control")]));

let password_widget = PasswordInput::new(); // render_value = false by default
let pass_html = password_widget.render("password", Some("secret"), &WidgetAttrs::new());
```

---

## Form Layout Renderers (`as_div`, `as_table`, `as_p`)

Use [`BoundField`](file:///root/dev/Rango/crates/djangors-forms/src/renderers.rs) to bind fields to widgets, values, labels, help text, and errors, then render using layout helpers:

```rust,illustrative
use djangors_forms::{as_div, as_p, as_table, BoundField, TextInput};

let name_widget = TextInput;
let field = BoundField::new("first_name", &name_widget)
    .with_value(Some("Alice"))
    .with_help_text("Enter your given name")
    .with_errors(vec![]);

let div_html = as_div(&[field], &[]);
```

- `as_div`: Renders fields inside `<div>` containers.
- `as_p`: Renders fields inside `<p>` paragraphs.
- `as_table`: Renders fields as `<tr><th><label></th><td><widget></td></tr>` table rows.

---

## FormSets & Security

[`FormSet`](file:///root/dev/Rango/crates/djangors-forms/src/formsets.rs) manages multiple instances of a form submitted in a single request.

Key features:
1. **Management Form**: Renders `TOTAL_FORMS` and `INITIAL_FORMS` as hidden inputs.
2. **Prefixing**: Prefixing field names per form (`form-0-title`, `form-1-title`, ...).
3. **Deletion**: `can_delete` option renders a `DELETE` checkbox per form.
4. **Security Cap**: `TOTAL_FORMS` is checked against `max_num` (default `1000`) before allocation to prevent memory-exhaustion attacks.

```rust,illustrative
use djangors_forms::{FormSet, FormSetFormResult};
use std::collections::HashMap;

let formset: FormSet<()> = FormSet::new()
    .with_prefix("author")
    .with_counts(2, 1)
    .with_can_delete(true);

let mgmt_form_html = formset.render_management_form();

let mut post_data = HashMap::new();
post_data.insert("author-TOTAL_FORMS".to_string(), "1".to_string());
post_data.insert("author-INITIAL_FORMS".to_string(), "0".to_string());
post_data.insert("author-0-name".to_string(), "Jane Doe".to_string());

let results = formset.clean_with(&post_data, |map| {
    Ok(map.get("name").cloned().unwrap_or_default())
});
```

---

## How Form Extraction Works

1. `Form<T>::from_request(&req).await` reads the full request body bytes via `req.body_bytes().await`.
2. Body bytes are deserialized into type `T` (which must implement `serde::de::DeserializeOwned`) using `serde_urlencoded::from_bytes`.
3. **Error Handling**:
   - If the request body contains missing required fields or invalid data types for `T`, extraction returns `DjangorsError::BadRequest(msg)` containing `"failed to parse form body: ..."` with HTTP status `400 Bad Request`.

---

## Combining Extractors in Handlers

`djangors-core` extractors implement `FromRequest`:

```rust,compile
use djangors_core::extract::{Form, FromRequest, Json, Query};
```

| Extractor | Source | Content Type / Format |
|---|---|---|
| `Form<T>` | Request Body | `application/x-www-form-urlencoded` |
| `Json<T>` | Request Body | `application/json` |
| `Query<T>` | URI Query String | `?key=value&...` |

### Path Parameter Extraction
Path parameters are retrieved using `extract_path_param`:

```rust,compile
# use djangors_core::{Request, PathParams, Response, DjangorsError};
use djangors_core::extract::extract_path_param;

pub async fn detail(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let question_id: i64 = extract_path_param(&params, "id")?;
    let _ = (req, question_id);
    Ok(Response::text(djangors_core::StatusCode::OK, ""))
}
```
