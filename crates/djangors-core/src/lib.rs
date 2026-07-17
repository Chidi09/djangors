//! HTTP kernel for the Djangors web framework.
//!
//! Provides the core [`Request`], [`Response`], [`Router`], and [`Handler`]
//! types that form the foundation of Djangors's HTTP layer.
//!
//! # CSRF Protection (v1 Scope)
//!
//! **CRITICAL SECURITY NOTE:**
//! CSRF middleware in v1 only validates unsafe requests (e.g. POST, PUT, PATCH, DELETE)
//! via the `X-CSRFToken` header. It does **not** validate CSRF tokens passed inside a
//! form body field (like Django's `csrfmiddlewaretoken`). Consequently:
//! - **Protected:** JSON/AJAX-style requests that set the `X-CSRFToken` header.
//! - **NOT Protected:** Classic `<form method="post">` HTML form submissions without JavaScript.
//!
//! **BREACH Defense (Future Work):**
//! This version uses a double-submit cookie scheme. Django's BREACH-hardened masked-secret
//! scheme is not yet implemented and is planned as future work.

pub mod app;
pub mod debug_page;
pub mod error;
pub mod extract;
pub mod handler;
pub mod logging;
pub mod middleware;
pub mod path_params;
pub mod request;
pub mod response;
pub mod router;
pub mod service;
pub mod settings;
pub mod signals;
pub mod state;

pub use app::Djangors;
pub use error::DjangorsError;
pub use handler::Handler;
pub use path_params::PathParams;
pub use request::Request;
pub use response::Response;
pub use router::Router;
pub use settings::DjangorsSettings;
pub use state::AppState;

/// Re-export of [`hyper::StatusCode`] for convenience.
pub use hyper::StatusCode;

pub fn html_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            '/' => escaped.push_str("&#x2F;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod integration_tests {
    use std::str::FromStr;

    use bytes::Bytes;
    use hyper::http::{HeaderMap, Method, Uri};

    use crate::error::DjangorsError;
    use crate::path_params::PathParams;
    use crate::request::Request;
    use crate::response::Response;
    use crate::router::Router;
    use crate::StatusCode;

    async fn index_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::OK, "home"))
    }

    async fn user_detail_handler_fn(
        _: Request,
        params: PathParams,
    ) -> Result<Response, DjangorsError> {
        let id: i64 = params
            .get_as("id")
            .map_err(|_| DjangorsError::BadRequest("bad id".into()))?;
        Ok(Response::text(StatusCode::OK, &format!("user {id}")))
    }

    async fn create_user_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::CREATED, "created"))
    }

    fn make_request(method: Method, path: &str) -> Request {
        let uri = Uri::from_str(path).expect("valid URI");
        Request::new(method, uri, HeaderMap::new(), Bytes::new())
    }

    fn body_str(resp: &Response) -> String {
        String::from_utf8(resp.body().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn full_route_dispatch_integration() {
        let router = Router::new()
            .get("/", index_handler_fn)
            .get("/users/{id:i64}", user_detail_handler_fn)
            .post("/users", create_user_handler_fn);

        // Test root
        let req = make_request(Method::GET, "/");
        let resp = router.handle(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_str(&resp), "home");

        // Test GET with i64 param
        let req = make_request(Method::GET, "/users/42");
        let resp = router.handle(req).await.unwrap();
        assert_eq!(body_str(&resp), "user 42");

        // Test POST
        let req = make_request(Method::POST, "/users");
        let resp = router.handle(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert_eq!(body_str(&resp), "created");

        // Test 404
        let req = make_request(Method::GET, "/notfound");
        let err = router.handle(req).await.unwrap_err();
        assert!(matches!(err, DjangorsError::NotFound));
    }
}
