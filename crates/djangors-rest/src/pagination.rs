//! Pluggable pagination strategies for list endpoints.
//!
//! Pagination used to be fixed: a hardcoded 100-row page, with cursor mode as a
//! boolean opt-in and no way to change the envelope. [`Pagination`] makes the
//! strategy a choice — page-number, limit/offset, or cursor — and lets a
//! project supply its own without touching the ViewSet.

use djangors_core::pagination::Paginator;
use djangors_core::request::Request;

/// Default page size for REST ViewSet list pagination.
/// Matches the admin's per-page convention (100).
pub const REST_PER_PAGE: i64 = 100;

/// How many rows to fetch, and from where.
///
/// A strategy turns the request's query string into this, and the ViewSet
/// turns it into `LIMIT`/`OFFSET`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSlice {
    /// Maximum rows to return.
    pub limit: i64,
    /// Rows to skip.
    pub offset: i64,
}

/// A pagination strategy: it decides the window, then shapes the response body.
pub trait Pagination: Send + Sync + 'static {
    /// The window of rows this request should receive.
    fn slice(&self, req: &Request, total: i64) -> PageSlice;

    /// Build the response envelope around the already-serialized rows.
    fn envelope(
        &self,
        req: &Request,
        total: i64,
        results: Vec<serde_json::Value>,
    ) -> serde_json::Value;

    /// Rows per page for this request.
    ///
    /// ViewSets need the size before they have a row count (the cursor path
    /// never issues a `COUNT`), so this must not be derived from
    /// [`Pagination::slice`] — passing a sentinel total there overflows any
    /// strategy that computes a page count.
    fn page_size(&self, req: &Request) -> i64;
}

/// Reads `?page=`, and reports `count` / `page` / `total_pages` alongside the
/// rows. This is the historical Djangors behaviour and remains the default.
#[derive(Debug, Clone)]
pub struct PageNumberPagination {
    /// Rows per page when the client does not (or may not) choose.
    pub page_size: i64,
    /// When set, `?page_size=` is honoured, clamped to this ceiling.
    pub max_page_size: Option<i64>,
}

impl Default for PageNumberPagination {
    fn default() -> Self {
        Self {
            page_size: REST_PER_PAGE,
            max_page_size: None,
        }
    }
}

/// Resolve a client-supplied size against a default and an optional ceiling.
///
/// Unparseable or out-of-range values fall back to the default rather than
/// erroring: a bad page size should not fail an otherwise valid list request.
fn resolve_size(req: &Request, default: i64, max: Option<i64>) -> i64 {
    let default = default.max(1);
    let Some(max) = max else {
        return default;
    };
    req.query("page_size")
        .and_then(|raw| raw.parse::<i64>().ok())
        .filter(|n| *n >= 1)
        .map(|n| n.min(max))
        .unwrap_or(default)
}

/// Read `?page=`, clamped to at least 1.
pub fn requested_page(req: &Request) -> i64 {
    req.query("page")
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or(1)
        .max(1)
}

impl Pagination for PageNumberPagination {
    fn page_size(&self, req: &Request) -> i64 {
        resolve_size(req, self.page_size, self.max_page_size)
    }

    fn slice(&self, req: &Request, total: i64) -> PageSlice {
        let limit = resolve_size(req, self.page_size, self.max_page_size);
        let paginator = Paginator::new(total, limit);
        PageSlice {
            limit,
            offset: paginator.offset(requested_page(req)),
        }
    }

    fn envelope(
        &self,
        req: &Request,
        total: i64,
        results: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        let limit = resolve_size(req, self.page_size, self.max_page_size);
        let paginator = Paginator::new(total, limit);
        serde_json::json!({
            "count": total,
            "page": requested_page(req),
            "total_pages": paginator.total_pages(),
            "results": results,
        })
    }
}

/// Reads `?limit=` and `?offset=` directly, and reports `count` / `limit` /
/// `offset`. Suited to clients that page by absolute position.
#[derive(Debug, Clone)]
pub struct LimitOffsetPagination {
    /// Rows per page when `?limit=` is absent.
    pub default_limit: i64,
    /// Ceiling applied to `?limit=`.
    pub max_limit: i64,
}

impl Default for LimitOffsetPagination {
    fn default() -> Self {
        Self {
            default_limit: REST_PER_PAGE,
            max_limit: REST_PER_PAGE,
        }
    }
}

impl LimitOffsetPagination {
    fn resolved(&self, req: &Request) -> PageSlice {
        let limit = req
            .query("limit")
            .and_then(|raw| raw.parse::<i64>().ok())
            .filter(|n| *n >= 1)
            .map(|n| n.min(self.max_limit.max(1)))
            .unwrap_or(self.default_limit.max(1));
        let offset = req
            .query("offset")
            .and_then(|raw| raw.parse::<i64>().ok())
            .filter(|n| *n >= 0)
            .unwrap_or(0);
        PageSlice { limit, offset }
    }
}

impl Pagination for LimitOffsetPagination {
    fn page_size(&self, req: &Request) -> i64 {
        self.resolved(req).limit
    }

