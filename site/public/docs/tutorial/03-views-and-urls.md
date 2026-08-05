# Tutorial Part 3: Views and URLs

In Part 3, we build out the Polls application views (`index`, `detail`, `results`), execute database queries via the Djangors ORM, extract URL path parameters, and handle 404 errors.

> [!NOTE]
> All view code in this part is taken directly from [`examples/polls/src/views.rs`](file:///root/dev/Rango/examples/polls/src/views.rs) and routing from [`examples/polls/src/urls.rs`](file:///root/dev/Rango/examples/polls/src/urls.rs).

---

## 1. Updating `src/urls.rs`

Register parameterized GET routes for `index`, `detail`, and `results` on the [`Router`](file:///root/dev/Rango/crates/djangors-core):

```rust,compile
# mod views {
#     use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
#     pub async fn index(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn detail(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn results(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn vote(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn login_view(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
#     pub async fn logout_view(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::html(StatusCode::OK, "")) }
# }
# mod admin {
#     use djangors_admin::AdminSite;
#     pub fn admin_site() -> AdminSite { AdminSite::new() }
# }
use djangors_core::Router;

pub fn urls() -> Router {
    djangors_admin::favicon_routes(
        Router::new()
            .get("/", views::index)
            .get("/{question_id:i64}/", views::detail)
            .get("/{question_id:i64}/results/", views::results)
            .post("/{question_id:i64}/vote/", views::vote)
            .post("/accounts/login/", views::login_view)
            .post("/accounts/logout/", views::logout_view)
            .mount("/admin", self::admin::admin_site().urls()),
    )
}
```

---

## 2. Querying Database in `index` View

In `src/views.rs`, implement `index` to query the 5 most recent published questions using the ORM `q!()` macro:

```rust,compile
# use polls::models::{Question, Choice};
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
use djangors_orm::{q, Model};

pub async fn index(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

    let latest_question_list = Question::objects()
        .filter(q!(pub_date__lte = chrono::Utc::now()))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .order_by("-pub_date")
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .limit(5)
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let body = latest_question_list
        .iter()
        .map(|q| format!("<li><a href=\"/{}/\">{}</a></li>", q.id, q.question_text))
        .collect::<String>();
    Ok(Response::html(StatusCode::OK, format!("<ul>{body}</ul>")))
}
```

---

## 3. Extracting Path Parameters & Handling 404s in `detail` View

The `detail` view extracts `question_id` from [`PathParams`](file:///root/dev/Rango/crates/djangors-core) using `.get_as::<i64>("question_id")?`. If the question does not exist, it converts `OrmError::NotFound` into `DjangorsError::NotFound`:

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
# use djangors_orm::{q, Model};
pub async fn detail(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

    let question_id: i64 = params.get_as("question_id")?;

    let question = Question::objects()
        .filter(q!(id = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|e| match e {
            djangors_orm::OrmError::NotFound { .. } => DjangorsError::NotFound,
            _ => DjangorsError::Internal(e.to_string()),
        })?;

    let choices = Choice::objects()
        .filter(q!(question = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let choices_html = choices
        .iter()
        .map(|c| {
            format!(
                r#"<li>
                    <input type="radio" name="choice" id="choice_{}" value="{}">
                    <label for="choice_{}">{}</label>
                </li>"#,
                c.id, c.id, c.id, c.choice_text
            )
        })
        .collect::<String>();

    let html = format!(
        r#"<h1>{}</h1>
        <form action="/{}/vote/" method="post">
            <ul>{}</ul>
            <input type="submit" value="Vote">
        </form>"#,
        question.question_text, question.id, choices_html
    );

    Ok(Response::html(StatusCode::OK, html))
}
```

---

## 4. Rendering Results in `results` View

The `results` view fetches question choices and displays current vote tallies:

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
# use djangors_orm::{q, Model};
pub async fn results(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
    let question_id: i64 = params.get_as("question_id")?;

    let question = Question::objects()
        .filter(q!(id = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|e| match e {
            djangors_orm::OrmError::NotFound { .. } => DjangorsError::NotFound,
            _ => DjangorsError::Internal(e.to_string()),
        })?;

    let choices = Choice::objects()
        .filter(q!(question = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let choices_html = choices
        .iter()
        .map(|c| format!("<li>{} -- {} vote(s)</li>", c.choice_text, c.votes))
        .collect::<String>();

    Ok(Response::html(
        StatusCode::OK,
        format!(
            "<h1>Results for {}</h1><ul>{}</ul><a href=\"/{}/\">Vote again?</a>",
            question.question_text, choices_html, question.id
        ),
    ))
}
```

---

## What's Real vs. What Django Has That Djangors Doesn't Yet

> [!IMPORTANT]
> **Key Architecture Differences from Django:**
> - **URL Syntax**: Djangors uses path parameters formatted as `/{param_name:type}/` (e.g., `/{question_id:i64}/`).
> - **Route Reversal**: Named route resolution (`reverse()`) is not used in `examples/polls`. Views build URL strings explicitly via `format!("/{}/results/", question_id)`.
> - **Templates vs. String Formatting**: The `examples/polls` application generates HTML dynamically using Rust `format!()` strings.
> - **Explicit DB Access**: Views retrieve the `Database` handle from request extensions via `req.state::<Database>()`.

---

## Running and Verifying

Start the development server:

```bash
DATABASE_URL="postgres://postgres:postgres@localhost/djangors_dev" dj run --port 8000
```

Test the views:

```bash
# Get poll index
curl http://localhost:8000/

# Get question detail for ID 1
curl http://localhost:8000/1/

# Get question results for ID 1
curl http://localhost:8000/1/results/
```
