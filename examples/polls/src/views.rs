//! Mixed: the HTTP plumbing (imports, handler signatures, `Response`/`Request`
//! usage, error propagation) is REAL and works today against `djangors-core`.
//! The database/ORM calls (`Question::objects()...`, `req.db()`) are
//! ASPIRATIONAL — Phase 2 territory. See README.md.

use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};

use crate::models::{Choice, Question};

/// REAL: this is a plain `async fn` with the exact signature
/// `Fn(Request, PathParams) -> impl Future<Output = Result<Response, DjangorsError>>`
/// that `Handler`'s blanket impl (djangors-core/src/handler.rs) accepts
/// directly — no `#[handler]` macro, no manual `Box::pin` wrapping. Register
/// it with `.get("/", index)` in urls.rs exactly as written.
pub async fn index(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    // ASPIRATIONAL from here down: `req.db()` (a connection pulled from
    // per-request app state) and the QuerySet chain don't exist yet.
    let latest_question_list = Question::objects()
        .filter(q!(pub_date__lte = chrono::Utc::now()))
        .order_by("-pub_date")
        .limit(5)
        .all(req.db())
        .await?;

    // REAL: Response::html already exists. Template rendering (rango-template,
    // Phase 3) isn't built yet, so this stands in for what will eventually be
    // `render(&req, "polls/index.html", context! { latest_question_list })`.
    let body = latest_question_list
        .iter()
        .map(|q| format!("<li>{}</li>", q.question_text))
        .collect::<String>();
    Ok(Response::html(StatusCode::OK, format!("<ul>{body}</ul>")))
}

/// REAL signature/plumbing; ASPIRATIONAL body (ORM `get_or_404`).
pub async fn detail(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    // REAL: PathParams::get_as::<i64> exists today.
    let question_id: i64 = params.get_as("question_id")?;

    // ASPIRATIONAL: Question::objects().get_or_404(...) — the ORM's
    // "fetch or return a 404 DjangorsError" convenience, mirroring Django's
    // get_object_or_404().
    let question = Question::objects()
        .get_or_404(req.db(), question_id)
        .await?;

    Ok(Response::html(
        StatusCode::OK,
        format!("<h1>{}</h1>", question.question_text),
    ))
}

/// REAL signature/plumbing including the `Form` extractor (djangors-core's
/// `extract.rs`, Phase 1); ASPIRATIONAL body (ORM update + F() expression).
pub async fn vote(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let question_id: i64 = params.get_as("question_id")?;

    // REAL: manual FromRequest call — Form<T>/Json<T>/Query<T> exist today
    // but aren't yet auto-extracted as handler parameters (noted as a
    // deliberate scope cut in the extract.rs module doc comment).
    #[derive(serde::Deserialize)]
    struct VoteForm {
        choice: i64,
    }
    let Form(vote) = Form::<VoteForm>::from_request(&req).await?;

    // ASPIRATIONAL: get_or_404, F() race-safe increment (Django's F()
    // objects, avoiding a read-modify-write race), set!() macro.
    let question = Question::objects()
        .get_or_404(req.db(), question_id)
        .await?;
    Choice::objects()
        .filter(q!(question = question.id, id = vote.choice))
        .update(req.db(), set!(votes = F("votes") + 1))
        .await?;

    // ASPIRATIONAL: reverse!() named-route reversal (Phase 1's router has
    // path matching but not named-route reversal yet).
    Ok(Response::redirect(&reverse!("polls:results", question_id)))
}

pub async fn results(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let question_id: i64 = params.get_as("question_id")?;
    let question = Question::objects()
        .get_or_404(req.db(), question_id)
        .await?;
    Ok(Response::html(
        StatusCode::OK,
        format!("<h1>Results for {}</h1>", question.question_text),
    ))
}
