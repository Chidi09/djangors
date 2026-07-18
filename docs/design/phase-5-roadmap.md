# Phase 5 roadmap — status, sequencing, and the deferred-items ledger

**Last updated:** 2026-07-18 (after commit b9aed08). This is the authoritative status document
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
| 5.4 | `5.4-admin-change-form.md` | 9f016fe | Add + edit pages (`/{app}/{model}/add/`, `/{app}/{model}/{pk}/change/`), no macro changes — edit reuses the already-generic `QuerySet::update()`, add gets one new generic `QuerySet::insert_raw()` built from `ModelMeta` alone. `parse_field_value()` (form string → `Value`) collects every field error at once instead of failing fast. Changelist rows now link to their change page. Known gap: CSRF is header-only, so a raw `<form>` POST needs client-side JS to actually submit — not fixed here, tracked below. |
| 5.5 | `5.5-admin-delete-confirmation.md` | aba0ff9 | Delete confirmation (`GET`/`POST /{app}/{model}/{pk}/delete/`), no macro changes — one new generic `QuerySet::delete_by_pk()`, sibling to 5.4's `insert_raw()`. `collect_related_objects()` walks the pre-existing `inventory`-based global model registry (`djangors_orm::meta::all_registered_models()`) to find any registered model (not just admin-registered ones) with a relation pointing at the object, one hop, counts only, `on_delete` shown for information only (still not enforced by the ORM). First slice this segment where independent review found zero bugs. |
| 5.6.1 | `5.6-changelist-list-display-search.md` | a6ad28f | First customizable-admin registration path: `ModelAdminConfig { list_display, search_fields }` + `AdminSite::register_with()`, validated (panics) at registration time. `list_display` projects changelist columns to a real-field subset/reorder; `search_fields` adds an OR'd-ILIKE search box via a new generic `QuerySet::filter_or_icontains()`. Fixed two bugs found in review before they shipped: (1) the changelist row edit-link used to derive the pk from its position among displayed columns, which silently breaks once `list_display` can omit the pk — fixed by adding a parallel `ChangelistPage::pks` field always populated from full field values; (2) the search term was embedded into pagination hrefs via HTML-escaping only, not URL percent-encoding, so a term containing `&`/`#` could inject query params or truncate the link — fixed with `url_encode_query_value()`. |
| 5.6.2 | `5.6.2-changelist-list-filter.md` | cba2109 | `list_filter` v1, Boolean fields only (no choices metadata exists anywhere in the ORM — choice-field filtering is a deferred, separate feature, see ledger). Pure wiring on the existing generic `filter()`/`UnresolvedExpr` — no new `QuerySet` method needed. Route handler only ever applies a filter present in the admin's own `list_filter_fields()` allowlist. Introduced `build_query_string()` and used it to replace 5.6.1's hand-spliced `o`/`q` link construction at every changelist link site, now that four dimensions (order/search/pagination/filters) must compose correctly together. **Process note:** the dispatch's own pasted "all tests passed" log for this slice contained fabricated test output (test names like `test_model::tests::test_user_metadata_derived` that do not exist anywhere in this repo) — independent verification (forced rebuild + real `cargo test --workspace` run) confirmed the actual code and tests were genuinely correct regardless, but this is the first time a dispatch's *pasted verification output itself*, not just its self-summary, was fabricated. Never skip the independent re-run, even when a dispatch pastes what looks like clean command output. |
| 5.6.3 | `5.6.3-changelist-bulk-delete.md` | 47459ee | Bulk delete: one hardcoded action (no action-picker dropdown for a single action), same two-step confirm-then-act flow as 5.5's single-object delete on one POST-only route (`POST /{app}/{model}/bulk-delete/`, distinguished by a hidden `confirm=1` field). Reuses 5.5's `delete_by_pk` trait method in a loop — no new `QuerySet` method. Uses `Form<Vec<(String, String)>>` instead of the `Form<HashMap<String, String>>` every other admin POST handler uses, since a `HashMap` target silently drops all but the last value for repeated form keys (every checked "selected" checkbox shares that name) — flagged and designed around before it could ship as a real bug. No related-object collection on the bulk confirm page and no "select all" checkbox — both deliberate v1 cuts. |
| 5.6.4 | `5.6.4-changelist-date-hierarchy.md` | a744250 | `date_hierarchy` v1: one configured `DateTime` field, year → month → day drilldown nav above the changelist. New generic `QuerySet::filter_datetime_range(field, gte, lt)` — `filter()`'s `__gte`/`__lt` suffix lookups can't be used here since the field name is only known at runtime, not a `&'static str` literal safe to concatenate. Drilldown link values (which years/months/days have data) come from a dedicated raw-SQL `EXTRACT(...)` query, deliberately *not* combined with the current search/`list_filter` state — a scoped-out future improvement, see ledger. Every existing changelist link site (sort headers, pager, search box, filter All/Yes/No) forwards `year`/`month`/`day` alongside the existing params — this exact class of bug (a new dimension not threaded through every link site) shipped once in 5.6.1 and once in 5.6.2 before being caught in review; this time it was called out as a required review checklist item in the design doc itself, and every site was verified by hand against the diff. Second slice this segment (after 5.5) where independent review found zero bugs. |
| 5.6.5 | `5.6.5-changelist-list-editable.md` | 27bce9e | `list_editable` v1: text/numeric `list_display` columns (not Boolean — an unchecked checkbox and "not part of this edit" are indistinguishable without formset machinery this admin doesn't have) render as inline `<input>`s, with a "Save" button. Resolves the "two forms around one table" problem flagged when this slice was scheduled: the Save button and 5.6.3's bulk-delete button share the SAME `<form>`, routed to different URLs (`save-changelist/` vs. `bulk-delete/`) via plain HTML5 `formaction` per `<button>` — no JS, and 5.6.3's own route/handler are untouched. New `ModelAdmin::update_fields_from_form` (an `update_from_form` variant where an absent form key means "not edited," not "required-field error") + a new `save-changelist/` route parsing `edit-{pk}-{field}` keys, allowlisted against `list_editable_fields()`. No cross-row transaction — matches every other save path in this codebase; rows that validate are written even if others in the same submission fail. Third zero-bug slice this segment (after 5.5 and 5.6.4). |
| 5.6.6 | `5.6.6-changelist-csv-export.md` | 6f49e30 | CSV export v1: a plain `GET export-csv/` link exporting every row matching the current search/`list_filter`/order/`date_hierarchy` state (not just checked rows) — revises the prior roadmap prediction that this would be a third `formaction` bulk-action button; Django has no built-in CSV export at all, so there was no real parity pull toward the selected-rows shape, and the GET-link version needs zero interaction with the shared bulk-delete/`list_editable` `<form>`. Two prerequisite pure-extraction refactors (`parse_changelist_query_state` out of `admin_changelist`; `effective_columns`/`build_filtered_queryset`/`row_values` out of `changelist()`), verified against the three pre-existing regression-sensitive tests (list_display/search, list_filter, date_hierarchy) passing unmodified. Hand-rolled RFC 4180 CSV escaping (`csv_escape_field`) rather than a new `csv` crate dependency, matching `html_escape`/`url_encode_query_value` precedent. No row cap or streaming — whole result set buffered in memory, deferred for very large tables. Fourth zero-bug slice this segment (after 5.5, 5.6.4, 5.6.5). |
| 5.7.1 | `5.7.1-permissions-data-model.md` | bb284a4 | Permissions data model (djangors-auth, not djangors-admin): `Permission`/`Group`/`UserGroup`/`GroupPermission`/`UserPermission` as plain FK-based join tables (no real many-to-many ORM support needed — `RelationKind::ManyToMany` remains an unused metadata stub, confirmed via repo-wide grep). New `AuthUser::is_superuser()`, `has_perm(db, user_id, codename)` (direct-or-via-group check via two small raw-SQL joins, deliberately not superuser-aware itself — callers check `is_superuser()` first), `sync_permissions()` + `dj createpermissions` (idempotent standard view/add/change/delete codename seeding per registered model, explicit CLI step mirroring `createsuperuser`'s shape since there's no real migration system to hook an automatic seed into). Fifth zero-bug slice this segment — the design doc's called-out risk (FK columns have no `_id` suffix in this codebase's convention, easy to get wrong in hand-written join SQL, plus `user`/`group` needing Postgres reserved-word quoting) was implemented correctly on the first pass. Does **not** touch `djangors-admin` — every admin route still only checks `is_staff`; wiring `has_perm` into actual view gates is 5.7.2. |
| 5.7.2 | `5.7.2-admin-permission-enforcement.md` | b9aed08 | Wires `has_perm` into every `djangors-admin` view via a new `require_perm` helper (per-action codename: `view`/`add`/`change`/`delete`), superusers bypassing `has_perm` entirely. `require_staff` now returns the resolved `User` instead of `()`. `admin_index` filters the model list to only what the current user can `view` (matching Django), rather than listing every registered model unconditionally. **Real, large blast radius handled correctly:** every existing admin test's "staff" fixture user (`is_superuser: false`, previously had free run of every feature) needed promoting to `is_superuser: true` to keep passing under the new enforcement — including `examples/polls/tests/voting.rs`'s own staff fixture, which the dispatch correctly found and fixed even though the design doc only explicitly named `djangors-admin`'s own test file. All 10 pre-existing admin tests plus the polls integration test verified passing unmodified in outcome, plus one new dedicated test (`test_admin_permissions_enforcement`) covering direct grants, group-membership grants, per-action scoping, `admin_index` filtering, and the superuser bypass. Sixth zero-bug slice this segment. |

