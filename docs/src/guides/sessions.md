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
