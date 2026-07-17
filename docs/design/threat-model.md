# Threat model — Djangors (as of Phase 4 part 4e)

**Status:** living document, revisited every phase that adds new attack surface (next expected
update: Phase 8, when token/JWT auth and WebSockets add trust boundaries that don't exist yet).
Everything below describes **real, already-committed code** — file/function references are exact,
not aspirational. Where a mitigation is intentionally partial, that's stated explicitly rather than
rounded up to "done."

## Trust boundaries

As of Phase 4, Djangors only has one real untrusted-input surface: **the browser**, talking to the
server over cookies, headers, query strings, and (JSON/form-encoded, not yet multipart) request
bodies. There is no API/token auth yet (`AuthBackend`/`Auth<U>` only support session-cookie-based
auth — Phase 8 adds `djangors-rest` token/JWT auth, a second trust boundary that doesn't exist
today). There is no multi-tenant/cross-app boundary yet (no `djangors-contrib-guardian`
object-level permissions, no multi-database routing trust distinction). The database itself is
trusted (no untrusted SQL execution path — all queries go through the ORM's parameterized query
builder; no raw-SQL escape hatch has been built yet per Phase 2's `PLAN.md` line, so there's
currently no first-party SQL-injection surface to model beyond "the ORM's query builder must keep
parameterizing correctly," which is exercised by the ORM's own test suite, not this doc).

## Per-subsystem threat/mitigation summary

### Sessions (`crates/djangors-sessions/src/lib.rs`)

- **Threat:** forged/tampered session cookie granting another user's session. **Mitigation:**
  `SignedCookieStore::decode` HMAC-SHA256-verifies the cookie value (`mac.verify_slice`, which is
  constant-time internally via the `hmac`/`subtle` crate machinery) before trusting any of its
  contents; malformed/tampered/expired input returns `None` (never panics, never partially
  trusts), see `decode`'s own doc comment and `test_tamper_rejection`.
- **Threat:** session replay after expiry. **Mitigation:** expiry is embedded in and verified from
  the *signed* payload itself (`decode`'s step 2), not trusted from the cookie's own `Max-Age`
  (which the client controls) — a client can't extend its own session's life by re-sending an old
  cookie with a rewritten `Max-Age`.
- **Threat:** session fixation (attacker plants a session ID before victim logs in, then reuses it
  post-login). **Mitigation:** `djangors_auth::login()` calls `session.cycle_key()` before setting
  `_auth_user_id` — see `crates/djangors-auth/src/lib.rs`. Verified by
  `test_login_session_mechanics`.
- **Threat:** session cookie theft via XSS. **Mitigation:** the session cookie is set `HttpOnly`
  unconditionally (`SessionService::call`, the hardcoded `"; HttpOnly;"` in the `Set-Cookie`
  string) — client-side JS cannot read it. `SameSite=Lax` also limits cross-site leakage via
  navigation.
- **Threat:** session cookie theft via network interception. **Mitigation:** `SignedCookieStore`
  has a `with_secure(bool)` builder — **not on by default**, an app must opt in (or Django-style
  `settings`-driven default-on wiring, which doesn't exist yet since there's no settings-to-layer
  wiring built for this in an example app). **Known gap**, tracked in the checklist doc.
- **Non-threat (already ruled out):** the signing key is not hardcoded — `SignedCookieStore::new`
  takes `secret_key: &[u8]` as a required parameter, sourced from the app's own secret
  (`settings.SECRET_KEY`-equivalent), not a constant in this crate.

### CSRF (`crates/djangors-core/src/middleware.rs`, `CsrfLayer`)

- **Threat:** cross-site request forgery on state-changing requests. **Mitigation:** double-submit
  cookie scheme — `CsrfService::call` rejects (403) any of POST/PUT/PATCH/DELETE unless the
  `X-CSRFToken` header value constant-time-equals (`constant_time_eq`) the `csrftoken` cookie
  value. An attacker's cross-site form can send the cookie automatically but cannot read it to set
  a matching header (no `HttpOnly` on this cookie is *required* for the scheme to work, but it
  also means client JS *can* read the token, which is the intended usage for AJAX requests).
- **Known, explicitly-scoped gap:** v1 only validates the header, not a `csrfmiddlewaretoken` form
  body field (see the `CRITICAL SECURITY NOTE` doc comment directly on `CsrfLayer`, and
  [[4.8-csrf]]'s own deferral note). **A classic `<form method="post">` submitted without
  JavaScript adding the header is NOT protected today.** This is the single most important gap in
  this document — any app relying on plain HTML form POSTs (not fetch/XHR) for state-changing
  requests has no CSRF protection until this is closed. Tracked, not yet scheduled.
- **Threat:** CSRF token itself stolen/guessed. **Mitigation:** `generate_csrf_token` draws 32
  bytes from `rand::thread_rng()` (CSPRNG-backed) — not enumerable/guessable.
- **Threat:** cookie sent over plaintext HTTP. **Mitigation:** same `with_secure(bool)` opt-in
  pattern as sessions, same known gap (not on by default).

### Security headers / host validation (`crates/djangors-core/src/middleware.rs`)

- **Threat:** clickjacking. **Mitigation:** `SecurityHeadersLayer` sets `X-Frame-Options: DENY`
  unconditionally.
- **Threat:** MIME-sniffing-based content-type confusion attacks. **Mitigation:**
  `X-Content-Type-Options: nosniff`, unconditional.
- **Threat:** referrer leakage to third parties. **Mitigation:** `Referrer-Policy: same-origin`,
  unconditional.
- **Threat:** protocol downgrade / SSL-stripping. **Mitigation:** `HstsLayer` (opt-in — an app
  wires it in explicitly, `hsts_layer(max_age)`, it is not bundled into
  `SecurityHeadersLayer`/applied by default) sets `Strict-Transport-Security`.
- **Threat:** Host header injection (cache poisoning, password-reset-link poisoning once that
  feature exists in part 4d). **Mitigation:** `HostValidationLayer` rejects (400) any request
  whose `Host` header (port-stripped, IPv6-bracket-aware, lowercased) isn't in an explicit
  allowlist — but **`allowed_hosts.is_empty()` means unrestricted** (`is_valid = true`
  unconditionally), i.e. this is opt-in-by-configuration, same as `ALLOWED_HOSTS = []` being
  Django's own insecure default in DEBUG mode. An app that never configures this layer has no
  protection.
- **No default Content-Security-Policy.** `PLAN.md`'s "CSP helper" bullet has not been built —
  there is no `CspLayer` yet, only the four headers above. **Known gap.**

### Authentication (`crates/djangors-auth/src/lib.rs`)

- **Threat:** password compromise via weak hashing. **Mitigation:** Argon2id via the `argon2`
  crate's own `PasswordHasher`/`PasswordVerifier` (never hand-rolled crypto), PHC-format storage
  (`hash_password`/`verify_password`).
- **Threat:** username enumeration via response-time side channel (does a login endpoint respond
  faster for nonexistent usernames, revealing which usernames exist?). **Mitigation:**
  `ModelBackend::authenticate` always calls `verify_password` — against the real hash if exactly
  one user matched, against a hardcoded `DUMMY_HASH` (same Argon2 cost parameters) otherwise — so
  the "user not found" and "wrong password" paths do comparable work. Not a formally
  timing-audited guarantee (Argon2's own memory-hard cost dominates and should swamp smaller
  timing differences elsewhere in the function, e.g. the DB lookup itself), but the dominant
  cost — password verification — is equalized across both paths, closing the main practical
  oracle. See [[4.11-auth-backend-login]]'s own note on why an automated timing assertion in the
  test suite isn't attempted (flaky by nature); the dummy-hash code path being real, executed code
  (not dead code skipped by an early return) is instead verified by reading the implementation.
- **Threat:** brute-force/credential-stuffing against the login endpoint. **Mitigation:**
  `RateLimitedBackend` (opt-in decorator around any `AuthBackend`), default 5 attempts/15 minutes
  via `default_login_throttle`. **Known gap:** single-process, in-memory only (see
  [[4.12-auth-rate-limit-signals]]) — does not coordinate across multiple app instances behind a
  load balancer; an attacker distributing requests across instances (or simply hitting an app that
  restarted) resets the counter. A distributed (cache-backed) limiter is future work, blocked on a
  `djangors-cache` crate that doesn't exist yet (Phase 7).
- **Threat:** stale/deleted/deactivated account still usable via an old session. **Mitigation:**
  `Auth<U>::from_request` re-fetches the user row from the DB on every request (not just trusting
  the session's cached user id) and checks `is_active()` — a deactivated account or deleted row
  fails extraction (`Unauthorized`) even with an otherwise-valid session cookie.
- **Threat:** XSS via signal payload data. **Mitigation:** `LoginFailed.username`'s doc comment
  explicitly flags it as attacker-controlled, warning against ever rendering it unescaped as HTML
  in a future logging/admin UI consumer of the signal.
- **Known gap, not yet built:** no groups or per-model permissions exist yet — every active user
  is currently equally privileged from `djangors-auth`'s own point of view (an app can still gate
  on `is_staff`/`is_superuser` fields directly, but there's no `has_perm`-style API). Unscheduled.
- **Threat:** password-reset link forged/reused after the password already changed. **Mitigation:**
  `generate_password_reset_token`/`verify_password_reset_token`
  (`crates/djangors-auth/src/lib.rs`, part 4d slice 1, [[4.14-password-reset-email]]) sign a
  token embedding the user id, an expiry, and a prefix of the *current* password hash — verified
  via `Mac::verify_slice` (constant-time). Once the password changes (including via the reset
  itself completing), the embedded hash prefix stops matching and every previously issued token
  for that user fails verification, without needing a separate used-token DB table.
- **Threat:** password-reset request used to enumerate registered emails. **Mitigation:**
  `request_password_reset` always returns `Ok(())`, whether or not the email matched a user — an
  attacker can't distinguish "sent" from "no such account" via the return value. **Caught during
  review, accepted as a documented gap, not fixed:** the *timing* isn't fully equalized — the
  no-match path returns early without generating a token or calling the mail backend, unlike
  `ModelBackend::authenticate`'s deliberate equal-cost dummy-hash path for login. Considered lower
  severity than the login case: this endpoint is inherently much lower-frequency, and the dominant
  timing signal on the match path would be mail-backend I/O latency (noisy, backend-dependent),
  not a precise, low-variance crypto operation like Argon2 — a much weaker practical oracle. See
  `security-checklist.md`'s A07 section for the same note.

### Query strings / cookies parsing (`crates/djangors-core/src/request.rs`, and the cookie-header
parsers in `middleware.rs`/`djangors-sessions`)

- **Threat:** a malformed/adversarial query string or `Cookie` header panics the server (DoS) or
  triggers memory-unsafety (not applicable in safe Rust, but panics are still a real availability
  risk — an unhandled panic in a request-handling task can, depending on the async runtime's panic
  behavior, take down more than just that one request). **Mitigation:** all three parsers
  (`Request::parse_query`, `extract_cookie` ×2, `SignedCookieStore::decode`) are hand-rolled over
  `&str`/byte slices using only infallible or `Option`/`Result`-returning operations (no
  `unwrap()` on attacker-controlled data) — reviewed by inspection, and as of this doc, also
  fuzz-tested (see `fuzz/` and the fuzzing section of `security-checklist.md`) rather than relying
  on code review alone.
- **Known gap:** multipart body parsing does not exist yet (see `security-checklist.md`'s File
  Upload section) — no fuzz target exists for it because there is no parser to fuzz. This is a
  scope gap in Phase 3, not Phase 4, but it means there is currently no file-upload attack surface
  at all (nothing to attack, since nothing accepts files).

## What this document intentionally does not cover

- Anything in Phase 5+ (admin, API/token auth, WebSockets, background tasks) — those phases will
  each need a revisit of this document once built, not a speculative threat model for code that
  doesn't exist.
- Infrastructure-level concerns (TLS termination config, reverse proxy trust, container/deploy
  hardening) — Phase 6/10 territory (`djangors check --deploy`, deployment story doc).
