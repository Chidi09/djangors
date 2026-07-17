use std::collections::HashMap;

use bytes::Bytes;
use hyper::http::{HeaderMap, HeaderValue, Method, Uri};
use percent_encoding::percent_decode_str;

use crate::state::AppState;

/// An HTTP request with a fully buffered body.
///
/// Constructed by the router from an incoming hyper request before being
/// passed to handlers. All body bytes are read eagerly at construction time.
#[derive(Debug, Clone)]
pub struct Request {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    query_params: HashMap<String, String>,
    state: AppState,
}

impl Request {
    /// Create a new `Request` from its component parts.
    ///
    /// The query string (if any) is parsed and URL-decoded at construction
    /// time and will be accessible via `.query(name)`.
    pub fn new(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Self {
        let query_params = Self::parse_query(uri.query().unwrap_or(""));
        Request {
            method,
            uri,
            headers,
            body,
            query_params,
            state: AppState::default(),
        }
    }

    /// Retrieve a piece of shared state attached to the app via
    /// [`Router::with_state`], if any was registered for type `T`.
    pub fn state<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.state.get::<T>()
    }

    /// Attach the app-wide state to this request.
    pub fn with_state(mut self, state: AppState) -> Self {
        self.state = state;
        self
    }

    fn parse_query(query: &str) -> HashMap<String, String> {
        let mut params = HashMap::new();
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let mut parts = pair.splitn(2, '=');
            let key_raw = parts.next().unwrap_or("");
            let val_raw = parts.next().unwrap_or("");
            let key = percent_decode_str(key_raw)
                .decode_utf8()
                .map(|k| k.into_owned())
                .unwrap_or_else(|_| key_raw.to_string());
            let val = percent_decode_str(val_raw)
                .decode_utf8()
                .map(|v| v.into_owned())
                .unwrap_or_else(|_| val_raw.to_string());
            if !key.is_empty() {
                params.insert(key, val);
            }
        }
        params
    }

    /// The HTTP method (GET, POST, etc.).
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// The URI path component (e.g. `"/hello/world"`).
    pub fn path(&self) -> &str {
        self.uri.path()
    }

    /// Access a request header by name.
    ///
    /// Returns `None` if the header is not present.
    pub fn header(&self, name: &str) -> Option<&HeaderValue> {
        self.headers.get(name)
    }

    /// Get a decoded query-string parameter by name.
    ///
    /// Returns `None` if the parameter is not present.
    pub fn query(&self, name: &str) -> Option<&str> {
        self.query_params.get(name).map(|s| s.as_str())
    }

    /// Get the raw, undecoded query string from the request URI (e.g. `"q=rust&page=2"`).
    ///
    /// Returns `None` if no query string is present.
    pub fn raw_query(&self) -> Option<&str> {
        self.uri.query()
    }

    /// Access all request headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Return the buffered request body bytes.
    ///
    /// This is a trivially async accessor; the body was fully read during
    /// request construction.
    pub async fn body_bytes(&self) -> &[u8] {
        &self.body
    }

    /// Consume the request and return its component parts.
    pub fn into_parts(self) -> (Method, Uri, HeaderMap, Bytes) {
        (self.method, self.uri, self.headers, self.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::http::Uri;

    #[tokio::test]
    async fn body_bytes() {
        let body = Bytes::from("hello");
        let req = Request::new(Method::GET, Uri::from_static("/"), HeaderMap::new(), body);
        assert_eq!(req.body_bytes().await, b"hello");
    }

    #[tokio::test]
    async fn method_and_path() {
        let req = Request::new(
            Method::POST,
            Uri::from_static("/foo/bar"),
            HeaderMap::new(),
            Bytes::new(),
        );
        assert_eq!(req.method(), &Method::POST);
        assert_eq!(req.path(), "/foo/bar");
    }

    #[tokio::test]
    async fn header_access() {
        let mut headers = HeaderMap::new();
        headers.insert("x-custom", HeaderValue::from_static("val"));
        let req = Request::new(Method::GET, Uri::from_static("/"), headers, Bytes::new());
        assert_eq!(req.header("x-custom").unwrap(), "val");
        assert!(req.header("missing").is_none());
    }

    #[tokio::test]
    async fn query_params() {
        let req = Request::new(
            Method::GET,
            Uri::from_static("/search?q=rust&page=2"),
            HeaderMap::new(),
            Bytes::new(),
        );
        assert_eq!(req.query("q"), Some("rust"));
        assert_eq!(req.query("page"), Some("2"));
        assert_eq!(req.query("missing"), None);
    }

    #[tokio::test]
    async fn query_params_url_encoded() {
        let req = Request::new(
            Method::GET,
            Uri::from_static("/search?name=hello%20world"),
            HeaderMap::new(),
            Bytes::new(),
        );
        assert_eq!(req.query("name"), Some("hello world"));
    }
}
