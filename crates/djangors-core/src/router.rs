use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use hyper::http::Method;

use crate::error::DjangorsError;
use crate::handler::Handler;
use crate::path_params::PathParams;
use crate::request::Request;
use crate::response::Response;
use crate::state::AppState;

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
    state: AppState,
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
            state: AppState::new(),
        }
    }

    /// Attach a piece of shared state (e.g. a database connection pool) that
    /// handlers can retrieve via [`Request::state`]. Can be called
    /// multiple times with different types to attach several independent
    /// pieces of state.
    ///
    /// Note: Sub-routers mounted using [`mount`](Self::mount) will NOT merge
    /// their own state with the parent. State should be configured on the top-level
    /// outer router that is served.
    pub fn with_state<T: Send + Sync + 'static>(mut self, value: T) -> Self {
        self.state = self.state.insert(value);
        self
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
    ///
    /// # Panic Isolation
    ///
    /// Handler execution is wrapped in a task spawned via `tokio::spawn` to catch
    /// and isolate any panics. A panicking handler must not take down other
    /// in-flight requests or crash the whole server process.
    pub async fn handle(&self, req: Request) -> Result<Response, DjangorsError> {
        let path = req.path().to_string();
        let method = req.method().clone();

        crate::signals::REQUEST_STARTED
            .send(crate::signals::RequestStarted {
                method: method.to_string(),
                path: path.clone(),
            })
            .await;

        let res = match self.match_path(&path, &method) {
            Some((idx, params)) => {
                let handler = self.routes[idx].handler.clone();
                // Spawn a new task to isolate the handler future. This allows catching
                // any panics that occur during execution and safely converting them to errors.
                let join_handle = tokio::spawn(async move { handler.call(req, params).await });
                match join_handle.await {
                    Ok(result) => result,
                    Err(join_err) => {
                        if join_err.is_panic() {
                            let payload = join_err.into_panic();
                            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                                s.to_string()
                            } else if let Some(s) = payload.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "unknown panic message".to_string()
                            };
                            Err(DjangorsError::Panicked(msg))
                        } else {
                            Err(DjangorsError::Internal(format!(
                                "handler task execution failed: {join_err}"
                            )))
                        }
                    }
                }
            }
            None => Err(DjangorsError::NotFound),
        };

        let status = match &res {
            Ok(resp) => resp.status().as_u16(),
            Err(err) => err.status_code().as_u16(),
        };

        crate::signals::REQUEST_FINISHED
            .send(crate::signals::RequestFinished {
                method: method.to_string(),
                path: path.clone(),
                status,
            })
            .await;

        res
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

        let req = Request::new(parts.method, parts.uri, parts.headers, body_bytes)
            .with_state(self.state.clone())
            .with_extensions(parts.extensions);

        match self.handle(req).await {
            Ok(resp) => resp.into_hyper(),
            Err(e) => e.into_response().into_hyper(),
        }
    }

    /// Dispatch an incoming hyper request, and render a Django-style debug page
    /// or a production-safe generic page depending on the `debug` setting.
    pub async fn dispatch_debug<B>(
        &self,
        hyper_req: hyper::Request<B>,
        debug: bool,
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
                let resp = if debug {
                    let dummy_req =
                        Request::new(parts.method, parts.uri, parts.headers, Bytes::new())
                            .with_state(self.state.clone());
                    crate::debug_page::render_debug_page(&err, &dummy_req)
                } else {
                    crate::debug_page::render_production_error_page(err.status_code())
                };
                return resp.into_hyper();
            }
        };

        let req = Request::new(parts.method, parts.uri, parts.headers, body_bytes)
            .with_state(self.state.clone())
            .with_extensions(parts.extensions);
        let req_clone = req.clone();

        match self.handle(req).await {
            Ok(resp) => resp.into_hyper(),
            Err(e) => {
                let resp = if debug {
                    crate::debug_page::render_debug_page(&e, &req_clone)
                } else {
                    crate::debug_page::render_production_error_page(e.status_code())
                };
                resp.into_hyper()
            }
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

    async fn panicking_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
        panic!("boom");
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

    #[tokio::test]
    async fn test_handler_panic_recovery() {
        let router = Router::new().get("/panic", panicking_handler_fn);
        let req = make_request(Method::GET, "/panic");
        let res = router.handle(req).await;

        assert!(res.is_err());
        let err = res.unwrap_err();
        if let DjangorsError::Panicked(msg) = err {
            assert_eq!(msg, "boom");
        } else {
            panic!("Expected DjangorsError::Panicked, got {:?}", err);
        }
    }

    #[tokio::test]
    async fn test_dispatch_debug_panic() {
        let router = Router::new().get("/panic", panicking_handler_fn);

        // Dispatch with debug = true
        let hyper_req = hyper::Request::builder()
            .method("GET")
            .uri("/panic")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();
        let hyper_resp = router.dispatch_debug(hyper_req, true).await;
        assert_eq!(hyper_resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

        use http_body_util::BodyExt;
        let body_bytes = hyper_resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(body_str.contains("Handler Panicked"));
        assert!(body_str.contains("boom"));
        assert!(body_str.contains("DEBUG = true"));

        // Dispatch with debug = false
        let hyper_req_prod = hyper::Request::builder()
            .method("GET")
            .uri("/panic")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();
        let hyper_resp_prod = router.dispatch_debug(hyper_req_prod, false).await;
        assert_eq!(hyper_resp_prod.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body_bytes_prod = hyper_resp_prod
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let body_str_prod = String::from_utf8(body_bytes_prod.to_vec()).unwrap();
        assert!(!body_str_prod.contains("boom"));
        assert!(!body_str_prod.contains("DEBUG = true"));
        assert!(body_str_prod.contains("Internal Server Error"));
    }

    #[tokio::test]
    async fn test_router_signals_fire() {
        use crate::signals::{REQUEST_FINISHED, REQUEST_STARTED};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let started_counter = Arc::new(AtomicUsize::new(0));
        let finished_counter = Arc::new(AtomicUsize::new(0));

        let started_clone = started_counter.clone();
        REQUEST_STARTED.connect(move |payload| {
            let started = started_clone.clone();
            async move {
                if payload.path == "/signals-test-unique-path-foo" {
                    started.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(payload.method, "GET");
                }
            }
        });

        let finished_clone = finished_counter.clone();
        REQUEST_FINISHED.connect(move |payload| {
            let finished = finished_clone.clone();
            async move {
                if payload.path == "/signals-test-unique-path-foo" {
                    finished.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(payload.method, "GET");
                    assert_eq!(payload.status, 200);
                }
            }
        });

        let router = Router::new().get(
            "/signals-test-unique-path-foo",
            |_: Request, _: PathParams| async { Ok(Response::text(StatusCode::OK, "ok")) },
        );

        let req = make_request(Method::GET, "/signals-test-unique-path-foo");
        let _resp = router.handle(req).await.unwrap();

        assert_eq!(started_counter.load(Ordering::SeqCst), 1);
        assert_eq!(finished_counter.load(Ordering::SeqCst), 1);
    }

    #[derive(Clone)]
    struct DbConnectionPool(String);

    async fn state_handler_fn(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
        let pool = req.state::<DbConnectionPool>().expect("pool should exist");
        Ok(Response::text(StatusCode::OK, &pool.0))
    }

    #[tokio::test]
    async fn test_router_app_state() {
        let router = Router::new()
            .with_state(DbConnectionPool("postgres://localhost".to_string()))
            .get("/state-test", state_handler_fn);

        let req = make_request(Method::GET, "/state-test").with_state(router.state.clone());
        let resp = router.handle(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_str = String::from_utf8(resp.body().to_vec()).unwrap();
        assert_eq!(body_str, "postgres://localhost");
    }

    #[derive(Clone)]
    struct FakeSession(String);

    async fn ext_handler_fn(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
        let session = req.ext::<FakeSession>().expect("extension should exist");
        Ok(Response::text(StatusCode::OK, &session.0))
    }

    #[tokio::test]
    async fn test_dispatch_propagates_hyper_extensions_to_handler() {
        let router = Router::new().get("/ext-test", ext_handler_fn);

        let mut hyper_req = hyper::Request::builder()
            .method(Method::GET)
            .uri("/ext-test")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();
        hyper_req
            .extensions_mut()
            .insert(FakeSession("injected-by-middleware".to_string()));

        let hyper_resp = router.dispatch(hyper_req).await;
        assert_eq!(hyper_resp.status(), StatusCode::OK);

        use http_body_util::BodyExt;
        let body_bytes = hyper_resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body_bytes[..], b"injected-by-middleware");
    }
}
