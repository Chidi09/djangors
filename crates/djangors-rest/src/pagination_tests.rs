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
