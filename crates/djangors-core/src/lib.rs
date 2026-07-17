//! HTTP kernel for the Djangors web framework.
//!
//! Provides the core [`Request`], [`Response`], [`Router`], and [`Handler`]
//! types that form the foundation of Djangors's HTTP layer.

pub mod app;
pub mod debug_page;
pub mod error;
pub mod extract;
pub mod handler;
pub mod middleware;
pub mod path_params;
pub mod request;
pub mod response;
pub mod router;
pub mod service;
pub mod settings;

pub use app::Djangors;
pub use error::DjangorsError;
pub use handler::Handler;
pub use path_params::PathParams;
pub use request::Request;
pub use response::Response;
pub use router::Router;
pub use settings::DjangorsSettings;

/// Re-export of [`hyper::StatusCode`] for convenience.
pub use hyper::StatusCode;

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