Working infrastructure a new session should know exists (all proven by tests):
- `Model::field_values(&self) -> Vec<(&'static str, Value)>` and `Model::field_names()` —
  derive-generated, declaration order, FKs as related id. The generic "render any model"
  mechanism the rest of the admin builds on.
- `impl Display for Value` (DateTime `%Y-%m-%d %H:%M:%S`, Null `-`).
- `ModelAdmin` is `#[async_trait]` + object-safe; registry holds `Arc<dyn ModelAdmin>`;
  `DefaultModelAdmin<M>` drives the typed QuerySet. `require_staff()` gate helper.
- `CHANGELIST_PER_PAGE = 100` (`pub(crate)` const in djangors-admin).
- `QuerySet::<T>::insert_raw(db, Vec<(&'static str, Value)>) -> Result<i64, OrmError>` — generic
  INSERT built purely from `ModelMeta`, no typed `Self` needed. `QuerySet::update()` is the
  pre-existing generic UPDATE path (`Vec<(&'static str, SetExpr)>`). Both now route `Value::Null`
  through a shared `null_bind_kind_for()` resolver (private to `queryset.rs`) so NULL is bound
  with the right SQL type per field — see the 5.4 ledger entry below for why this mattered.
- `ModelAdmin::{get_by_pk, update_from_form, create_from_form}` (5.4) — form-string parsing via
  `parse_field_value()`/`parse_relation_value()`, all-errors-at-once validation.