    fn slice(&self, req: &Request, _total: i64) -> PageSlice {
        self.resolved(req)
    }

    fn envelope(
        &self,
        req: &Request,
        total: i64,
        results: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        let slice = self.resolved(req);
        serde_json::json!({
            "count": total,
            "limit": slice.limit,
            "offset": slice.offset,
            "results": results,
        })
    }
}

/// Keyset pagination over an ordered field, reporting `next_cursor` /
/// `previous_cursor`. Stable under concurrent inserts, unlike offset paging.
///
/// The ViewSet drives the actual keyset query (it needs the ordering field and
/// primary key); this strategy supplies the page size and the envelope.
#[derive(Debug, Clone)]
pub struct CursorPagination {
    /// Rows per page.
    pub page_size: i64,
    /// When set, `?page_size=` is honoured, clamped to this ceiling.
    pub max_page_size: Option<i64>,
}

impl Default for CursorPagination {
    fn default() -> Self {
        Self {
            page_size: REST_PER_PAGE,
            max_page_size: None,
        }
    }
}

impl CursorPagination {
    /// Build the cursor envelope, given the cursor the ViewSet computed.
    pub fn envelope_with_cursor(
        &self,
        total: i64,
        results: Vec<serde_json::Value>,
        next_cursor: Option<String>,
    ) -> serde_json::Value {
        serde_json::json!({
            "count": total,
            "results": results,
            "next_cursor": next_cursor,
            "previous_cursor": serde_json::Value::Null,
        })
    }
}

impl Pagination for CursorPagination {
    fn page_size(&self, req: &Request) -> i64 {
        resolve_size(req, self.page_size, self.max_page_size)
    }

    fn slice(&self, req: &Request, _total: i64) -> PageSlice {
        PageSlice {
            limit: resolve_size(req, self.page_size, self.max_page_size),
            offset: 0,
        }
    }

    fn envelope(
        &self,
        _req: &Request,
        total: i64,
        results: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        self.envelope_with_cursor(total, results, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use hyper::http::{HeaderMap, Method, Uri};

    fn req(query: &str) -> Request {
        let uri: Uri = format!("/items?{query}").parse().unwrap();
        Request::new(Method::GET, uri, HeaderMap::new(), Bytes::new())
    }

    #[test]
    fn page_number_computes_offset_from_page() {
        let p = PageNumberPagination::default();
        assert_eq!(p.slice(&req("page=3"), 1000).offset, 200);
        assert_eq!(p.slice(&req("page=1"), 1000).offset, 0);
        // Page 0 and negatives clamp to the first page rather than erroring.
        assert_eq!(p.slice(&req("page=0"), 1000).offset, 0);
        assert_eq!(p.slice(&req("page=-5"), 1000).offset, 0);
    }

    #[test]
    fn page_size_is_ignored_unless_max_page_size_opts_in() {
        let fixed = PageNumberPagination::default();
        assert_eq!(fixed.slice(&req("page_size=5"), 100).limit, REST_PER_PAGE);

        let flexible = PageNumberPagination {
            page_size: 20,
            max_page_size: Some(50),
        };
        assert_eq!(flexible.slice(&req("page_size=5"), 100).limit, 5);
        // Clamped to the ceiling, not rejected.
        assert_eq!(flexible.slice(&req("page_size=999"), 100).limit, 50);
        // Garbage falls back to the default.
        assert_eq!(flexible.slice(&req("page_size=abc"), 100).limit, 20);
        assert_eq!(flexible.slice(&req("page_size=0"), 100).limit, 20);
    }

    #[test]
    fn limit_offset_reads_both_params_and_clamps_limit() {
        let p = LimitOffsetPagination {
            default_limit: 25,
            max_limit: 100,
        };
        assert_eq!(
            p.slice(&req("limit=10&offset=30"), 500),
            PageSlice {
                limit: 10,
                offset: 30
            }
        );
        assert_eq!(p.slice(&req("limit=9999"), 500).limit, 100);
        assert_eq!(p.slice(&req(""), 500).limit, 25);
        assert_eq!(p.slice(&req("offset=-3"), 500).offset, 0);
    }

    #[test]
    fn envelopes_carry_the_shape_each_strategy_promises() {
        let rows = vec![serde_json::json!({"id": 1})];

        let page = PageNumberPagination::default().envelope(&req("page=2"), 150, rows.clone());
        assert_eq!(page["count"], 150);
        assert_eq!(page["page"], 2);
        assert_eq!(page["total_pages"], 2);

        let lo =
            LimitOffsetPagination::default().envelope(&req("limit=10&offset=5"), 150, rows.clone());
        assert_eq!(lo["limit"], 10);
        assert_eq!(lo["offset"], 5);

        let cursor =
            CursorPagination::default().envelope_with_cursor(150, rows, Some("abc".to_string()));
        assert_eq!(cursor["next_cursor"], "abc");
        assert!(cursor["previous_cursor"].is_null());
    }
}
