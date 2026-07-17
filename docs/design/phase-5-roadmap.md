# Phase 5 roadmap — status, sequencing, and the deferred-items ledger

**Last updated:** 2026-07-17 (after commit 9b2fd47). This is the authoritative status document
for Phase 5 (THE ADMIN) and the single place where every deliberately-deferred item across the
project is tracked, so no session ever has to re-derive project state from git archaeology.
Update this file whenever a slice lands or a new deferral is made.

## Where things stand

Phases 0–4 are fully done and committed (Phase 4 DoD met: polls requires login to vote,
password-reset works via console mail backend, OWASP self-assessment written —
`security-checklist.md` / `threat-model.md` in this directory are the living security docs).

Phase 5 slices landed so far:

| Slice | Design doc | Commit | What it built |
|---|---|---|---|
| 5.1 | `5.1-admin-registry-login.md` | 9edf249 | `AdminSite` + registry, `ModelAdmin` trait + `DefaultModelAdmin`, staff-gated `/` index, `Forbidden` (403) error variant. Registry snapshot captured in handler closures because `Router::mount` never carries sub-router state. |
| 5.2 | `5.2-admin-changelist.md` | ac647f9 | Changelist at `/{app}/{model}/`: all fields as columns via derive-emitted `Model::field_values()`/`field_names()`, click-to-sort (`?o=`, validated by `order_by` against `ModelMeta` — injection/reflected-XSS safe by construction), page-100 pagination, cells escaped via now-public `djangors_core::html_escape`. Also fixed a Phase-2-latent ORM bug: `count()`/`exists()`/`aggregate()` emitted `ORDER BY` from default `meta.ordering`, which Postgres rejects in aggregate queries. |
| 5.3 | `5.3-createsuperuser-polls-admin.md` | 9b2fd47 | Real `dj createsuperuser` (non-interactive, `DJANGORS_SUPERUSER_PASSWORD` env, argon2, duplicate check) + admin mounted in `examples/polls` (`admin.rs` is now a real `admin.py`-equivalent), socket-level integration tests. Also fixed a latent macro bug: derived `save()`/`update()` bound every SQL NULL as `None::<i64>`, breaking any `None` value in a non-i64 nullable column (e.g. `last_login: None` against TIMESTAMPTZ); NULL binds are now typed per-field. |

Working infrastructure a new session should know exists (all proven by tests):
- `Model::field_values(&self) -> Vec<(&'static str, Value)>` and `Model::field_names()` —
  derive-generated, declaration order, FKs as related id. The generic "render any model"
  mechanism the rest of the admin builds on.
- `impl Display for Value` (DateTime `%Y-%m-%d %H:%M:%S`, Null `-`).
- `ModelAdmin` is `#[async_trait]` + object-safe; registry holds `Arc<dyn ModelAdmin>`;
  `DefaultModelAdmin<M>` drives the typed QuerySet. `require_staff()` gate helper.
- `CHANGELIST_PER_PAGE = 100` (`pub(crate)` const in djangors-admin).

## Recommended sequencing for the remaining Phase 5 bullets

1. **5.4 — Change form v1 (add + edit), the next big rock.** The missing primitive is the
   *reverse* of `field_values()`: constructing/updating a model instance from form-submitted
   strings. Design questions to resolve in the doc before dispatch:
   - Probably a derive-emitted `fn apply_field_values(&mut self, ...)` or
     `fn from_field_strings(...)` with per-`FieldKind` parsing (String passthrough, i32/i64
     parse, bool checkbox semantics, DateTime parse of `%Y-%m-%d %H:%M:%S`, FK id parse),
     returning per-field validation errors — this is also the seed of Phase 6's ModelForm.
   - POST target routes `/{app}/{model}/{pk}/change/` and `/{app}/{model}/add/`; CSRF already
     works (polls proves the token/cookie/header flow); render plain-format! forms with
     `html_escape`d values.
   - `auto`/pk fields readonly; `Option` fields get an empty-string→None rule.
   - Changelist first-column links to the change form once it exists (Django convention,
     explicitly deferred in 5.2).
2. **5.5 — Delete confirmation** (GET confirm page + POST delete). Full related-object
   collection needs FK metadata traversal (`RelationMeta` exists); v1 can list direct FK
   dependents only. Depends on nothing else.
3. **5.6 — Changelist customization**: `list_display` (subset of fields + computed methods —
   needs a `ModelAdmin` method returning column closures), `search_fields` (ILIKE across
   named fields), `list_filter` v1 (bool/choices fields), then `date_hierarchy`,
   `list_editable`, bulk actions (delete first, then CSV export — XLSX needs a new dependency,
   justify it then), saved views last.
