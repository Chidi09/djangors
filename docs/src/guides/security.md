# Security

Djangors includes built-in security middleware covering CSRF protection, security response headers, host header validation, and HMAC-signed session cookie storage.

## CSRF Protection (`CsrfLayer`)

`CsrfLayer` (`djangors_core::middleware::csrf_layer()`) implements CSRF protection using a double-submit cookie scheme with header and form-body validation fallbacks.

```rust,compile
# fn main() {
# let router_service = djangors_core::router::RouterService::new(djangors_core::Router::new(), false);
use djangors_core::middleware::csrf_layer;
use tower::ServiceBuilder;

let service = ServiceBuilder::new()
    .layer(csrf_layer())
    .service(router_service);
# }
```

### Mechanism & Verification Flow
1. **Cookie Generation**: On safe requests (`GET`, `HEAD`, `OPTIONS`), if no `csrftoken` cookie exists, a cryptographically secure random 32-byte hex token is generated and returned in a `csrftoken` cookie (`SameSite=Lax; Path=/; Max-Age=31536000`). Note: The cookie is intentionally *not* `HttpOnly` so client-side JavaScript can read it to populate request headers.
2. **Unsafe Method Validation**: For unsafe HTTP methods (`POST`, `PUT`, `PATCH`, `DELETE`), the token in the `X-CSRFToken` request header is compared against the `csrftoken` cookie value using constant-time comparison (`constant_time_eq`).
3. **Form Fallback**: If the header is missing or mismatched, `CsrfLayer` sets `CsrfPendingFormCheck` on the request extensions. Form submission handlers verify the `csrfmiddlewaretoken` form body field against the pending token, returning `403 Forbidden` if missing or invalid.

**Production checklist**: `csrf_layer()` defaults to `Secure` **off** on the `csrftoken` cookie, the
same way `SignedCookieStore::new()` defaults `Secure` off (see below) — this keeps local HTTP
development working out of the box. In production, enable it the same way the session store does:
`csrf_layer().with_secure(!settings.debug)`. Both `examples/school` and `examples/polls` wire this
correctly; if you're copying from an older reference, check that your own `main.rs` does too — an
internal security review (2026-07-27) found neither example app had actually done this before,
despite the session-cookie guidance below already documenting the correct pattern.

---

## Security Headers (`SecurityHeadersLayer`)

`SecurityHeadersLayer` (`djangors_core::middleware::security_headers_layer()`) sets standard security response headers mimicking Django's `SecurityMiddleware`:

```rust,compile
# fn main() {
# let router_service = djangors_core::router::RouterService::new(djangors_core::Router::new(), false);
use tower::ServiceBuilder;
use djangors_core::middleware::security_headers_layer;

let service = ServiceBuilder::new()
    .layer(security_headers_layer())
    .service(router_service);
# }
```

### Headers Set
- **`X-Content-Type-Options: nosniff`**: Prevents browsers from MIME-sniffing responses away from the declared content type.
- **`X-Frame-Options: DENY`**: Protects against clickjacking by preventing the app from being embedded in `<frame>`, `<iframe>`, or `<object>` elements.
- **`Referrer-Policy: same-origin`**: Restricts referrer information sent in HTTP headers to same-origin requests.

---

## Strict Transport Security (`HstsLayer`)

Enforces HTTPS via HTTP Strict Transport Security (HSTS):

```rust,compile
# fn main() {
use djangors_core::middleware::hsts_layer;

// Sets Strict-Transport-Security: max-age=31536000; includeSubDomains
let hsts = hsts_layer(31536000).with_include_subdomains(true);
# }
```

---

## Host Header Validation (`HostValidationLayer`)

`HostValidationLayer` validates incoming HTTP `Host` headers against the project's `ALLOWED_HOSTS` setting:

```rust,compile
# fn main() {
use djangors_core::middleware::HostValidationLayer;

let layer = HostValidationLayer::new(vec!["example.com".to_string(), "api.example.com".to_string()]);
# }
```

- Strip trailing port numbers before matching.
- Requests with disallowed Host headers are immediately rejected with `400 Bad Request: Disallowed Host` before reaching application handlers.

---

## Signed Cookie Session Store (`SignedCookieStore`)

`SignedCookieStore` (`djangors_sessions::SignedCookieStore`) stores session data in client-side cookies signed with HMAC-SHA256 using `settings.SECRET_KEY`:

```rust,compile
# fn main() {
# let secret_key_bytes = b"01234567890123456789012345678901";
use djangors_sessions::{SessionLayer, SignedCookieStore};

let store = SignedCookieStore::new(secret_key_bytes)
    .with_cookie_name("djangors_sessionid".into())
    .with_secure(true); // Enforce Secure attribute in production

let session_layer = SessionLayer::new(store);
# }
```

### Security Features
- **HMAC Signature**: Cookies are encoded as `b64_json.b64_expiry.b64_mac`. Any tampering with payload or expiration invalidates the MAC signature, causing `decode()` to return `None` (clearing session state).
- **`HttpOnly` & `SameSite=Lax`**: Session cookies are marked `HttpOnly` (inaccessible to client JavaScript) and `SameSite=Lax`.
- **Session Fixation Prevention (`cycle_key`)**: Calling `session.cycle_key()` during authentication (`login()`) rotates the session key string, preventing session fixation attacks.