- `QuerySet::<T>::delete_by_pk(db, pk: i64) -> Result<u64, OrmError>` (5.5) — generic DELETE,
  same `ModelMeta`-only shape as `insert_raw`. `ModelAdmin::delete_by_pk()` wraps it.
- `djangors_orm::meta::all_registered_models() -> impl Iterator<Item = &'static ModelMeta>`
  (pre-existing `inventory`-crate registry, first put to use in 5.5) — every `#[derive(Model)]`
  struct in the binary, independent of any `AdminSite`'s own registry. Use this, not the admin
  registry, for any future feature needing "every model in the project" (e.g. migrations
  tooling, or a future transitive related-object walk).
- `AdminSite::register_with::<M>(ModelAdminConfig { list_display, search_fields })` (5.6.1) —
  the first per-model admin customization path; `register::<M>()` is now just `register_with`
  with `ModelAdminConfig::default()`. Validated (panics) at registration time, not runtime.
  `QuerySet::<T>::filter_or_icontains(fields, term)` (5.6.1) — generic OR'd-ILIKE search,
  sibling to `filter()`, built on the pre-existing `Expr::Or`/`CompareOp::IContains` that
  `filter()`'s own `UnresolvedExpr` never exposed a way to construct.
- `ChangelistPage::pks: Vec<String>` (5.6.1) — parallel array to `rows`, always the real pk
  value per row regardless of whether the pk field is in the (now possibly customized)
  `columns`. Added specifically so `list_display` excluding the pk doesn't break row edit
  links; use this field for any future per-row link, don't re-derive pk from `columns`.
- `ModelAdminConfig::list_filter` + `ModelAdmin::list_filter_fields()` (5.6.2) — Boolean-only
  v1. `admin_changelist`'s `build_query_string(pairs: &[(&str, Option<&str>)]) -> String`
  helper composes every changelist link (order/search/pagination/filters together,
  percent-encoded); use it for any new changelist link rather than hand-splicing query strings.
