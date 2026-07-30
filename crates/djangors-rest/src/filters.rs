//! Composable query-string filter backends for ViewSets.
//!
//! A [`FilterBackend`] turns query parameters into `QuerySet` constraints. The
//! built-in [`ViewSetConfig`](crate::viewsets::ViewSetConfig) allowlist only
//! ever produced exact matches (`?status=open`); these backends add the lookup
//! suffixes, free-text search, and ordering that DRF gets from `django-filter`
//! and its filter-backend stack.
//!
//! Backends are applied in order and each one narrows the queryset further, so
//! they compose:
//!
//! ```ignore
//! let backends: Vec<Arc<dyn FilterBackend<Article>>> = vec![
//!     Arc::new(FieldFilter::new(&["status", "author_id", "published_at"])),
//!     Arc::new(SearchFilter::new(&["title", "body"])),
//!     Arc::new(OrderingFilter::new(&["published_at", "title"])),
//! ];
//! ```
//!
//! Every backend is allowlist-driven: a parameter naming a field that was not
//! explicitly permitted is ignored, never passed through to SQL. That keeps a
//! client from filtering on columns the endpoint never meant to expose.

use std::sync::Arc;

use djangors_core::error::DjangorsError;
use djangors_core::request::Request;
use djangors_orm::error::FromRow;
use djangors_orm::expr::{UnresolvedCompare, UnresolvedExpr, Value};
use djangors_orm::meta::{FieldKind, Model};
use djangors_orm::queryset::QuerySet;

/// The lookup suffixes a client may attach to a filterable field.
///
/// Deliberately a fixed list rather than "whatever the ORM understands": the
/// suffix arrives from the query string, so it is untrusted input and gets
/// validated against this set before it is ever concatenated into a field name.
pub const ALLOWED_LOOKUPS: &[&str] = &[
    "eq",
    "ne",
    "lt",
    "lte",
    "gt",
    "gte",
    "contains",
    "icontains",
    "startswith",
    "endswith",
    "iexact",
    "in",
    "isnull",
];

/// Narrows a queryset based on the request's query parameters.
///
/// Generic over the model rather than over the method, so `Arc<dyn
/// FilterBackend<M>>` stays object-safe and a ViewSet can hold a
/// heterogeneous list of backends.
pub trait FilterBackend<M: Model + FromRow>: Send + Sync + 'static {
    /// Applies this backend's constraints to `qs`.
    fn filter_queryset(&self, req: &Request, qs: QuerySet<M>)
        -> Result<QuerySet<M>, DjangorsError>;
}

/// Applies every backend in `backends` to `qs`, in order.
pub fn apply_backends<M: Model + FromRow + 'static>(
    backends: &[Arc<dyn FilterBackend<M>>],
    req: &Request,
    mut qs: QuerySet<M>,
) -> Result<QuerySet<M>, DjangorsError> {
    for backend in backends {
        qs = backend.filter_queryset(req, qs)?;
    }
    Ok(qs)
}

/// Parses a query-string value into a [`Value`] matching the field's declared type.
///
/// Returns `None` when the text does not parse as the field's type, which the
/// callers treat as "ignore this parameter" rather than as an error — a
/// malformed filter should not fail an otherwise valid list request.
fn parse_typed<M: Model>(field_name: &str, raw: &str) -> Option<Value> {
    let meta = M::meta();
    let Some(field) = meta.fields.iter().find(|f| f.name == field_name) else {
        // Relations are stored as bigint ids.
        return raw.parse::<i64>().ok().map(Value::I64);
    };
    match field.kind {
        FieldKind::Integer | FieldKind::BigInt => raw.parse::<i64>().ok().map(Value::I64),
        FieldKind::Float => raw.parse::<f64>().ok().map(Value::F64),
        FieldKind::Boolean => match raw {
            "true" | "1" => Some(Value::Bool(true)),
            "false" | "0" => Some(Value::Bool(false)),
            _ => None,
        },
        FieldKind::DateTime => {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
                Some(Value::DateTime(dt.with_timezone(&chrono::Utc)))
            } else {
                chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                    .ok()
                    .map(|naive| {
                        Value::DateTime(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                            naive,
                            chrono::Utc,
                        ))
                    })
            }
        }
        _ => Some(Value::Text(raw.to_string())),
    }
}

/// Field filtering with Django-style lookup suffixes.
///
/// Reads `?field=value` and `?field__lookup=value` for every allowlisted field,
/// e.g. `?age__gte=18`, `?status__in=open,pending`, `?name__icontains=ada`,
/// `?deleted_at__isnull=true`.
///
/// The field list is `&'static [&'static str]` because the ORM's expression
/// layer stores field names as `&'static str`; that is also what makes it
/// impossible for a client-supplied name to reach SQL.
pub struct FieldFilter {
    fields: &'static [&'static str],
}

impl FieldFilter {
    /// Allows filtering on each name in `fields`.
    pub fn new(fields: &'static [&'static str]) -> Self {
        Self { fields }
    }

    /// The full parameter name for a field/lookup pair, matched against the
    /// query string. Returns the `&'static str` field name so the resulting
    /// expression borrows from the allowlist, never from request input.
    fn candidates(field: &'static str) -> impl Iterator<Item = (String, &'static str)> {
        std::iter::once((field.to_string(), field)).chain(
            ALLOWED_LOOKUPS
                .iter()
                .map(move |lookup| (format!("{}__{}", field, lookup), field)),
        )
    }
}

