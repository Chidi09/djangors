use std::convert::Infallible;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use tower::layer::Layer;
use tower::Service;

/// Logs each request's method, path, status, and latency via `tracing`.
/// Roughly equivalent to Django's request logging.
pub fn logging_layer() -> tower_http::trace::TraceLayer<tower_http::trace::HttpMakeClassifier> {
    tower_http::trace::TraceLayer::new_for_http()
}

/// Sets security-related response headers mimicking Django's `SecurityMiddleware`:
/// `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
/// and `Referrer-Policy: same-origin`.
#[derive(Clone)]
pub struct SecurityHeadersLayer;

impl<S> Layer<S> for SecurityHeadersLayer
where
    S: Service<
            hyper::Request<Incoming>,
            Response = hyper::Response<Full<Bytes>>,
            Error = Infallible,
        > + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Service = tower_http::set_header::SetResponseHeader<
        tower_http::set_header::SetResponseHeader<
            tower_http::set_header::SetResponseHeader<S, hyper::http::HeaderValue>,
            hyper::http::HeaderValue,
        >,
        hyper::http::HeaderValue,
    >;

    fn layer(&self, inner: S) -> Self::Service {
        tower_http::set_header::SetResponseHeaderLayer::overriding(
            hyper::http::HeaderName::from_static("x-content-type-options"),
            hyper::http::HeaderValue::from_static("nosniff"),
        )
        .layer(
            tower_http::set_header::SetResponseHeaderLayer::overriding(
                hyper::http::HeaderName::from_static("x-frame-options"),
                hyper::http::HeaderValue::from_static("DENY"),
            )
            .layer(
                tower_http::set_header::SetResponseHeaderLayer::overriding(
                    hyper::http::HeaderName::from_static("referrer-policy"),
                    hyper::http::HeaderValue::from_static("same-origin"),
                )
                .layer(inner),
            ),
        )
    }
}

/// Returns a [`SecurityHeadersLayer`].
pub fn security_headers_layer() -> SecurityHeadersLayer {
    SecurityHeadersLayer
}

/// Normalizes request paths by trimming trailing slashes.
/// Roughly equivalent to Django's `CommonMiddleware` trailing-slash behavior.
pub fn normalize_path_layer() -> tower_http::normalize_path::NormalizePathLayer {
    tower_http::normalize_path::NormalizePathLayer::trim_trailing_slash()
}

/// Compresses response bodies using gzip and brotli encoding.
pub fn compression_layer() -> tower_http::compression::CompressionLayer {
    tower_http::compression::CompressionLayer::new()
}

/// Sets and propagates the `X-Request-ID` header on every request/response using a
/// UUID generator. Roughly equivalent to how Django's `CommonMiddleware` or system
/// request-id middleware works.
#[derive(Clone)]
pub struct RequestIdLayer;

impl<S> Layer<S> for RequestIdLayer
where
    S: Service<
            hyper::Request<Incoming>,
            Response = hyper::Response<Full<Bytes>>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Service = tower_http::request_id::SetRequestId<
        tower_http::request_id::PropagateRequestId<S>,
        tower_http::request_id::MakeRequestUuid,
    >;

    fn layer(&self, inner: S) -> Self::Service {
        let header = hyper::http::HeaderName::from_static("x-request-id");
        tower_http::request_id::SetRequestIdLayer::new(
            header.clone(),
            tower_http::request_id::MakeRequestUuid,
        )
        .layer(tower_http::request_id::PropagateRequestIdLayer::new(header).layer(inner))
    }
}

/// Returns a [`RequestIdLayer`].
pub fn request_id_layer() -> RequestIdLayer {
    RequestIdLayer
}

/// Cheaply `Clone`, inserted into the request's extensions by `CsrfLayer`
/// (via the same `Request::ext::<T>()` mechanism `Session` uses) so
/// handlers/templates can read the current request's token value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsrfToken(pub String);

/// CSRF protection middleware implementing a double-submit cookie scheme.
///
/// **CRITICAL SECURITY NOTE (v1 Scope):**
/// This middleware only validates CSRF tokens via the custom HTTP header (default: `X-CSRFToken`).
/// It does **not** look at or validate form body fields (e.g. `csrfmiddlewaretoken`).
/// Consequently, classic HTML `<form>` submissions without client-side JavaScript headers are
/// NOT protected in this version.
#[derive(Clone)]
pub struct CsrfLayer {
    cookie_name: String,
    header_name: String,
}

