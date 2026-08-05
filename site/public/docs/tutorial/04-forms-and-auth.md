# Tutorial Part 4: Forms, POST Requests, and Authentication

In Part 4, we handle HTML form submissions, execute atomic database updates using F-expressions, enforce login-gating, and implement session-based authentication (`login_view` and `logout_view`).

> [!NOTE]
> All code snippets in this part are directly sourced from [`examples/polls/src/views.rs`](file:///root/dev/Rango/examples/polls/src/views.rs).

---

## 1. Extracting Form Submissions with `Form<T>`

Djangors provides a `Form<T>` extractor powered by `serde::Deserialize` for extracting URL-encoded form data.

In `src/views.rs`, implement the `vote` view:

```rust,compile
# use polls::models::{Question, Choice};
use djangors_auth::{Auth, AuthBackend};
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response};
use djangors_orm::{q, Model};

pub async fn vote(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    // 1. Enforce authentication (returns 401 Unauthorized if user is not logged in)
    let _auth = Auth::<djangors_auth::User>::from_request(&req).await?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

    let question_id: i64 = params.get_as("question_id")?;

    #[derive(serde::Deserialize)]
    struct VoteForm {
        choice: i64,
    }
    let Form(vote) = Form::<VoteForm>::from_request(&req).await?;

    let question = Question::objects()
        .filter(q!(id = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|e| match e {
            djangors_orm::OrmError::NotFound { .. } => DjangorsError::NotFound,
            _ => DjangorsError::Internal(e.to_string()),
        })?;

    // 2. Perform atomic increment on choice votes using F() expressions
    Choice::objects()
        .filter(q!(question = question.id, id = vote.choice))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .update(
            db,
            djangors_orm::set!(votes = djangors_orm::F("votes") + 1i64),
        )
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    // 3. Redirect to the results page
    Ok(Response::redirect(&format!("/{}/results/", question_id)))
}
```

---

## 2. Implementing Login and Logout Views

Authentication in Djangors leverages `ModelBackend` for credential verification and `djangors_sessions::Session` for session persistence.

Add `login_view` and `logout_view` to `src/views.rs`:

```rust,compile
# use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
# use djangors_core::extract::{Form, FromRequest};
# use djangors_auth::AuthBackend;
#[derive(serde::Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

pub async fn login_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

    let session = req.ext::<djangors_sessions::Session>().ok_or_else(|| {
        DjangorsError::Internal("session extension absent (SessionLayer misconfigured)".to_string())
    })?;

    let Form(form) = Form::<LoginForm>::from_request(&req).await?;

    let backend = djangors_auth::ModelBackend;
    match backend
        .authenticate(db, &form.username, &form.password)
        .await
    {
        Ok(Some(user)) => {
            djangors_auth::login(session, &user);
            Ok(Response::redirect("/"))
        }
        Ok(None) => Ok(Response::redirect("/accounts/login/?error=1")),
        Err(e) => Err(DjangorsError::Internal(e.to_string())),
    }
}

pub async fn logout_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let session = req.ext::<djangors_sessions::Session>().ok_or_else(|| {
        DjangorsError::Internal("session extension absent (SessionLayer misconfigured)".to_string())
    })?;

    djangors_auth::logout(session).await;
    Ok(Response::redirect("/"))
}
```

---

## What's Real vs. What Django Has That Djangors Doesn't Yet

> [!IMPORTANT]
> **Key Architecture Differences from Django:**
> - **Strongly-Typed Form Extraction**: Request body parsing uses `Form<T>::from_request(&req).await?` where `T` is a Rust struct deriving `serde::Deserialize`.
> - **Explicit Auth Gating**: Instead of Python `@login_required` decorators, handlers extract `Auth::<User>` directly via `Auth::<djangors_auth::User>::from_request(&req).await?`.
> - **Atomic F-Expressions**: Database field updates use `set!(field = F("field") + delta)` macros for race-condition-free increments.
> - **Redirect Response**: Responses issue HTTP redirects using `Response::redirect(url)`.

---

## Running and Verifying

1. Start the dev server:

```bash
DATABASE_URL="postgres://postgres:postgres@localhost/djangors_dev" dj run --port 8000
```

2. Submit a login POST request:

```bash
curl -X POST http://localhost:8000/accounts/login/ \
  -d "username=testuser&password=correct_password" -i
```

3. Post a vote for question 1 (authenticated with CSRF and session cookies):

```bash
curl -X POST http://localhost:8000/1/vote/ \
  -H "Cookie: csrftoken=YOUR_CSRF_TOKEN; djangors_sessionid=YOUR_SESSION_ID" \
  -H "X-CSRFToken: YOUR_CSRF_TOKEN" \
  -d "choice=1" -i
```