impl<M: Model + FromRow> FilterBackend<M> for FieldFilter {
    fn filter_queryset(
        &self,
        req: &Request,
        mut qs: QuerySet<M>,
    ) -> Result<QuerySet<M>, DjangorsError> {
        for &field in self.fields {
            for (param, base) in Self::candidates(field) {
                let Some(raw) = req.query(&param) else {
                    continue;
                };

                let lookup = param.rsplit_once("__").map(|(_, l)| l).unwrap_or("eq");
                let is_lookup_param = param != base;

                // `__in` takes a comma-separated list; everything else is a
                // single typed scalar.
                let value = if is_lookup_param && lookup == "in" {
                    let items: Vec<Value> = raw
                        .split(',')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .filter_map(|s| parse_typed::<M>(base, s))
                        .collect();
                    Value::List(items)
                } else if is_lookup_param && lookup == "isnull" {
                    Value::Bool(!matches!(raw, "false" | "0"))
                } else if is_lookup_param
                    && matches!(
                        lookup,
                        "contains" | "icontains" | "startswith" | "endswith" | "iexact"
                    )
                {
                    // Substring lookups are always textual, even against a
                    // non-text column.
                    Value::Text(raw.to_string())
                } else {
                    match parse_typed::<M>(base, raw) {
                        Some(v) => v,
                        None => continue,
                    }
                };

                // Rebuild the lookup name from the allowlist, not from the
                // query string, so only a `&'static str` reaches the ORM.
                let field_expr: &'static str = if is_lookup_param {
                    match ALLOWED_LOOKUPS.iter().find(|l| **l == lookup) {
                        Some(l) => leak_lookup(base, l),
                        None => continue,
                    }
                } else {
                    base
                };

                let cmp = UnresolvedExpr::And(vec![UnresolvedCompare {
                    field: field_expr,
                    value,
                }]);
                qs = qs
                    .filter(cmp)
                    .map_err(|e| DjangorsError::BadRequest(e.to_string()))?;
            }
        }
        Ok(qs)
    }
}

/// Interns `"{field}__{lookup}"` so it can be handed to the ORM as a
/// `&'static str`.
///
/// Both inputs come from compile-time allowlists ([`FieldFilter::fields`] and
/// [`ALLOWED_LOOKUPS`]), so the set of distinct strings this can produce is
/// bounded by the program's own configuration — it cannot be grown by request
/// traffic. A cache keeps repeated requests from leaking on every call.
fn leak_lookup(field: &'static str, lookup: &'static str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<(&'static str, &'static str), &'static str>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = guard.get(&(field, lookup)) {
        return existing;
    }
    let leaked: &'static str = Box::leak(format!("{}__{}", field, lookup).into_boxed_str());
    guard.insert((field, lookup), leaked);
    leaked
}

/// Free-text search across several text fields — DRF's `SearchFilter`.
///
/// Reads `?search=term` and matches it case-insensitively against every
/// configured field, OR'd together.
pub struct SearchFilter {
    fields: &'static [&'static str],
    param: &'static str,
}

impl SearchFilter {
    /// Searches `fields` using the default `?search=` parameter.
    pub fn new(fields: &'static [&'static str]) -> Self {
        Self {
            fields,
            param: "search",
        }
    }

    /// Overrides the query-parameter name.
    pub fn with_param(mut self, param: &'static str) -> Self {
        self.param = param;
        self
    }
}

impl<M: Model + FromRow> FilterBackend<M> for SearchFilter {
    fn filter_queryset(
        &self,
        req: &Request,
        qs: QuerySet<M>,
    ) -> Result<QuerySet<M>, DjangorsError> {
        let Some(term) = req.query(self.param).filter(|t| !t.is_empty()) else {
            return Ok(qs);
        };
        if self.fields.is_empty() {
            return Ok(qs);
        }
        qs.filter_or_icontains(self.fields, term)
            .map_err(|e| DjangorsError::BadRequest(e.to_string()))
    }
}

/// Client-controlled ordering restricted to an allowlist — DRF's `OrderingFilter`.
///
/// Reads `?ordering=field` or `?ordering=-field`, comma-separated for multiple
/// keys. Names outside the allowlist are ignored.
pub struct OrderingFilter {
    fields: &'static [&'static str],
    param: &'static str,
}

impl OrderingFilter {
    /// Permits ordering by each name in `fields`.
    pub fn new(fields: &'static [&'static str]) -> Self {
        Self {
            fields,
            param: "ordering",
        }
    }

    /// Overrides the query-parameter name.
    pub fn with_param(mut self, param: &'static str) -> Self {
        self.param = param;
        self
    }
}

impl<M: Model + FromRow> FilterBackend<M> for OrderingFilter {
    fn filter_queryset(
        &self,
        req: &Request,
        mut qs: QuerySet<M>,
    ) -> Result<QuerySet<M>, DjangorsError> {
        let Some(raw) = req.query(self.param) else {
            return Ok(qs);
        };
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let descending = part.starts_with('-');
            let clean = part.strip_prefix('-').unwrap_or(part);
            let Some(&allowed) = self.fields.iter().find(|f| **f == clean) else {
                continue;
            };
            // Rebuild from the allowlist so the ORM never sees request text.
            let spec: &'static str = if descending {
                desc_of(allowed)
            } else {
                allowed
            };
            qs = qs
                .order_by(spec)
                .map_err(|e| DjangorsError::BadRequest(e.to_string()))?;
        }
        Ok(qs)
    }
}

/// Interns `"-{field}"` for descending ordering, for the same reason as
/// [`leak_lookup`]: the ORM takes `&'static str`, and the input is a
/// compile-time allowlist entry.
fn desc_of(field: &'static str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<&'static str, &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = guard.get(field) {
        return existing;
    }
    let leaked: &'static str = Box::leak(format!("-{}", field).into_boxed_str());
    guard.insert(field, leaked);
    leaked
}