impl CsrfLayer {
    /// Creates a new `CsrfLayer` with default settings:
    /// - Cookie name: `csrftoken`
    /// - Header name: `x-csrftoken`
    pub fn new() -> Self {
        Self {
            cookie_name: "csrftoken".to_string(),
            header_name: "x-csrftoken".to_string(),
        }
    }

    /// Sets a custom cookie name.
    pub fn with_cookie_name(mut self, name: String) -> Self {
        self.cookie_name = name;
        self
    }

    /// Sets a custom header name.
    pub fn with_header_name(mut self, name: String) -> Self {
        self.header_name = name;
        self
    }
}

impl Default for CsrfLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns a default [`CsrfLayer`].
pub fn csrf_layer() -> CsrfLayer {
    CsrfLayer::new()
}

impl<S> Layer<S> for CsrfLayer {
    type Service = CsrfService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CsrfService {
            inner,
            cookie_name: self.cookie_name.clone(),
            header_name: self.header_name.clone(),
        }
    }
}

#[derive(Clone)]
pub struct CsrfService<S> {
    inner: S,
    cookie_name: String,
    header_name: String,
}

fn extract_cookie(headers: &hyper::HeaderMap, cookie_name: &str) -> Option<String> {
    for cookie_header in headers.get_all(hyper::header::COOKIE) {
        let cookie_str = cookie_header.to_str().ok()?;
        for cookie in cookie_str.split(';') {
            let mut parts = cookie.trim().splitn(2, '=');
            let name = parts.next()?;
            if name == cookie_name {
                let val = parts.next()?;
                return Some(val.to_string());
            }
        }
    }
    None
}

fn generate_csrf_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        write!(&mut s, "{:02x}", b).unwrap();
    }
    s
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

