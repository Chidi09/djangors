# REST Framework

`djangors-rest` is the API layer: ViewSets that turn a model into CRUD routes,
serializers that shape request and response bodies, and pluggable pagination,
filtering, permissions, and throttling. It is closely modelled on Django REST
Framework.

## A minimal ViewSet

`viewset_routes` mounts the five standard actions for a model. Routes require an
authenticated user by default; opt out explicitly with
`viewset_routes_with_permission` and `AllowAny`.

| Method | Path | Action |
| --- | --- | --- |
| `GET` | `/` | `list` |
| `POST` | `/` | `create` |
| `GET` | `/{pk}` | `retrieve` |
| `PUT` / `PATCH` | `/{pk}` | `update` |
| `DELETE` | `/{pk}` | `destroy` |

`PATCH` is a genuine partial update: fields the body omits are left untouched
rather than reset to their defaults.

## Serializers

A serializer decides which fields are readable, which are writable, and what
counts as valid. `ModelSerializer` derives all of that from the model's
metadata; `FieldSet` narrows it.

```rust,compile
# use djangors_rest::{FieldSet, ModelSerializer};
# use polls::models::Question;
# fn main() {
// Expose everything except `pub_date`, and never accept `id` on a write.
let serializer = ModelSerializer::<Question>::new(
    FieldSet::all().excluding(&["pub_date"]).read_only(&["id"]),
);
# let _ = serializer;
# }
```

- **`read_only`** fields are rendered but rejected on write, rather than
  silently ignored.
- **`write_only`** fields (passwords, tokens) are accepted on write and never
  appear in a response.

### Validation

Field errors accumulate into `ValidationErrors` and render as a `422` whose
`details` carries the `{field: [messages]}` map, so a client sees every problem
at once instead of one per round trip.

```rust,compile
# use djangors_rest::ValidationErrors;
# fn main() {
let mut errors = ValidationErrors::new();
errors.add("question_text", "This field may not be blank.");
errors.add_non_field("Either a question or a choice is required.");
# let _ = errors;
# }
```

### Nested serializers

Because the serializer trait is synchronous it never issues queries itself.
Load the relation with `select_related`, then render it with
`NestedSerializer`, which embeds the related object in place of the raw foreign
key. When no related instance is supplied the field keeps its id, so a missing
join degrades to the flat representation rather than to `null`.

```rust,compile
# use djangors_rest::{ModelSerializer, NestedSerializer};
# use polls::models::{Choice, Question};
# fn main() {
let serializer = NestedSerializer::new(
    ModelSerializer::<Choice>::default(),
    "question",
    ModelSerializer::<Question>::default(),
);
# let _ = serializer;
# }
```

## Pagination

Three strategies ship, all behind the `Pagination` trait:

- **`PageNumberPagination`** — `?page=2`, the default.
- **`LimitOffsetPagination`** — `?limit=20&offset=40`.
- **`CursorPagination`** — opaque cursor, stable under concurrent inserts and
  issues no `COUNT`.

Page size is server-controlled by default. Clients may only override it with
`?page_size=` when `max_page_size` opts in, and the value is clamped to that
cap.

## Filter backends

`ViewSetConfig::filterable_fields` gives exact-match `?field=value` filtering.
Filter backends add lookup suffixes, free-text search, and client-controlled
ordering. They run in order and each one narrows the queryset further.

```rust,compile
# use djangors_rest::{FieldFilter, OrderingFilter, SearchFilter, ViewSetConfig, ViewSetOptions};
# use polls::models::Question;
# fn main() {
let options = ViewSetOptions::<Question>::new(ViewSetConfig::default())
    .with_filter_backend(FieldFilter::new(&["id", "pub_date"]))
    .with_filter_backend(SearchFilter::new(&["question_text"]))
    .with_filter_backend(OrderingFilter::new(&["pub_date"]));
# let _ = options;
# }
```

That accepts requests such as:

```text
?pub_date__gte=2026-01-01T00:00:00Z
?id__in=1,2,3
?question_text__icontains=rust
?search=rust
?ordering=-pub_date
```

Every backend is allowlist-driven. A parameter naming a field that was not
explicitly permitted is ignored rather than passed to SQL, and an unrecognised
lookup suffix is dropped the same way — a client cannot filter on a column the
endpoint never meant to expose.

## Permissions

`AllowAny`, `IsAuthenticated`, `IsStaff`, `IsSuperuser`, and `IsReadOnly` ship
built in, and combine with `and` / `or` / `negate`:

```rust,compile
# use djangors_rest::{IsReadOnly, IsStaff, PermissionExt};
# fn main() {
// Staff may write; everyone else is limited to safe methods.
let permission = IsStaff.or(IsReadOnly);
# let _ = permission;
# }
```

## Throttling

`Throttle` applies a per-user (falling back to per-IP) rate limit to every
action on a ViewSet, using DRF's rate strings. It is backed by the same
cache-backed sliding window as `djangors_core::ratelimit`, so accounting is
best-effort under concurrency — the cache API has no atomic increment.

```rust,compile
# use std::sync::Arc;
# use djangors_cache::{Cache, InMemoryCache};
# use djangors_rest::Throttle;
# fn main() {
let store: Arc<dyn Cache> = Arc::new(InMemoryCache::new(10_000));
let throttle = Throttle::new("questions", "100/hour", store).expect("valid rate");
# let _ = throttle;
# }
```

`parse_rate` accepts `second`, `minute`, `hour`, and `day` with their
abbreviations and plurals. A malformed rate returns `None` rather than silently
falling back to some default budget, so a typo is a configuration error you
catch at startup.

The `scope` (the first argument) isolates the budget: two endpoints at the same
rate sharing one cache do not consume each other's allowance.

## Error responses

Errors render as JSON when the request's `Accept` header asks for it, using a
stable envelope:

```json
{
  "error": {
    "code": "validation_error",
    "message": "Validation failed",
    "details": { "question_text": ["This field may not be blank."] }
  }
}
```

`code` is a stable, machine-readable string; `details` is omitted entirely when
there is nothing to report. Build one directly with
`DjangorsError::api(status, code, message)` and attach a payload with
`.with_details(json)`, or convert at a `?` site with the `ApiResultExt` helpers.
