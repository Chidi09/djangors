# Security checklist — OWASP Top 10 self-assessment (Phase 4 DoD)

**Status:** point-in-time self-assessment as of Phase 4 part 4e, satisfying `PLAN.md`'s Phase 4
DoD line ("OWASP top-10 self-assessment written"). Companion to [[threat-model]] (which explains
*why* each mitigation exists and what its limits are — this doc is the flatter, checklist-shaped
view). Every "Mitigated" claim below points at a real file/function/test; nothing here is
aspirational. Revisit at the start of every future phase that touches request handling, auth, or
data access.

Categories follow OWASP Top 10:2021.

## A01:2021 – Broken Access Control

**Mitigated.** `Auth<U>` extractor (`crates/djangors-auth/src/lib.rs`) re-validates the
session's user against the DB on every request (active-status check included) — see
[[threat-model]]'s Authentication section. **Resolved:** Per-model permission system landed in
5.7.1 (commit `bb284a4`), introducing `Permission`/`Group`/`UserGroup`/`GroupPermission`/`UserPermission`
models, `has_perm`, `sync_permissions()`, and `dj createpermissions`; every admin view gained
per-action `require_perm` checks in 5.7.2 (commit `b9aed08`). (Note: object-level permissions via
`djangors-contrib-guardian` remain Phase 7).

## A02:2021 – Cryptographic Failures

**Mitigated.** Passwords: Argon2id via the `argon2` crate, never hand-rolled
(`hash_password`/`verify_password`, `crates/djangors-auth/src/lib.rs`). Session integrity:
HMAC-SHA256 via the `hmac`/`sha2` crates (`SignedCookieStore`,
`crates/djangors-sessions/src/lib.rs`), constant-time verification via `Mac::verify_slice`.
Random values (session keys, CSRF tokens): `rand::thread_rng()` (CSPRNG-backed), not
`std::hash`/predictable sources.
**Resolved (CSRF form-body validation):** Core-level `csrfmiddlewaretoken` form-body validation
landed in 5.8.9 (commit `8a448db`) and every admin `<form>` emits the hidden field as of 5.8.10
(commit `b2c5d16`). A plain browser form POST works end to end without JS.
**Resolved in example apps (`Secure` cookie flag):** `Secure` cookie flag defaults to opt-in via
`with_secure(bool)` for local HTTP development. Both example apps (`examples/school` and `examples/polls`)
were updated during the 2026-07-27 security review to wire `.with_secure(!settings.debug)`.

## A03:2021 – Injection

