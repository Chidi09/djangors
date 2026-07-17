use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use hyper::http::Method;

use crate::error::DjangorsError;
use crate::handler::Handler;
use crate::path_params::PathParams;
use crate::request::Request;
use crate::response::Response;

#[derive(Debug, Clone)]
enum CaptureType {
    String,
    I64,
    Slug,
}

#[derive(Debug, Clone)]
enum Segment {
    Literal(String),
    Capture(CaptureType),
}

#[derive(Clone)]
struct Route {
    pattern: String,
    method: Method,
    segments: Vec<Segment>,
    param_names: Vec<String>,
    handler: Arc<dyn Handler>,
}

/// A URL router that matches incoming requests to registered handlers.
///
/// Routes are defined with Django-style path syntax:
/// - Literal segments match exactly (e.g. `hello/world`)
/// - `{name}` captures any single path segment as a `String`
/// - `{name:i64}` captures a segment that must parse as `i64`
/// - `{name:slug}` captures a segment matching `[a-zA-Z0-9_-]+`
///
/// Routers can be nested with [`mount`](Self::mount).
///
/// `Router` is cheaply [`Clone`]-able (the route table is `Arc`-wrapped) so it
/// can be cloned per-request by `tower::Service` without deep-copying every
/// registered route.
#[derive(Clone)]
pub struct Router {
    routes: Arc<Vec<Route>>,
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl Router {
    /// Create an empty router.
    pub fn new() -> Self {
        Router {
            routes: Arc::new(Vec::new()),
        }
    }

    /// Register a handler for the given path and HTTP method.
    ///
    /// # Panics
    /// Panics if the path pattern contains invalid capture syntax.
    pub fn route(mut self, path: &str, method: Method, handler: impl Handler + 'static) -> Self {
        let pattern = path
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string();
        let (segments, param_names) = Self::parse_pattern(&pattern);
        Arc::make_mut(&mut self.routes).push(Route {
            pattern,
            method,
            segments,
            param_names,
            handler: Arc::new(handler),
        });
        self
    }

    /// Register a GET handler.
    pub fn get(self, path: &str, handler: impl Handler + 'static) -> Self {
        self.route(path, Method::GET, handler)
    }

    /// Register a POST handler.
    pub fn post(self, path: &str, handler: impl Handler + 'static) -> Self {
        self.route(path, Method::POST, handler)
    }

    /// Register a PUT handler.
    pub fn put(self, path: &str, handler: impl Handler + 'static) -> Self {
        self.route(path, Method::PUT, handler)
    }

    /// Register a DELETE handler.
    pub fn delete(self, path: &str, handler: impl Handler + 'static) -> Self {
        self.route(path, Method::DELETE, handler)
    }

    /// Mount a sub-router at the given prefix.
    ///
    /// All routes from `sub_router` are registered with `prefix` prepended.
    /// For example, mounting a router with route `users/{id}` at `/api`
    /// creates a route matching `/api/users/{id}`.
    pub fn mount(mut self, prefix: &str, sub_router: Router) -> Self {
        let prefix = prefix.trim_start_matches('/').trim_end_matches('/');
        for route in sub_router.routes.iter() {
            let full_pattern = if route.pattern.is_empty() {
                prefix.to_string()
            } else {
                format!("{}/{}", prefix, route.pattern)
            };
            let (segments, param_names) = Self::parse_pattern(&full_pattern);
            Arc::make_mut(&mut self.routes).push(Route {
                pattern: full_pattern,
                method: route.method.clone(),
                segments,
                param_names,
                handler: route.handler.clone(),
            });
        }
        self
    }

    /// Parse a path pattern into segments and parameter names.
    fn parse_pattern(pattern: &str) -> (Vec<Segment>, Vec<String>) {
        let mut segments = Vec::new();
        let mut param_names = Vec::new();

        if pattern.is_empty() {
            return (segments, param_names);
        }

        for part in pattern.split('/') {
            if part.is_empty() {
                continue;
            }
            if part.starts_with('{') && part.ends_with('}') {
                let inner = &part[1..part.len() - 1];
                let (name, capture_type) = if let Some((name, ty)) = inner.split_once(':') {
                    let ct = match ty {
                        "i64" => CaptureType::I64,
                        "slug" => CaptureType::Slug,
                        _ => CaptureType::String,
                    };
                    (name, ct)
                } else {
                    (inner, CaptureType::String)
                };
                param_names.push(name.to_string());
                segments.push(Segment::Capture(capture_type));
            } else {
                segments.push(Segment::Literal(part.to_string()));
            }
        }

        (segments, param_names)
    }

    /// Try to match a path and method against registered routes.
    fn match_path(&self, path: &str, method: &Method) -> Option<(usize, PathParams)> {
        let normalized = path.trim_start_matches('/').trim_end_matches('/');
        let parts: Vec<&str> = if normalized.is_empty() {
            Vec::new()
        } else {
            normalized.split('/').collect()
        };

        for (idx, route) in self.routes.iter().enumerate() {
            if route.method != *method {
                continue;
            }
            if route.segments.len() != parts.len() {
                continue;
            }

            let mut params = PathParams::new();
            let mut matched = true;
            let mut param_idx = 0;

            for (i, segment) in route.segments.iter().enumerate() {
                let matched_segment = match segment {
                    Segment::Literal(expected) => parts[i] == expected.as_str(),
                    Segment::Capture(ct) => {
                        let valid = match ct {
                            CaptureType::I64 => parts[i].parse::<i64>().is_ok(),
                            CaptureType::Slug => parts[i]
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                            CaptureType::String => true,
                        };
                        if valid {
                            if param_idx < route.param_names.len() {
                                params.insert(&route.param_names[param_idx], parts[i]);
                            }
                            param_idx += 1;
                        }
                        valid
                    }
                };

                if !matched_segment {
                    matched = false;
                    break;
                }
            }

            if matched {
                return Some((idx, params));
            }
        }

        None
    }

    /// Dispatch a fully constructed [`Request`] to the matching handler.
    ///
    /// Matches the request's path and method against registered routes, calls
    /// the matching handler, and returns the response. Returns a 404 error if
    /// no route matches.
    pub async fn handle(&self, req: Request) -> Result<Response, DjangorsError> {
        let path = req.path().to_string();
        let method = req.method().clone();

        match self.match_path(&path, &method) {
            Some((idx, params)) => {
                let handler = &self.routes[idx].handler;
                handler.call(req, params).await
            }
            None => Err(DjangorsError::NotFound),
        }
    }

    /// Dispatch an incoming hyper request, consuming the body, and return a
    /// hyper response.
    ///
    /// This is the top-level entry point intended for use by server bindings.
    /// Body read errors are converted to 500 responses.
    pub async fn dispatch<B>(
        &self,
        hyper_req: hyper::Request<B>,
    ) -> hyper::Response<http_body_util::Full<Bytes>>
    where
        B: hyper::body::Body<Data = Bytes> + Send,
        B::Error: fmt::Display,
    {
        let (parts, body) = hyper_req.into_parts();

        let body_bytes = match http_body_util::BodyExt::collect(body).await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                let err = DjangorsError::Internal(format!("failed to read request body: {e}"));
                return err.into_response().into_hyper();
            }
        };

