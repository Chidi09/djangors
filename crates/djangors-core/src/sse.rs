use std::pin::Pin;

use bytes::Bytes;
use futures_util::stream::Stream;
use futures_util::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::http::header::{CACHE_CONTROL, CONNECTION, CONTENT_TYPE};
use hyper::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

/// A streaming HTTP response backed by an async [`Stream`] of [`Bytes`] chunks.
///
/// Constructed via [`StreamingResponse::new`] or [`StreamingResponse::sse`].
pub struct StreamingResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) stream:
        Pin<Box<dyn Stream<Item = Result<Bytes, std::convert::Infallible>> + Send + Sync>>,
}

impl StreamingResponse {
    /// Create a new generic streaming response.
    pub fn new<S>(status: StatusCode, stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, std::convert::Infallible>> + Send + Sync + 'static,
    {
        Self {
            status,
            headers: HeaderMap::new(),
            stream: Box::pin(stream),
        }
    }

    /// Create a Server-Sent Events (SSE) streaming response from a stream of strings.
    ///
    /// Sets default SSE headers:
    /// - `Content-Type: text/event-stream`
    /// - `Cache-Control: no-cache`
    /// - `Connection: keep-alive`
    ///
    /// Formats each stream item as an SSE data frame (`data: {item}\n\n`).
    pub fn sse<S>(stream: S) -> Self
    where
        S: Stream<Item = String> + Send + Sync + 'static,
    {
        let sse_stream = stream.map(|item| {
            let chunk = format!("data: {item}\n\n");
            Ok::<Bytes, std::convert::Infallible>(Bytes::from(chunk))
        });
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));

        Self {
            status: StatusCode::OK,
            headers,
            stream: Box::pin(sse_stream),
        }
    }

    /// Add a response header (builder pattern).
    ///
    /// # Panics
    /// Panics if `name` or `value` contain invalid characters for HTTP headers.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).expect("header name must be valid HTTP token");
        let header_value = HeaderValue::from_str(value).expect("header value must be valid ASCII");
        self.headers.insert(header_name, header_value);
        self
    }

    /// The HTTP status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Convert this streaming response into a hyper [`Response`](hyper::Response) backed by
    /// a [`BoxBody`].
    pub fn into_hyper(self) -> hyper::Response<BoxBody<Bytes, std::convert::Infallible>> {
        let mapped_stream = self.stream.map(|res| res.map(hyper::body::Frame::data));
        let body = BodyExt::boxed(http_body_util::StreamBody::new(mapped_stream));
        let mut res = hyper::Response::new(body);
        *res.status_mut() = self.status;
        *res.headers_mut() = self.headers;
        res
    }
}