4. **5.7 — Permissions**: requires groups/model-level permissions, which were *deferred out of
   Phase 4* — that work (auth tables, permission checks) has to land before "permission
   enforcement everywhere" is possible. Design it as its own auth-side doc first.
5. **5.8 — History/audit, theming, extension points**: after CRUD is complete. Theming is the
   right moment to introduce djangors-template into the admin (every page is plain `format!`
   HTML until then, deliberately).
6. **School example + DoD**: the Phase 5 DoD references a school example that does not exist
   yet — building it (models: students, enrollment, grades) is its own slice near the end,
   and is the real acceptance test for "a non-programmer can CRUD comfortably".

## Deferred-items ledger (project-wide)

Deliberate deferrals, where they're documented, and when they should land. Nothing here is
forgotten-by-accident; do not silently re-defer past the milestone listed.

**Blocking parts of Phase 5:**
- **Groups + model-level permissions** (deferred from Phase 4, `4.9`-era decision): needed for
  5.7 permission enforcement. The admin currently has exactly one permission: `is_staff`.
- **Generic `AuthUser` support in the admin** (5.1 scope decision): the staff gate is hardcoded
  to concrete `djangors_auth::User`; extending `AuthUser` with `is_staff()` is the documented
  path when a real custom-user use case appears.
- **FK display beyond raw id** (5.2): Django shows the related object's `__str__`; we have no
  Display-for-model convention. Revisit with the change form's FK widget (5.4/5.6).

**Admin-adjacent, non-blocking:**
- **Interactive `createsuperuser` prompting** (5.3): needs a hidden-input dep (e.g. rpassword);
  env-var mode is the CI-friendly v1.
- **Admin theming/templates** (5.2/5.3): all admin HTML is `format!` strings until 5.8.

**Framework-wide, owed to later phases:**
- **Real migration files / `dj makemigrations`** (Phase 3 deferral): still a stub; every test
  uses raw-SQL `CREATE TABLE`. Owed to Phase 6. The admin does not depend on it.
- **Named-route reversal `reverse!()`** (Phase 2 deferral): views hand-format URLs. Owed to
  Phase 6; the admin's relative-link approach sidesteps it for now.
- **Multipart/file-upload parsing** (Phase 2 deferral): no admin file widgets until it exists
  (Phase 6/7 territory).
- **CSRF form-body-field validation** (Phase 4, `4.8`): header+cookie double-submit only;
  hidden-input tokens unsupported. Matters more once server-rendered admin forms (5.4) are the
  main POST surface — revisit during 5.4.
- **Distributed rate limiting** (Phase 4, `4.12`): `RateLimitedBackend` is single-process
  in-memory, documented on the type. Phase 8 (deployment) territory.
- **Full djangors-mail** (Phase 4, `4.14`): console backend only, explicitly scoped as the
  Phase-4 minimum; SMTP etc. is the Phase 7 deliverable.
- **CSP helper layer** (Phase 4 security middleware bullet): headers middleware ships
  HSTS/XCTO/Referrer-Policy/XFO but no CSP builder. Owed to Phase 7/8; tracked in
  `security-checklist.md`.
- **Timing-oracle gap in `request_password_reset`** (Phase 4, `4.14` review): no dummy work on
  unknown email, documented as accepted-low-risk in `threat-model.md` (A07) — a deliberate
  accept, not a todo, unless the threat model changes.
- **cargo-deny in CI + continuous fuzzing** (Phase 4, `4.13`): fuzz targets exist and run
  manually (`fuzz/`, needs nightly); wiring into CI is owed to Phase 6's CI work.
- **Session engines beyond signed-cookie** (Phase 4 scope decision): database/cache session
  stores still unbuilt; signed-cookie is the only real store.
- **Publishing to crates.io**: blocked on the user running `cargo login` — not an
  implementation task. All 15 crates are otherwise publish-shaped (metadata present).

## Process (unchanged, for any fresh session)

Orchestrator/reviewer/committer only: design doc → exhaustive dispatch prompt → `agy --model
"Gemini 3.5 Flash (Low)"` background dispatch → background monitor (done-marker / transient
"Error: timeout waiting for response" / flat-CPU stall with rustc-child check) → independent
verification (read every diff; `cargo fmt --all`, `build --workspace --all-targets`, `clippy
--workspace --all-targets -- -D warnings`, `test --workspace`) → fix small mechanical issues
personally, redispatch only structural ones → detailed commit (author `chidi09
<chidiisking7@gmail.com>`, no co-author trailer) → update this file + living security docs →
next slice without pausing. Postgres for tests: `postgres://postgres:postgres@localhost/
djangors_test`. The review step has caught a real bug on nearly every dispatch — keep it at
full rigor.
