use crate::views;
use djangors_core::Router;

pub fn urls() -> Router {
    djangors_admin::favicon_routes(
        Router::new()
            .get("/healthz", views::healthz)
            .post("/accounts/login/", views::login_view)
            .post("/accounts/logout/", views::logout_view)
            .mount("/admin", crate::admin::admin_site().urls()),
    )
}
