use crate::views;
use djangors_core::Router;

pub fn urls() -> Router {
    Router::new()
        .post("/accounts/login/", views::login_view)
        .post("/accounts/logout/", views::logout_view)
        .mount("/admin", crate::admin::admin_site().urls())
}