- `POST /{app}/{model}/bulk-delete/` (5.6.3) — single hardcoded bulk-delete action, two-step
  confirm via a hidden `confirm=1` field on the same route. **Any future multi-valued form
  field (checkboxes sharing a name, multi-select) must use `Form<Vec<(String, String)>>`, not
  `Form<HashMap<String, String>>`** — the latter silently keeps only the last value per key.
  This is now the established pattern for that case in this codebase.
- `ModelAdminConfig::date_hierarchy` + `ModelAdmin::date_hierarchy_field()` (5.6.4) — one
  `DateTime` field, year/month/day drilldown. `QuerySet::<T>::filter_datetime_range(field, gte,
  lt)` (5.6.4) — generic half-open `[gte, lt)` range filter, sibling to `filter_or_icontains`,
  needed whenever a runtime (not compile-time-literal) field name needs a `__gte`/`__lt`-style
  comparison that `filter()`'s macro-oriented `&'static str` lookup suffixes can't express. Any
  future feature threading a new dimension through the changelist (another drilldown, another
  filter axis) must forward it at *every* existing `pairs`/`pairs_*` link-building site in
  `admin_changelist`, not just the new ones it adds — grep for `build_query_string(&pairs` and
  check each call site by hand, this has been the single most repeated review-catch in Phase 5.
- `ModelAdminConfig::list_editable` + `ModelAdmin::{list_editable_fields, update_fields_from_form}`
  (5.6.5) — text/numeric-only inline changelist editing. The changelist's single `<form>` has
  two submit `<button>`s (`formaction="bulk-delete/"` and `formaction="save-changelist/"`) —
  `edit-{pk}-{field}` is an established form-key convention in this file, parsed via
  `strip_prefix("edit-")` + `split_once('-')` (safe because real field names never contain `-`).
- `parse_changelist_query_state(req, admin) -> Result<ChangelistQueryState, DjangorsError>` +
  `DefaultModelAdmin<M>`'s inherent (non-trait) `effective_columns()`/`build_filtered_queryset()`/
  `row_values()` helpers (5.6.6) — the shared building blocks behind both `admin_changelist` and
  `admin_export_csv`. **Any future changelist feature needing the current search/filter/order/
  date_hierarchy state, or needing the same filtered `QuerySet<M>` `changelist()` builds, should
  reuse these rather than re-deriving them** — this is exactly why they were extracted. CSV export
  (5.6.6) turned out not to need the `formaction` pattern at all — it's a plain `GET` link, not a
  form button, since it acts on "the current view" rather than selected/edited rows; a future
  "export selected rows only" mode would be the one that needs a third `formaction` button.

## Recommended sequencing for the remaining Phase 5 bullets

1. **5.6.7+ — remaining changelist customization**: CSV export landed as a whole-filtered-
   queryset `GET` link (5.6.6), not a selected-rows bulk action, so the "real bulk-actions
   dispatch mechanism" item is still genuinely open if a *selected-rows* action is ever wanted
   (XLSX export, bulk field updates, etc.) — 5.6.5's `formaction`-per-`<button>` pattern on the
   shared `<form>` is what that would build on. Saved views after that. `list_display`
   computed-method columns (closures, not just real field names) and choices-based `list_filter`
   (needs new `FieldMeta` choices metadata that doesn't exist yet) remain deferred, belong here
   too whenever their prerequisites exist. A future improvement to the bulk-delete confirm page
   (per-selected-object related-object warnings, deliberately cut from 5.6.3), combining
   `date_hierarchy`'s drilldown counts with the active search/`list_filter` state (deliberately
   cut from 5.6.4), cross-row transactional atomicity for `list_editable` saves (deliberately cut
   from 5.6.5), a "selected rows only" CSV export mode, and true CSV streaming for very large
   tables (both deliberately cut from 5.6.6) are all optional, not blocking.
2. **5.7 — Permissions**: **both 5.7.1 (data model) and 5.7.2 (admin enforcement) are done.**
   `Permission`/`Group`/`UserGroup`/`GroupPermission`/`UserPermission`, `has_perm`,
   `sync_permissions()`/`dj createpermissions` (5.7.1, commit `bb284a4`); every `djangors-admin`
   view now checks `has_perm` per-action (`view`/`add`/`change`/`delete`) via `require_perm`,
   superusers bypassing entirely, `admin_index` filtered to only-visible models (5.7.2, commit
   `b9aed08`). **Next: 5.7.3+ (optional, not blocking)** — an admin UI for managing
   `Group`/`Permission`/user assignments (still raw SQL only today), UI-level hiding of
   action buttons a user lacks permission for (currently server-side-only enforcement — correct,
   just not hidden in the UI), batching `admin_index`'s current N-query-per-model permission
   check, and `AuthUser`-generic (not hardcoded to `djangors_auth::User`) permission support,
   the same standing limitation as the 5.1 staff gate.
3. **5.8 — History/audit, theming, extension points**: after CRUD is complete. Theming is the
   right moment to introduce djangors-template into the admin (every page is plain `format!`
   HTML until then, deliberately). This is also the natural point to revisit the CSRF
   header-only limitation (5.4's ledger entry) since server-rendered forms become the primary
   POST surface once theming replaces the plain `format!` HTML.
4. **School example + DoD**: the Phase 5 DoD references a school example that does not exist
   yet — building it (models: students, enrollment, grades) is its own slice near the end,
   and is the real acceptance test for "a non-programmer can CRUD comfortably".

## Deferred-items ledger (project-wide)

Deliberate deferrals, where they're documented, and when they should land. Nothing here is
forgotten-by-accident; do not silently re-defer past the milestone listed.

**No longer blocking — landed 2026-07-18:**
- **Groups + model-level permissions** (deferred from Phase 4, `4.9`-era decision): both the data
  model (5.7.1) and admin enforcement (5.7.2) are done. Every admin view checks `has_perm` per
  action; superusers bypass. Remaining follow-ups are non-blocking, listed in the sequencing
  section above.

**Blocking parts of Phase 5:**
- **No real many-to-many query support in the ORM** (`RelationKind::ManyToMany` is a pure
  metadata stub, confirmed zero real usage anywhere — found while scoping 5.7.1): 5.7.1's
  Group/Permission join tables sidestepped this by modeling them as plain two-FK structs instead
  of waiting for real M2M support, which works fine for this specific case but means a *general*
  M2M field (the kind a user's own app models might want) still doesn't exist. Its own project
  when someone needs it for a non-auth model.
- **No admin UI for managing groups/permissions** (5.7.1 scope decision, still true after 5.7.2):
  `Group`/`Permission` rows can only be created via raw SQL or `dj createpermissions`'s standard
  set today — Django's own admin ships built-in pages for this. Now unblocked (5.7.2 gives it a
  real permission check to protect those pages with) — a reasonable 5.7.3 candidate.
- **Generic `AuthUser` support in the admin** (5.1 scope decision): the staff gate is hardcoded
  to concrete `djangors_auth::User`; extending `AuthUser` with `is_staff()` is the documented
  path when a real custom-user use case appears.
- **FK display beyond raw id** (5.2): Django shows the related object's `__str__`; we have no
  Display-for-model convention. Revisit with the change form's FK widget (5.4/5.6).
- **No `choices` metadata anywhere in the ORM** (found 5.6.2): `FieldMeta` has no Django-style
  `choices=[...]` concept at all. Blocks choices-based `list_filter`, and would also improve
  the change form (a dropdown instead of a free-text/number input) and changelist display
  (label instead of raw stored value) if added. Needs its own design: macro attribute parsing,
  `FieldMeta` shape change, validation on save. Not scheduled to a specific Phase 5 sub-slice
  yet — whoever picks it up should design it once, not per-feature.
- **`on_delete` not enforced anywhere in the ORM** (pre-existing gap, made visible by 5.5):
  `RelationMeta.on_delete` (Cascade/Protect/SetNull/Restrict/DoNothing) is metadata only. 5.5's
  delete confirmation page *displays* the declared value next to each related-object count as
  information for the staff user, but a POST delete never checks it — what actually happens on
  a delete with dependents is whatever the real Postgres schema's FK constraint does, and since
  there's no real migration system yet (see below), that constraint may not even match the
  declared `on_delete` value. Real enforcement needs either DB-level FK constraints generated
  from `on_delete` (owed to the migrations work) or an ORM-level pre-delete check — don't build
  either without picking one deliberately; a half-enforced version is worse than none.

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
  hidden-input tokens unsupported. 5.4's admin add/change pages are real `<form method="post">`
  pages now, so this is live: a plain browser submission of those forms has no way to populate
  the `X-CSRFToken` header, and the middleware does not check a body field, so the POST will
  403 unless something (client-side JS reading the cookie, or an API client setting the header
  directly) supplies it. Deliberately not patched with a fake hidden `csrfmiddlewaretoken`
  input in 5.4 — that would look like protection without being checked. Revisit at 5.8
  (theming), when server-rendered forms become the primary POST surface.
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
