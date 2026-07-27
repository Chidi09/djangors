#![deny(missing_docs)]
//! Session engines for the Djangors web framework.
//!
//! Provides signed-cookie session management.

use base64::Engine;
use hmac::{Hmac, Mac};
use hyper::header::{HeaderMap, HeaderValue, COOKIE, SET_COOKIE};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::layer::Layer;
use tower::Service;

type HmacSha256 = Hmac<Sha256>;

/// A per-request session handle. Cheaply `Clone` (wraps an `Arc<Mutex<..>>`)
/// so the same handle can be inserted into the request's extensions (for
/// handlers to read/mutate) and retained by SessionLayer (to serialize
/// after the handler returns) without any response-side round-trip.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Mutex<SessionInner>>,
}

struct SessionInner {
    data: HashMap<String, Value>,
    modified: bool,
    /// Set by `cycle_key`; on the next encode, forces a fresh session
    /// identity (not just re-signing the same data).
    cycled: bool,
    new: bool,
}

impl Session {
    /// Creates a new empty session with a unique session key.
    pub fn new_empty() -> Self {
        let mut data = HashMap::new();
        data.insert(
            "_session_key".to_string(),
            Value::String(generate_session_key()),
        );
        Self {
            inner: Arc::new(Mutex::new(SessionInner {
                data,
                modified: false,
                cycled: false,
                new: true,
            })),
        }
    }

    /// Gets a deserialized value for the specified key from the session.
    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let inner = self.inner.lock().ok()?;
        let val = inner.data.get(key)?;
        serde_json::from_value(val.clone()).ok()
    }

    /// Sets a serializable value for the specified key in the session and marks it as modified.
    pub fn set<T: serde::Serialize>(&self, key: &str, value: T) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Ok(json_val) = serde_json::to_value(value) {
                inner.data.insert(key.to_string(), json_val);
                inner.modified = true;
            }
        }
    }

    /// Removes a key and its value from the session and marks it as modified.
    pub fn remove(&self, key: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.data.remove(key).is_some() {
                inner.modified = true;
            }
        }
    }

    /// Clears all non-internal key-value pairs from the session and marks it as modified.
    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.data.clear();
            inner.data.insert(
                "_session_key".to_string(),
                Value::String(generate_session_key()),
            );
            inner.modified = true;
        }
    }

    /// Returns `true` if the session contains no application data keys.
    pub fn is_empty(&self) -> bool {
        if let Ok(inner) = self.inner.lock() {
            inner.data.keys().all(|k| k == "_session_key")
        } else {
            true
        }
    }

    /// Rotate session identity in place (auth calls this on login/logout).
    pub fn cycle_key(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.cycled = true;
            inner.modified = true;
        }
    }
}

fn generate_session_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut s = String::with_capacity(32);
    for b in bytes {
        use std::fmt::Write;
        write!(&mut s, "{:02x}", b).unwrap();
    }
    s
}

/// A signed cookie store for encoding and decoding session data.
pub struct SignedCookieStore {
    key: Vec<u8>,        // from settings.SECRET_KEY, NOT hardcoded, NOT optional
    cookie_name: String, // default "djangors_sessionid"
    max_age: Duration,   // default matches Django's 2-week default
    secure: bool,
}

impl SignedCookieStore {
    /// Creates a new `SignedCookieStore` using the provided secret key.
    pub fn new(secret_key: &[u8]) -> Self {
        Self {
            key: secret_key.to_vec(),
            cookie_name: "djangors_sessionid".to_string(),
            max_age: Duration::from_secs(14 * 24 * 60 * 60), // 2 weeks
            secure: false,
        }
    }

    /// Configures a custom cookie name.
    pub fn with_cookie_name(mut self, name: String) -> Self {
        self.cookie_name = name;
        self
    }

    /// Configures the cookie max age duration.
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// Configures whether the cookie should set the `Secure` flag.
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    /// Encodes and signs the session state into a cookie value string.
    pub fn encode(&self, session: &Session) -> String {
        let mut inner = session.inner.lock().unwrap();
        if inner.cycled {
            inner.data.insert(
                "_session_key".to_string(),
                Value::String(generate_session_key()),
            );
            inner.cycled = false;
        }
        let json_str = serde_json::to_string(&inner.data).unwrap();
        let b64_json = base64::engine::general_purpose::STANDARD.encode(json_str.as_bytes());

        let expires_at_unix = (std::time::SystemTime::now() + self.max_age)
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        let b64_expiry = base64::engine::general_purpose::STANDARD
            .encode(expires_at_unix.to_string().as_bytes());

        let msg = format!("{}.{}", b64_json, b64_expiry);

        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts any key length");
        mac.update(msg.as_bytes());
        let mac_result = mac.finalize().into_bytes();
        let b64_mac = base64::engine::general_purpose::STANDARD.encode(mac_result);

        format!("{}.{}.{}", b64_json, b64_expiry, b64_mac)
    }

