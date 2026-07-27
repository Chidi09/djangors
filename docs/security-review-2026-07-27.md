# Internal Security Review — 2026-07-27

## What this is, and what it isn't

This is an **internal, automated + manual security review** of Djangors, performed as part of
ongoing Phase 10 hardening work. It is **not a substitute for the third-party security audit**
`PLAN.md` still lists as an open item — that item remains open, and remains something only a real
budget/vendor decision can complete, since a third-party audit's value comes specifically from
independence this review cannot provide. Treat this as real, useful groundwork that narrows what
an eventual external auditor needs to look at, not as the audit itself.

## Scope and methodology

- **Automated dependency scanning**: `cargo audit` against the full workspace `Cargo.lock` (472
  crate dependencies), checked against the RustSec advisory database (1,169 advisories loaded).
- **Manual code review** of the security-critical subsystems: CSRF middleware, session cookie
  signing, password hashing/authentication, login rate limiting, admin permission checks, the new
  per-endpoint rate limiter, static-file path-traversal protection, and the ORM's SQL construction
  (injection surface).
- **`unsafe` code audit**: a full-workspace grep for `unsafe` blocks.

## Findings

### HIGH — `quick-xml` 0.40.1, two RustSec advisories (RUSTSEC-2026-0194, RUSTSEC-2026-0195)

Pulled in transitively via the `s3` crate (added this session for the S3 storage backend,
`crates/djangors-staticfiles`). Both are denial-of-service class: a quadratic-runtime issue when
checking a start tag for duplicate attribute names, and unbounded namespace-declaration allocation
enabling memory exhaustion. Both fixed in `quick-xml` 0.41.0.

**Status: confirmed currently unfixable from this project.** `s3` 0.1.36 (the latest published
version, confirmed via the crates.io API) pins `quick-xml = "0.40.1"`, which Cargo's default caret
semantics resolve as `>=0.40.1, <0.41.0` for a 0.x dependency — verified directly with `cargo
update -p quick-xml --precise 0.41.0 --dry-run`, which fails with "candidate versions found which
didn't match: 0.41.0... required by package `s3 v0.1.36`." This can only be fixed by the `s3` crate
publishing a new release with an updated `quick-xml` requirement, or by this project switching S3
client crates. **Recommendation**: track the `s3` crate for a new release; if none appears within
a reasonable window, evaluate switching `S3Storage`'s backend crate. Re-run `cargo audit` before
every release until resolved.

### MEDIUM — `rsa` 0.9.10, Marvin Attack timing side-channel (RUSTSEC-2023-0071)

Pulled in transitively via `jsonwebtoken`, itself only reachable when `djangors-rest`'s optional
`jwt` feature is enabled (confirmed: `cargo tree -i rsa` finds nothing in the default build graph;
it only appears in `Cargo.lock` because the `jwt` feature is resolvable). **No fixed version
exists upstream** — this is a long-standing, unresolved advisory in the `rsa` crate's RSA
implementation generally, not specific to how Djangors uses it. **Recommendation**: document this
in `docs/src/guides/auth.md`'s JWT section — projects enabling the `jwt` feature and using
RSA-family JWT algorithms (RS256/RS384/RS512) should prefer HMAC (HS256) or ECDSA (ES256) signing
if the timing side-channel risk is a concern for their threat model, since those algorithm
families don't depend on the affected `rsa` crate at all.

### MEDIUM — CSRF and session cookies defaulted to non-`Secure`, and neither example app enabled it

`CsrfLayer::new()` and `SignedCookieStore::new()` both default their cookie's `Secure` attribute to
`false` — a deliberate, documented choice so local HTTP development works without extra
configuration, with a `.with_secure(bool)` builder method to opt in for production. **The gap**:
neither `examples/school` nor `examples/polls` — the two reference applications real users are
most likely to copy from — actually called `.with_secure(...)` anywhere, even though
`docs/src/guides/security.md`'s own session-cookie section already documented the correct pattern
(`.with_secure(true) // Enforce Secure attribute in production`). This meant both cookies would
still lack the `Secure` flag in a hypothetical production deployment of either example as-is,
allowing them to be transmitted over an accidental plain-HTTP connection (a misconfigured proxy, a
user typing `http://` manually, etc.).

**Fixed as part of this review**: both example apps now call
`.with_secure(!settings.debug)` on the session store and
`csrf_layer().with_secure(!settings.debug)`, tying cookie security directly to the existing
`settings.debug` flag both apps already load. The security guide's CSRF section now cross-references
this pattern explicitly (it previously only appeared in the session-cookie section). Verified: both
examples still build clean, `cargo test --workspace` and `mdbook build` both pass.

### Positive findings (no action needed, noted for completeness)

- **Zero `unsafe` code anywhere in the `crates/` tree** — confirmed via a full-workspace grep. The
  entire framework is safe Rust.
- **Password hashing uses Argon2id** (the OWASP-recommended, memory-hard variant) with per-password
  random salts generated via `OsRng` (a real OS-provided CSPRNG), not a weaker/faster algorithm.
- **Real timing-attack mitigation on login**: `AuthBackend::authenticate` runs a genuine Argon2
  verification against a constant dummy hash even when the supplied username doesn't exist,
  preventing username enumeration via response-timing differences.
- **Constant-time comparisons** are used for both CSRF token validation and session HMAC
  verification (`constant_time_eq`), not a fast-fail `==`.
- **No raw user-supplied values found interpolated into SQL strings** — a targeted grep across
  `djangors-orm`/`djangors-rest` found `format!` usage building SQL only ever interpolates
  column/table/field identifiers already validated against static `ModelMeta`, never a raw request
  value; all actual values are bound as real parameters (`$1`, `.bind(...)`).
- **The per-endpoint rate limiter's `ByIp` strategy already documents its own limitation** (header-
  based, spoofable without a trusted reverse proxy) directly in its doc comment — this was
  identified and stated honestly during its own original design (item 5 of the architecture-parity
  roadmap), not left as a silent gap.
- **Static-file path-traversal protection** (`LocalDiskStorage::resolve_path`) canonicalizes both
  the candidate path and the storage root and requires the former to be a prefix of the latter —
  this defends against both `../`-style traversal and symlink-escape attacks (both have dedicated
  regression tests: `test_local_storage_rejects_traversal`,
  `test_local_storage_rejects_escaping_symlink`).

## Summary table

| Finding | Severity | Status |
|---|---|---|
| `quick-xml` 0.40.1 (2 advisories, via `s3` crate) | High | Tracked, not fixable from this project yet |
| `rsa` 0.9.10 (via optional `jwt` feature) | Medium | No upstream fix; documented mitigation (prefer HS256/ES256) |
| CSRF/session cookies default non-`Secure`; examples didn't opt in | Medium | **Fixed** in this review |

## What this review did not cover

This pass did not include: fuzzing any parser, a formal threat model document, dependency license
auditing, a review of the `djangors-contrib-*` crates, or anything requiring dynamic/runtime
instrumentation beyond `cargo audit`'s static advisory-database check. These, plus the actual
independent third-party audit, remain open items.