**Mitigated by design.** All DB access goes through the ORM's parameterized query builder
(`djangors_orm::Model::objects()`/`filter(q!(...))`) — no raw SQL string interpolation path exists
in framework code today (a raw-SQL escape hatch is planned per `PLAN.md` Phase 2 but not yet
built, so there's no first-party SQL-injection surface to assess beyond the ORM's own query-builder
correctness, covered by the ORM crate's own test suite, out of scope for this doc). No template
engine is exercised by anything in Phase 4 (djangors-template auto-escapes by default per its own
design, per `PLAN.md` Phase 3 — not re-verified here since it's outside this phase's scope).

## A04:2021 – Insecure Design

**Addressed via this project's own working pattern**, not a one-time checklist item: every
non-trivial piece of Phase 4 was preceded by a `docs/design/4.N-*.md` doc that explicitly states
what's deferred and why (see [[4.7-sessions]] through [[4.13-security-review-fuzzing]]), rather
than ad hoc implementation. The [[threat-model]] doc is the living artifact this checklist expects
to be kept current.

## A05:2021 – Security Misconfiguration

**Partially mitigated, opt-in-heavy.** `SecurityHeadersLayer` (X-Frame-Options,
X-Content-Type-Options, Referrer-Policy) is unconditional wherever wired in, but *wiring it in* is
still an app/example-app decision — no framework-level "secure by default, opt out" project
scaffold exists yet (that's Phase 6's `djangors new` generator territory). `HstsLayer` and
`HostValidationLayer` are both fully opt-in with an insecure-if-unconfigured default
(`HostValidationLayer::new(vec![])` is unrestricted — mirrors Django's own
`ALLOWED_HOSTS = []` DEBUG-mode behavior, not a Djangors-specific weakness, but still worth
flagging). **No default Content-Security-Policy** — `PLAN.md`'s "CSP helper" line item is not yet
built. **Gap**, tracked, not yet scheduled.

## A06:2021 – Vulnerable and Outdated Components

**Process, not a one-time state.** This project has an established discipline (enforced during
every dispatch-review cycle this session) of checking `Cargo.lock` for an already-resolved
version before pinning any new dependency, to avoid duplicate/incompatible crate versions in the
build graph — caught and fixed twice this session (a `sha2` version mismatch in djangors-sessions,
an unused `tracing` dependency). `PLAN.md`'s Phase 0 line calls for `cargo-deny` in CI for
licenses/advisories — **not yet wired up** (Phase 0/CI scope, not done as of this doc). **Gap.**

## A07:2021 – Identification and Authentication Failures

**Mitigated for what's built**, see [[threat-model]]'s Authentication section in full: Argon2id
hashing, timing-oracle-resistant `ModelBackend::authenticate` (dummy-hash path always exercised),
session-fixation protection (`login()`'s `cycle_key()`), login rate limiting
(`RateLimitedBackend`, opt-in, single-process — gap noted in threat model), re-validated
active-user check on every authenticated request (`Auth<U>`). Password reset now exists
(`generate_password_reset_token`/`verify_password_reset_token`/`request_password_reset`/
`confirm_password_reset`, part 4d slice 1, [[4.14-password-reset-email]]): a signed token embeds
the user id, an expiry, and a prefix of the user's *current* password hash, so any previously
issued token self-invalidates the moment the password actually changes — no separate
used-token table needed. **Minor gap found during review:** unlike `ModelBackend::authenticate`'s
deliberate equal-cost dummy-hash path for both found/not-found usernames,
`request_password_reset` does *not* do equivalent dummy work when the email doesn't match any
user — it returns early without generating a token or calling the mail backend, so a
sufficiently precise timing measurement could in principle distinguish "email registered" from
"email not registered." Accepted as low-severity for v1: this endpoint is inherently
lower-frequency than login, and the dominant timing signal would be mail-backend I/O latency
(itself noisy/variable), not a precise crypto operation like Argon2 — a much weaker oracle than
the login case. Tracked here rather than silently accepted. **Gap:** no
account-lockout-notification/audit-visible-to-user story, no MFA (`djangors-contrib-otp` is
Phase 7).

## A08:2021 – Software and Data Integrity Failures

**Not yet applicable / not yet assessed.** No CI/CD pipeline signing, no deserialization of
untrusted data beyond JSON/form bodies (handled via `serde`'s typed deserialization, not
`eval`-style dynamic loading), no auto-update mechanism exists in this framework. Revisit when
Phase 6 (CLI/deployment) or Phase 8 (background tasks, which may deserialize queued job payloads)
land.

## A09:2021 – Security Logging and Monitoring Failures

**Partially mitigated.** Audit signals exist for the auth-relevant events —
`LOGIN_SUCCEEDED`/`LOGIN_FAILED`/`LOGGED_OUT`
(`crates/djangors-auth/src/lib.rs`, part 4c) — and general request logging exists via
`logging_layer()` (`tower_http::trace::TraceLayer`, `crates/djangors-core/src/middleware.rs`).
**Gap:** nothing *consumes* the audit signals yet (no default subscriber writing them to a
durable audit log/table — that's `djangors-contrib-audit`, Phase 7). A signal firing with zero
subscribers is a no-op in practice for any app that hasn't wired one up itself.

## A10:2021 – Server-Side Request Forgery (SSRF)

**Not applicable yet.** No framework code makes outbound HTTP requests to app/user-supplied URLs
(no webhook delivery, no URL-preview/fetch feature, no image-proxy). Revisit if/when such a
feature is built (none currently planned in `PLAN.md`).

## Parser robustness (fuzzing) — supplementary to the Top 10 above

Query-string parsing (`Request::parse_query`), both cookie-header parsers (`extract_cookie` in
`djangors-core`/`djangors-sessions`), and signed session-cookie decoding
(`SignedCookieStore::decode`) are all attacker-facing (values a browser fully controls) and are
now covered by `cargo-fuzz` targets in `/root/dev/Rango/fuzz/` (see
[[4.13-security-review-fuzzing]] Part 1 for exact target scope and how each was smoke-tested).
**Gap (Strengthened):** Multipart body parsing exists via the `Multipart` extractor in
`crates/djangors-core/src/extract.rs` (using the `multer` crate with `Constraints::size_limit`).
It buffers full request bodies in memory rather than streaming to temporary files, making it the
largest attacker-controlled parser in the project with **no fuzz coverage**. A dedicated multipart
fuzz target is needed. **Gap:** fuzzing is a one-off local smoke run per target, not wired into
CI/OSS-Fuzz for continuous coverage — future work.

## Summary of open gaps (all tracked above, repeated here for scanning)

1. **Resolved:** Core-level `csrfmiddlewaretoken` form-body validation landed in 5.8.9 (commit `8a448db`) and admin forms emit the hidden field as of 5.8.10 (commit `b2c5d16`).
2. **Resolved in example apps:** `Secure` cookie flag is opt-in for local dev; both example apps (`examples/school` and `examples/polls`) were updated during 2026-07-27 review to wire `.with_secure(!settings.debug)`.
3. No default Content-Security-Policy.
4. Rate limiting is single-process/in-memory, not distributed.
5. **Resolved:** Groups and per-model permissions (`Permission`/`Group`/`UserGroup`/`GroupPermission`/`UserPermission`, `has_perm`, `sync_permissions()`, `dj createpermissions`) landed in 5.7.1 (commit `bb284a4`) and admin views enforce `require_perm` as of 5.7.2 (commit `b9aed08`).
6. `request_password_reset` doesn't do dummy-work timing equalization for nonexistent emails
   (accepted low-severity, see A07 above).
7. `cargo-deny` not yet wired into CI.
8. Audit signals fire but have no default durable-storage subscriber (Phase 7's
   `djangors-contrib-audit`).
9. **Strengthened (Open Gap):** Multipart body parsing exists (`djangors-core::extract::Multipart` using `multer` with size limits), buffering request bodies in memory, but has no fuzz coverage — making it the largest unfuzzed attacker-controlled parser in the workspace.
10. Fuzzing is local-smoke-only, not continuous.
11. `djangors-mail`'s `ConsoleBackend` is a deliberately minimal Phase-4-only pull-forward (no
    SMTP/file/HTML backends) — the real Phase 7 `djangors-mail` is separate, unscheduled work.
