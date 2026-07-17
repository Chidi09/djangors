use djangors_core::Router;

use crate::views;

pub fn urls() -> Router {
    Router::new()
        .get("/", views::index)
        .get("/{question_id:i64}/", views::detail)
        .get("/{question_id:i64}/results/", views::results)
        .post("/{question_id:i64}/vote/", views::vote)
        .post("/accounts/login/", views::login_view)
        .post("/accounts/logout/", views::logout_view)
}