    /// Returns `None` for missing/malformed/tampered/expired cookies —
    /// never an `Err` that would need special-casing by the layer.
    pub fn decode(&self, cookie_value: &str) -> Option<Session> {
        let parts: Vec<&str> = cookie_value.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let b64_json = parts[0];
        let b64_expiry = parts[1];
        let b64_mac = parts[2];

        // 1. Verify HMAC
        let msg = format!("{}.{}", b64_json, b64_expiry);
        let mac_bytes = base64::engine::general_purpose::STANDARD
            .decode(b64_mac)
            .ok()?;
        let mut mac = HmacSha256::new_from_slice(&self.key).ok()?;
        mac.update(msg.as_bytes());
        mac.verify_slice(&mac_bytes).ok()?;

        // 2. Verify Expiry
        let expiry_bytes = base64::engine::general_purpose::STANDARD
            .decode(b64_expiry)
            .ok()?;
        let expiry_str = std::str::from_utf8(&expiry_bytes).ok()?;
        let expiry_secs: u64 = expiry_str.parse().ok()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        if expiry_secs <= now {
            return None;
        }

        // 3. Decode JSON
        let json_bytes = base64::engine::general_purpose::STANDARD
            .decode(b64_json)
            .ok()?;
        let data: HashMap<String, Value> = serde_json::from_slice(&json_bytes).ok()?;

        Some(Session {
            inner: Arc::new(Mutex::new(SessionInner {
                data,
                modified: false,
                cycled: false,
                new: false,
            })),
        })
    }
}