        let req = Request::new(parts.method, parts.uri, parts.headers, body_bytes);

        match self.handle(req).await {
            Ok(resp) => resp.into_hyper(),
            Err(e) => e.into_response().into_hyper(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use hyper::http::{HeaderMap, Method, StatusCode, Uri};
    use std::str::FromStr;

    fn make_request(method: Method, path: &str) -> Request {
        let uri = Uri::from_str(path).expect("valid URI");
        Request::new(method, uri, HeaderMap::new(), Bytes::new())
    }

    async fn ok_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::OK, "ok"))
    }

    async fn echo_name_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let name = params.get("name").unwrap_or("?");
        Ok(Response::text(StatusCode::OK, name))
    }

    async fn echo_id_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let id: i64 = params.get_as("id").unwrap_or(0);
        Ok(Response::text(StatusCode::OK, &format!("item {id}")))
    }

    async fn echo_slug_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let slug = params.get("slug").unwrap_or("?");
        Ok(Response::text(StatusCode::OK, slug))
    }

    async fn echo_two_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let a = params.get("a").unwrap_or("?");
        let b: i64 = params.get_as("b").unwrap_or(0);
        Ok(Response::text(StatusCode::OK, &format!("{a}/{b}")))
    }

    async fn get_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::OK, "get"))
    }

    async fn post_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::OK, "post"))
    }

    async fn mount_handler_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let id = params.get("id").unwrap_or("?");
        Ok(Response::text(StatusCode::OK, id))
    }

    async fn root_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::OK, "root"))
    }

    #[tokio::test]
    async fn literal_route_match() {
        let router = Router::new().get("/hello", ok_handler_fn);
        let req = make_request(Method::GET, "/hello");
        let resp = router.handle(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn literal_route_no_match() {
        let router = Router::new().get("/hello", ok_handler_fn);
        let req = make_request(Method::GET, "/world");
        let result = router.handle(req).await;
        assert!(matches!(result.unwrap_err(), DjangorsError::NotFound));
    }

    #[tokio::test]
    async fn method_mismatch() {
        let router = Router::new().get("/hello", ok_handler_fn);
        let req = make_request(Method::POST, "/hello");
        let result = router.handle(req).await;
        assert!(matches!(result.unwrap_err(), DjangorsError::NotFound));
    }

    #[tokio::test]
    async fn string_capture() {
        let router = Router::new().get("/hello/{name}", echo_name_fn);
        let req = make_request(Method::GET, "/hello/world");
        let resp = router.handle(req).await.unwrap();
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body, "world");
    }

    #[tokio::test]
    async fn i64_capture_valid() {
        let router = Router::new().get("/item/{id:i64}", echo_id_fn);
        let req = make_request(Method::GET, "/item/42");
        let resp = router.handle(req).await.unwrap();
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body, "item 42");
    }

    #[tokio::test]
    async fn i64_capture_invalid() {
        let router = Router::new().get("/item/{id:i64}", echo_id_fn);
        let req = make_request(Method::GET, "/item/abc");
        let result = router.handle(req).await;
        assert!(matches!(result.unwrap_err(), DjangorsError::NotFound));
    }

    #[tokio::test]
    async fn slug_capture_valid() {
        let router = Router::new().get("/post/{slug:slug}", echo_slug_fn);
        let req = make_request(Method::GET, "/post/my-blog_post-123");
        let resp = router.handle(req).await.unwrap();
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body, "my-blog_post-123");
    }

    #[tokio::test]
    async fn slug_capture_invalid() {
        let router = Router::new().get("/post/{slug:slug}", echo_slug_fn);
        let req = make_request(Method::GET, "/post/hello.world");
        let result = router.handle(req).await;
        assert!(matches!(result.unwrap_err(), DjangorsError::NotFound));
    }

    #[tokio::test]
    async fn multiple_captures() {
        let router = Router::new().get("/{a}/{b:i64}", echo_two_fn);
        let req = make_request(Method::GET, "/foo/99");
        let resp = router.handle(req).await.unwrap();
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body, "foo/99");
    }

    #[tokio::test]
    async fn post_route() {
        let router = Router::new()
            .get("/a", get_handler_fn)
            .post("/a", post_handler_fn);
        let req_post = make_request(Method::POST, "/a");
        let resp = router.handle(req_post).await.unwrap();
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body, "post");
    }

    #[tokio::test]
    async fn mount_sub_router() {
        let sub = Router::new().get("/users/{id}", mount_handler_fn);
        let router = Router::new().mount("/api", sub);
        let req = make_request(Method::GET, "/api/users/7");
        let resp = router.handle(req).await.unwrap();
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body, "7");
    }

    #[tokio::test]
    async fn root_path_match() {
        let router = Router::new().get("/", root_handler_fn);
        let req = make_request(Method::GET, "/");
        let resp = router.handle(req).await.unwrap();
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body, "root");
    }

    #[tokio::test]
    async fn integration_dispatch() {
        async fn handler_func(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
            let name = params.get("name").unwrap_or("?");
            Ok(Response::text(StatusCode::OK, &format!("Hello, {name}!")))
        }

        let router = Router::new().get("/greet/{name}", handler_func);

        let req = make_request(Method::GET, "/greet/Alice");
        let resp = router.handle(req).await.unwrap();
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body, "Hello, Alice!");
    }

    #[tokio::test]
    async fn plain_async_fn_handler_no_wrapping() {
        async fn greet(_req: Request, params: PathParams) -> Result<Response, DjangorsError> {
            let name = params.get("name").unwrap_or("stranger");
            Ok(Response::text(StatusCode::OK, &format!("Hello, {name}!")))
        }

        let router = Router::new().get("/greet/{name}", greet);

        let req = make_request(Method::GET, "/greet/World");
        let resp = router.handle(req).await.unwrap();
        let body = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body, "Hello, World!");
    }
}
