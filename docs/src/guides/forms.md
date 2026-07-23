# Forms & Form Processing

`djangors-core` provides typed request body extraction, including URL-encoded form parsing.

## The `Form<T>` Extractor

The `Form<T>` struct (`djangors_core::extract::Form`) extracts and deserializes form data submitted via HTTP `POST` requests (`application/x-www-form-urlencoded`).

```rust
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

## How Form Extraction Works

1. `Form<T>::from_request(&req).await` reads the full request body bytes via `req.body_bytes().await`.
2. Body bytes are deserialized into type `T` (which must implement `serde::de::DeserializeOwned`) using `serde_urlencoded::from_bytes`.
3. **Error Handling**:
   - If the request body contains missing required fields or invalid data types for `T`, extraction returns `DjangorsError::BadRequest(msg)` containing `"failed to parse form body: ..."` with HTTP status `400 Bad Request`.

---

## Combining Extractors in Handlers

`djangors-core` extractors implement `FromRequest`:

```rust
use djangors_core::extract::{Form, FromRequest, Json, Query};
```

| Extractor | Source | Content Type / Format |
|---|---|---|
| `Form<T>` | Request Body | `application/x-www-form-urlencoded` |
| `Json<T>` | Request Body | `application/json` |
| `Query<T>` | URI Query String | `?key=value&...` |

### Path Parameter Extraction
Path parameters are retrieved using `extract_path_param`:

```rust
use djangors_core::extract::extract_path_param;
use djangors_core::PathParams;

pub async fn detail(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let question_id: i64 = extract_path_param(&params, "id")?;
    // ...
}
```