fn extract_session_cookie(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    for cookie_header in headers.get_all(COOKIE) {
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

/// The tower Layer/Service that does the actual per-request work: parses
/// the `Cookie` request header, decodes+verifies the session cookie via
/// the store, inserts the `Session` into the hyper request's `Extensions`
/// before calling the inner service, and (only if `session.modified` or
/// newly created/cleared) sets `Set-Cookie` on the response afterward by
/// reading the same `Arc`-shared handle it kept.
#[derive(Clone)]
pub struct SessionLayer {
    store: Arc<SignedCookieStore>,
}

impl SessionLayer {
    /// Creates a new `SessionLayer` wrapping the specified store.
    pub fn new(store: SignedCookieStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }
}

impl<S> Layer<S> for SessionLayer {
    type Service = SessionService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SessionService {
            inner,
            store: self.store.clone(),
        }
    }
}

/// Tower service middleware managing per-request session lifecycle.
#[derive(Clone)]
pub struct SessionService<S> {
    inner: S,
    store: Arc<SignedCookieStore>,
}

impl<S, ReqBody, ResBody> Service<hyper::Request<ReqBody>> for SessionService<S>
where
    S: Service<hyper::Request<ReqBody>, Response = hyper::Response<ResBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        let store = self.store.clone();

        let session =
            if let Some(cookie_val) = extract_session_cookie(req.headers(), &store.cookie_name) {
                store.decode(&cookie_val).unwrap_or_else(Session::new_empty)
            } else {
                Session::new_empty()
            };

        req.extensions_mut().insert(session.clone());

        Box::pin(async move {
            let mut resp = inner.call(req).await?;

            let should_save = {
                if let Ok(inner) = session.inner.lock() {
                    inner.modified || inner.new
                } else {
                    false
                }
            };

            if should_save {
                let cookie_value = store.encode(&session);
                let max_age_secs = store.max_age.as_secs();
                let mut set_cookie_val = format!(
                    "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
                    store.cookie_name, cookie_value, max_age_secs
                );
                if store.secure {
                    set_cookie_val.push_str("; Secure");
                }
                if let Ok(hdr_val) = HeaderValue::from_str(&set_cookie_val) {
                    resp.headers_mut().append(SET_COOKIE, hdr_val);
                }
            }

            Ok(resp)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use djangors_core::{DjangorsError, Request, Response, Router};
    use http_body_util::Full;
    use hyper::StatusCode;
    use tower::ServiceExt;

    #[test]
    fn test_round_trip() {
        let secret = b"my-very-secret-key-that-is-long-enough";
        let store = SignedCookieStore::new(secret);
        let session = Session::new_empty();
        session.set("key_one", "hello_world".to_string());
        session.set("key_two", 42);

        let cookie_val = store.encode(&session);
        let decoded = store
            .decode(&cookie_val)
            .expect("Should decode successfully");

        assert_eq!(
            decoded.get::<String>("key_one"),
            Some("hello_world".to_string())
        );
        assert_eq!(decoded.get::<i32>("key_two"), Some(42));
    }

    #[test]
    fn test_tamper_rejection() {
        let secret = b"my-very-secret-key-that-is-long-enough";
        let store = SignedCookieStore::new(secret);
        let session = Session::new_empty();
        session.set("secured_val", "confidential".to_string());

        let cookie_val = store.encode(&session);

        // Attempting to decode the valid cookie works
        let decoded_ok = store.decode(&cookie_val).expect("Should decode");
        assert_eq!(
            decoded_ok.get::<String>("secured_val"),
            Some("confidential".to_string())
        );

        // Tamper with the cookie string. A valid cookie is base64.base64.base64.
        // We will flip a byte in the MAC signature part (the last segment).
        let parts: Vec<&str> = cookie_val.split('.').collect();
        assert_eq!(parts.len(), 3);

        let tampered_mac = {
            let mut mac_bytes = base64::engine::general_purpose::STANDARD
                .decode(parts[2])
                .unwrap();
            // Flip the last byte to corrupt the MAC without making base64 malformed
            if let Some(last) = mac_bytes.last_mut() {
                *last ^= 0xFF;
            }
            base64::engine::general_purpose::STANDARD.encode(&mac_bytes)
        };

        let tampered_cookie = format!("{}.{}.{}", parts[0], parts[1], tampered_mac);
        let decoded_tampered = store.decode(&tampered_cookie);
        assert!(
            decoded_tampered.is_none(),
            "Tampered session must return None"
        );
    }

    #[test]
    fn test_expiry() {
        let secret = b"my-very-secret-key-that-is-long-enough";
        // Create store with zero max-age
        let store = SignedCookieStore::new(secret).with_max_age(Duration::ZERO);
        let session = Session::new_empty();
        session.set("test", "value".to_string());

        let cookie_val = store.encode(&session);

        // Even though the signature is valid, decoding should fail immediately due to expiration check
        let decoded = store.decode(&cookie_val);
        assert!(decoded.is_none(), "Expired session must fail to decode");
    }

    #[test]
    fn test_cycle_key() {
        let secret = b"my-very-secret-key-that-is-long-enough";
        let store = SignedCookieStore::new(secret);
        let session = Session::new_empty();
        session.set("foo", "bar".to_string());

        let cookie_val_1 = store.encode(&session);

        // Rotate session identity
        session.cycle_key();
        let cookie_val_2 = store.encode(&session);

        // Prove pre-rotation and post-rotation cookie strings are different
        assert_ne!(cookie_val_1, cookie_val_2);

        // Decode both and show that they both can get the value, but they have different session keys
        let decoded_1 = store.decode(&cookie_val_1).unwrap();
        let decoded_2 = store.decode(&cookie_val_2).unwrap();

        assert_eq!(decoded_1.get::<String>("foo"), Some("bar".to_string()));
        assert_eq!(decoded_2.get::<String>("foo"), Some("bar".to_string()));

        let sk_1 = decoded_1.get::<String>("_session_key").unwrap();
        let sk_2 = decoded_2.get::<String>("_session_key").unwrap();
        assert_ne!(sk_1, sk_2, "Session keys must be rotated/different");
    }

    #[test]
    fn test_no_session_cookie_empty() {
        let secret = b"my-very-secret-key-that-is-long-enough";
        let store = SignedCookieStore::new(secret);
        let decoded = store.decode("");
        assert!(decoded.is_none());
    }

    // End-to-end tower/router integration tests
    async fn set_handler(
        req: Request,
        _: djangors_core::PathParams,
    ) -> Result<Response, DjangorsError> {
        let session = req
            .ext::<Session>()
            .expect("Session extension must be present");
        let count: i32 = session.get("count").unwrap_or(0);
        session.set("count", count + 1);
        Ok(Response::text(StatusCode::OK, &(count + 1).to_string()))
    }

    async fn get_handler(
        req: Request,
        _: djangors_core::PathParams,
    ) -> Result<Response, DjangorsError> {
        let session = req
            .ext::<Session>()
            .expect("Session extension must be present");
        let count: i32 = session.get("count").unwrap_or(0);
        Ok(Response::text(StatusCode::OK, &count.to_string()))
    }

    #[tokio::test]
    async fn test_end_to_end_router_session() {
        use tower::service_fn;
        use tower::ServiceBuilder;

        let router = Router::new()
            .post("/set", set_handler)
            .get("/get", get_handler);

        let secret = b"super-secret-key-for-testing-purposes-only";
        let store = SignedCookieStore::new(secret);
        let layer = SessionLayer::new(store);

        let svc_fn = service_fn(move |req: hyper::Request<Full<Bytes>>| {
            let router = router.clone();
            async move { Ok::<_, std::convert::Infallible>(router.dispatch(req).await) }
        });

        let mut svc = ServiceBuilder::new().layer(layer).service(svc_fn);

        // 1. Request with no session cookie (empty count) -> should return 1
        let req1 = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri("/set")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp1 = svc.ready().await.unwrap().call(req1).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        // We must have received a Set-Cookie header
        let set_cookie_header = resp1.headers().get(SET_COOKIE).expect("Should set cookie");
        let set_cookie_str = set_cookie_header.to_str().unwrap().to_string();

        // Read response body
        use http_body_util::BodyExt;
        let body1 = resp1.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body1[..], b"1");

        // Extract the value from the Set-Cookie string (e.g. djangors_sessionid=VALUE; Path=...)
        let cookie_value = set_cookie_str
            .split(';')
            .next()
            .unwrap()
            .split_once('=')
            .unwrap()
            .1
            .to_string();

        // 2. Request 2: carrying the cookie to /get
        let req2 = hyper::Request::builder()
            .method(hyper::Method::GET)
            .uri("/get")
            .header(COOKIE, format!("djangors_sessionid={}", cookie_value))
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp2 = svc.ready().await.unwrap().call(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let body2 = resp2.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body2[..], b"1");

        // 3. Request 3: calling /set again carrying the cookie -> should return 2
        let req3 = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri("/set")
            .header(COOKIE, format!("djangors_sessionid={}", cookie_value))
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp3 = svc.ready().await.unwrap().call(req3).await.unwrap();
        assert_eq!(resp3.status(), StatusCode::OK);
        let body3 = resp3.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body3[..], b"2");
    }

    #[tokio::test]
    async fn test_no_session_cookie_handler_access() {
        use tower::service_fn;
        use tower::ServiceBuilder;

        let router = Router::new().get("/get", get_handler);
        let secret = b"super-secret-key-for-testing-purposes-only";
        let store = SignedCookieStore::new(secret);
        let layer = SessionLayer::new(store);

        let svc_fn = service_fn(move |req: hyper::Request<Full<Bytes>>| {
            let router = router.clone();
            async move { Ok::<_, std::convert::Infallible>(router.dispatch(req).await) }
        });

        let mut svc = ServiceBuilder::new().layer(layer).service(svc_fn);

        let req = hyper::Request::builder()
            .method(hyper::Method::GET)
            .uri("/get")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        use http_body_util::BodyExt;
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"0");
    }

    #[tokio::test]
    async fn test_session_cookie_secure() {
        use tower::service_fn;
        use tower::ServiceBuilder;

        let router = Router::new().post("/set", set_handler);
        let secret = b"super-secret-key-for-testing-purposes-only";

        // 1. Default (secure not called or false)
        let store_default = SignedCookieStore::new(secret);
        let layer_default = SessionLayer::new(store_default);
        let svc_fn1 = service_fn({
            let router = router.clone();
            move |req| {
                let router = router.clone();
                async move { Ok::<_, std::convert::Infallible>(router.dispatch(req).await) }
            }
        });
        let mut svc_default = ServiceBuilder::new().layer(layer_default).service(svc_fn1);

        let req1 = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri("/set")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp1 = svc_default.ready().await.unwrap().call(req1).await.unwrap();
        let set_cookie_default = resp1.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
        assert!(!set_cookie_default.contains("; Secure"));

        // 2. with_secure(true)
        let store_secure = SignedCookieStore::new(secret).with_secure(true);
        let layer_secure = SessionLayer::new(store_secure);
        let svc_fn2 = service_fn({
            let router = router.clone();
            move |req| {
                let router = router.clone();
                async move { Ok::<_, std::convert::Infallible>(router.dispatch(req).await) }
            }
        });
        let mut svc_secure = ServiceBuilder::new().layer(layer_secure).service(svc_fn2);

        let req2 = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri("/set")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let resp2 = svc_secure.ready().await.unwrap().call(req2).await.unwrap();
        let set_cookie_secure = resp2.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
        assert!(set_cookie_secure.contains("; Secure"));
    }
}
