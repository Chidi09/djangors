# Threat model — Djangors

**Status:** living document. Last substantive revision 2026-07-31 (v0.6.0, post-Phase-14).
Everything below describes **real, already-committed code** — file/function references are exact,
not aspirational. Where a mitigation is intentionally partial, that's stated explicitly rather than
rounded up to "done."

The per-subsystem sections below were written during Phase 4 and remain accurate for the
subsystems they cover. The Assets, Adversaries, and Residual Risk sections were added in 15.x, and
the trust-boundary section was rewritten then: the original said Djangors had exactly one
untrusted-input surface and no multipart parsing, no token auth, and no tenancy. All three of those
statements have since become false, which is precisely the failure mode a living document exists to
prevent.

## Assets

What an attacker is actually after:

- **The session signing key (`settings.SECRET_KEY`) and the cookies it signs.** `SignedCookieStore`
  (`crates/djangors-sessions/src/lib.rs`) signs client-side session cookies with HMAC-SHA256.
  Compromise of the key is total: arbitrary session forgery and impersonation of any account,
  including superusers. This is the single highest-value asset in the system.
- **Password hashes.** Argon2id with per-password CSPRNG salts. Compromise exposes accounts to
  offline cracking, bounded by Argon2id's cost parameters.
- **The authenticated admin surface (`djangors-admin`).** Model CRUD over every registered model,
  which in a real deployment means the whole database.
- **Tenant isolation (`djangors-contrib-tenancy`).** A cross-tenant read is a data breach affecting
  parties who never interacted with the attacker.
- **Payment records (`djangors-contrib-payments`).**
- **Database integrity itself**, via the ORM.

## Adversaries

- **Unauthenticated internet attacker.** Arbitrary HTTP: crafted headers, query strings, path
  params, JSON, and multipart bodies. Goals are injection, CSRF, path traversal, and
  resource exhaustion in parsers.
- **Authenticated low-privilege user.** Vertical escalation (reaching admin views or exercising
  ungranted permissions) and horizontal escalation (another user's or another tenant's rows). This
  is the adversary the entire per-action `require_perm` system exists for, and the one most likely
  to actually exist in a deployed application.
- **Compromised or malicious dependency.** 472 crates in `Cargo.lock`. Not hypothetical — `cargo
  audit` runs in CI against the RustSec database and currently reports three triaged advisories
  (see `docs/security-review-2026-07-27.md`).

## Trust boundaries

- **Browser → server.** The primary untrusted surface. Every incoming byte — cookies, headers,
  query strings, path parameters, and JSON, form-encoded, **and multipart** bodies — is attacker
  controlled. Multipart parsing (`crates/djangors-core/src/extract.rs`, via `multer`) was added
  after this document's first draft and is now the largest and least-structured of these.
- **API client → server.** `djangors-rest` adds token and optional JWT authentication, a second
  authentication boundary distinct from session cookies. The `jwt` feature is optional and carries
  a known advisory (RUSTSEC-2023-0071, `rsa`); HS256/ES256 are the documented mitigation.
- **Tenant → tenant.** `djangors-contrib-tenancy` scopes queries by tenant. v1 resolves the tenant
  from an `X-Tenant-Id` header, so tenant identity is only as trustworthy as whatever sets that
  header — an application that lets a client set it directly has no isolation at all. This is a
  deployment-configuration boundary as much as a code one.
- **Low-privilege user → staff/superuser**, inside an already-authenticated session. Enforced by
  `Auth<U>`, `has_perm`, and `require_perm`.
- **Server → database.** Trusted. All queries go through the ORM's parameterised builder, with
  every identifier quoted since 12.1. The raw-SQL escape hatch that now exists is typed via sqlx
  rather than string-interpolated. The 2026-07-27 review found no raw user-value interpolation
  anywhere in the ORM or REST layers; savepoint names (15.1) are the one caller-supplied string
  that reaches query text, and are both validated against `[A-Za-z_][A-Za-z0-9_]*` and quoted.
- **Server → third parties.** SMTP (`djangors-mail`), S3 (`djangors-staticfiles`), Redis
  (`djangors-cache`).

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
- **Multipart** (superseding this section's original text, which said multipart parsing did not
  exist and that there was therefore "nothing to attack"). It exists:
  `djangors_core::extract::Multipart` over the `multer` crate, bounded by
  `Constraints::size_limit` (whole-stream and per-field). It is now the largest attacker-controlled
  parser in the project, and unlike the three hand-rolled parsers above it is third-party code.
  A `cargo-fuzz` target was added in 15.x (`fuzz/fuzz_targets/multipart.rs`). **Residual risk:** it
  buffers rather than streaming to temp files, so the size limit is the only thing standing between
  concurrent uploads and memory pressure.

## Explicitly out of scope, and why

- **Physical host access.** Overrides every logical control; belongs to the infrastructure
  provider.
- **Compromised developer workstation.** An attacker with local code execution can alter source or
  steal keys before anything this framework does takes effect.
- **A malicious superuser.** A superuser is omnipotent by design. Constraining one is an
  organisational control (separation of duties, out-of-band approval, external audit), not
  something the framework can or should try to enforce.
- **Volumetric DDoS.** The built-in rate limiter is single-process and in-memory; distributed
  volumetric attacks need edge infrastructure.
- **Infrastructure-level concerns** (TLS termination, reverse-proxy trust, container hardening) —
  `dj check --deploy` and the deployment guide cover the application side of this, not the
  network side.

## Known residual risk

Carried forward honestly rather than closed:

1. **Multipart buffers rather than streams.** Size-limited, but concurrent large uploads remain a
   memory-pressure vector.
2. **Rate limiting is single-process and in-memory.** Horizontally scaled deployments do not share
   counters without a centralised backend.
3. **`on_delete` is enforced only for `Protect`, and only in the admin layer.** `Cascade`,
   `SetNull`, `Restrict`, and `DoNothing` are metadata-only — they are not schema constraints, so
   a write that bypasses the admin bypasses them entirely.
4. **Tenant identity comes from an `X-Tenant-Id` header.** An application that lets a client set it
   has no isolation. This is a documented deployment responsibility, not a framework guarantee.
5. **Fuzzing is local and one-off**, not continuous or wired into CI/OSS-Fuzz.
6. **No independent third-party audit.** Every review to date is internal, including this document.
7. **Phase 5+ subsystems are only partially modelled here.** The admin, background tasks, and
   WebSockets/SSE each deserve a fuller pass than the per-subsystem sections above give them.
