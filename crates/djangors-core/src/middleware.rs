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
