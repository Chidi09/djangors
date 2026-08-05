# Sessions and CSRF Protection

Djangors provides a client-side, signed-cookie session engine and double-submit CSRF (Cross-Site Request Forgery) protection, mirroring Django's design patterns.

---

## Signed-Cookie Sessions

Djangors stores session data directly in the client's browser using signed cookies. 

### How it Works
1. **Serialization**: Session key-value data is serialized to JSON.
2. **Encoding**: The JSON payload is Base64-encoded.
3. **Signing**: Djangors signs the payload with HMAC-SHA256 using the application's secret key (`SECRET_KEY`).
4. **Storage**: The combined payload and signature are sent to the client as a cookie (defaults to `"djangors_sessionid"`).

At the start of a request, the `SessionLayer` middleware extracts the cookie, verifies the HMAC signature to ensure it hasn't been tampered with, deserializes the JSON data, and inserts a request-scoped `Session` handle into the request's extension map.

### Code Example

```rust,illustrative
// Accessing the Session handle in a view
let session = req.session();

// Write value to session
session.set("user_id", 42i64);
session.set("theme", "dark".to_string());

// Read value from session
if let Some(user_id) = session.get::<i64>("user_id") {
    println!("Logged in user: {user_id}");
}

// Remove a specific key
session.remove("theme");
```

### Security Flags
Cookie configurations are managed by `SignedCookieStore` with safe defaults:
* **`HttpOnly`**: Set to `true` by default. This prevents client-side scripts from reading the cookie, mitigating Cross-Site Scripting (XSS) session theft.
* **`Secure`**: Set to `true` if your application is configured to require HTTPS.
* **`SameSite`**: Set to `Lax` to balance convenience with cross-site request security.
* **`max_age`**: Default cookie expiration duration is 2 weeks.

### Session Rotation (`cycle_key`)
To prevent session fixation attacks (where an attacker provides a known session ID to a victim and waits for them to log in), call `session.cycle_key()` upon user login or logout:

```rust,illustrative
// Authenticate user
session.cycle_key(); // Generates a new internal session identity
session.set("user_id", new_user_id);
```

---

## Programmatic Session Access

### `Session::is_empty()`

Returns `true` when the session holds no application data (only the internal `_session_key`). Useful
for "do I need to write a `Set-Cookie` at all?" checks:

```rust,illustrative
let session = Session::new_empty();
assert!(session.is_empty());

session.set("theme", "dark".to_string());
assert!(!session.is_empty());
```

### Manual encode/decode with `SignedCookieStore`

`SignedCookieStore` encodes and decodes sessions independently of the middleware — handy for custom
middleware or code that must set the cookie header itself instead of delegating to `SessionLayer`.

| Method | Signature | Notes |
| --- | --- | --- |
| `store.encode(&session)` | `fn(&self, &Session) -> String` | Serializes, base64-encodes, signs (HMAC-SHA256), returns `"<b64 payload>.<b64 expiry>.<b64 hmac>"` |
| `store.decode(cookie_value)` | `fn(&self, &str) -> Option<Session>` | Verifies signature + expiry, then decodes; `None` for missing/malformed/tampered/expired cookies |

```rust,illustrative
use djangors_sessions::{Session, SignedCookieStore};

let store = SignedCookieStore::new(b"my-32-byte-minimum-secret-key!!");
let session = Session::new_empty();
session.set("user_id", 42i64);

let cookie_value = store.encode(&session);       // "<b64 payload>.<b64 expiry>.<b64 hmac>"
let restored = store.decode(&cookie_value);       // Some(session) — data verified intact
assert_eq!(restored.unwrap().get::<i64>("user_id"), Some(42));
```

### `SessionService`: the layer's service type

`SessionLayer` is a
[`tower::Layer`](https://docs.rs/tower/latest/tower/trait.Layer.html); wrapping an inner service
yields a `SessionService<S>` (`djangors_sessions::SessionService`) as the concrete service type.
This is the Tower middleware that does the per-request work: read the `Cookie` header, decode +
verify it via the store, insert the `Session` into the request's `Extensions`, then write
`Set-Cookie` back on the response when the session was modified or newly created.

```rust,illustrative
use djangors_sessions::{SessionService, SessionLayer, SignedCookieStore};
use tower::Layer;

fn stack(inner: djangors_core::router::RouterService) -> SessionService<djangors_core::router::RouterService> {
    SessionLayer::new(SignedCookieStore::new(b"my-32-byte-minimum-secret-key!!"))
        .layer(inner)
}
```

---

## CSRF Protection

Djangors uses a double-submit cookie scheme to protect against Cross-Site Request Forgery (CSRF).

### The Mechanism
1. The `CsrfLayer` middleware reads the request cookie named `csrftoken`. If not present, it generates a cryptographically secure random token and sets it as the `csrftoken` cookie.
2. For unsafe methods (`POST`, `PUT`, `PATCH`, `DELETE`), Djangors requires the client to submit the same CSRF token value in the request.
3. The server performs a constant-time comparison (using `constant_time_eq`) between the cookie value and the submitted request value.

### Submitting the Token
Clients can submit the CSRF token in one of two ways, both of which are accepted by Djangors:

#### 1. Plain HTML Forms (`csrfmiddlewaretoken`)
When rendering a form, include the token value as a hidden input parameter named `csrfmiddlewaretoken`.
```html
<form method="post" action="/submit">
  <!-- Renders a hidden field with the CSRF token -->
  <input type="hidden" name="csrfmiddlewaretoken" value="{{ csrf_token }}">
  <button type="submit">Submit</button>
</form>
```
During a form post (`application/x-www-form-urlencoded`), the Djangors router parses the form body, extracts `csrfmiddlewaretoken`, and compares it against the expected token.

#### 2. Javascript Clients (`X-CSRFToken` Header)
For AJAX/fetch requests (such as SPA clients sending JSON bodies), the `csrftoken` cookie is intentionally **not** marked `HttpOnly` so that client-side JavaScript can read it. 

JS clients must extract the value from the cookie and send it in the **`X-CSRFToken`** header with their request.
```javascript
// Example fetch request
fetch('/api/data', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    'X-CSRFToken': getCookie('csrftoken')
  },
  body: JSON.stringify({ key: 'value' })
});
```
The CSRF middleware intercepts the request, reads the header, and validates it.