impl<S, B> Service<hyper::Request<B>> for CsrfService<S>
where
    S: Service<hyper::Request<B>, Response = hyper::Response<Full<Bytes>>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = hyper::Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<B>) -> Self::Future {
        let mut inner = self.inner.clone();
        let cookie_name = self.cookie_name.clone();
        let header_name = self.header_name.clone();

        let cookie_val = extract_cookie(req.headers(), &cookie_name);
        let mut new_token_generated = false;
        let token_value = match cookie_val {
            Some(val) if !val.is_empty() => val,
            _ => {
                new_token_generated = true;
                generate_csrf_token()
            }
        };

        let method = req.method();
        let is_unsafe = method == hyper::Method::POST
            || method == hyper::Method::PUT
            || method == hyper::Method::PATCH
            || method == hyper::Method::DELETE;

        if is_unsafe {
            let mut is_valid = false;
            if let Some(header_val) = req.headers().get(&header_name) {
                if constant_time_eq(header_val.as_bytes(), token_value.as_bytes()) {
                    is_valid = true;
                }
            }
            if !is_valid {
                let response = hyper::Response::builder()
                    .status(403)
                    .body(Full::new(Bytes::from(
                        "403 Forbidden: CSRF token missing or invalid",
                    )))
                    .unwrap();
                return Box::pin(async move { Ok(response) });
            }
        }

        req.extensions_mut().insert(CsrfToken(token_value.clone()));

        Box::pin(async move {
            let mut resp = inner.call(req).await?;

            if new_token_generated {
                // Deliberately not setting HttpOnly (unlike the session cookie):
                // the token must be readable by client-side JS to populate
                // the X-CSRFToken header on AJAX requests, matching Django's
                // own default behavior.
                let set_cookie_val = format!(
                    "{}={}; Path=/; SameSite=Lax; Max-Age=31536000",
                    cookie_name, token_value
                );
                if let Ok(hdr_val) = hyper::header::HeaderValue::from_str(&set_cookie_val) {
                    resp.headers_mut()
                        .append(hyper::header::SET_COOKIE, hdr_val);
                }
            }

            Ok(resp)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
    use hyper::header::{COOKIE, SET_COOKIE};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tower::service_fn;
    use tower::ServiceExt;

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
        assert!(!constant_time_eq(b"hell", b"hello"));
    }

    #[tokio::test]
    async fn test_safe_method_no_cookie_no_header() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let inner_svc = service_fn(move |_req: hyper::Request<Full<Bytes>>| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            let resp = hyper::Response::builder()
                .status(200)
                .body(Full::new(Bytes::from("ok")))
                .unwrap();
            async move { Ok::<_, Infallible>(resp) }
        });

        let mut svc = CsrfLayer::new().layer(inner_svc);

        let req = hyper::Request::builder()
            .method("GET")
            .uri("/")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Response must set a fresh csrftoken cookie
        let set_cookie_header = resp.headers().get(SET_COOKIE).expect("Should set cookie");
        let set_cookie_str = set_cookie_header.to_str().unwrap();
        assert!(set_cookie_str.contains("csrftoken="));
        assert!(set_cookie_str.contains("Path=/"));
        assert!(set_cookie_str.contains("SameSite=Lax"));
        assert!(set_cookie_str.contains("Max-Age=31536000"));
        assert!(!set_cookie_str.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn test_unsafe_method_no_cookie_no_header_rejected() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let inner_svc = service_fn(move |_req: hyper::Request<Full<Bytes>>| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            let resp = hyper::Response::builder()
                .status(200)
                .body(Full::new(Bytes::from("ok")))
                .unwrap();
            async move { Ok::<_, Infallible>(resp) }
        });

        let mut svc = CsrfLayer::new().layer(inner_svc);

        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), 403);
        assert_eq!(counter.load(Ordering::SeqCst), 0); // Inner handler never called
    }

    #[tokio::test]
    async fn test_unsafe_method_cookie_present_missing_or_mismatched_header_rejected() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let inner_svc = service_fn(move |_req: hyper::Request<Full<Bytes>>| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            let resp = hyper::Response::builder()
                .status(200)
                .body(Full::new(Bytes::from("ok")))
                .unwrap();
            async move { Ok::<_, Infallible>(resp) }
        });

        let mut svc = CsrfLayer::new().layer(inner_svc);

        // Case 1: Cookie present, header missing
        let req1 = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .header(COOKIE, "csrftoken=validtoken1234567890123456789012")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp1 = svc.ready().await.unwrap().call(req1).await.unwrap();
        assert_eq!(resp1.status(), 403);
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        // Case 2: Cookie present, header mismatched
        let req2 = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .header(COOKIE, "csrftoken=validtoken1234567890123456789012")
            .header("X-CSRFToken", "differentvalidtokenvaluehere12345")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp2 = svc.ready().await.unwrap().call(req2).await.unwrap();
        assert_eq!(resp2.status(), 403);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_unsafe_method_cookie_present_matching_header_succeeds() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let inner_svc = service_fn(move |_req: hyper::Request<Full<Bytes>>| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            let resp = hyper::Response::builder()
                .status(200)
                .body(Full::new(Bytes::from("ok")))
                .unwrap();
            async move { Ok::<_, Infallible>(resp) }
        });

        let mut svc = CsrfLayer::new().layer(inner_svc);

        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .header(COOKIE, "csrftoken=validtoken123")
            .header("X-CSRFToken", "validtoken123")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_end_to_end_router_csrf() {
        use tower::service_fn;
        use tower::ServiceBuilder;

        async fn test_handler(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
            let token = req.ext::<CsrfToken>().expect("CsrfToken should exist");
            Ok(Response::text(StatusCode::OK, &token.0))
        }

        let router = Router::new().get("/", test_handler).post("/", test_handler);

        let layer = CsrfLayer::new();

        let svc_fn = service_fn(move |req: hyper::Request<Full<Bytes>>| {
            let router = router.clone();
            async move { Ok::<_, Infallible>(router.dispatch(req).await) }
        });

        let mut svc = ServiceBuilder::new().layer(layer).service(svc_fn);

        // 1. GET to obtain Set-Cookie
        let req1 = hyper::Request::builder()
            .method("GET")
            .uri("/")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp1 = svc.ready().await.unwrap().call(req1).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        let set_cookie_header = resp1.headers().get(SET_COOKIE).expect("Should set cookie");
        let set_cookie_str = set_cookie_header.to_str().unwrap().to_string();
        let cookie_value = set_cookie_str
            .split(';')
            .next()
            .unwrap()
            .split_once('=')
            .unwrap()
            .1
            .to_string();

        // 2. POST carrying cookie and matching header
        let req2 = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .header(COOKIE, format!("csrftoken={}", cookie_value))
            .header("X-CSRFToken", &cookie_value)
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp2 = svc.ready().await.unwrap().call(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        // 3. POST carrying cookie but wrong/mismatched header value
        let req3 = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .header(COOKIE, format!("csrftoken={}", cookie_value))
            .header("X-CSRFToken", "mismatchedtokenvalue1234567890123")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp3 = svc.ready().await.unwrap().call(req3).await.unwrap();
        assert_eq!(resp3.status(), StatusCode::FORBIDDEN);
    }
}
