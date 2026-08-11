use super::*;

#[test]
fn parse_rate_accepts_drf_style_strings() {
    assert_eq!(
        parse_rate("100/hour"),
        Some((100, Duration::from_secs(3600)))
    );
    assert_eq!(parse_rate("10/minute"), Some((10, Duration::from_secs(60))));
    assert_eq!(parse_rate("5/day"), Some((5, Duration::from_secs(86400))));
    assert_eq!(parse_rate("2/second"), Some((2, Duration::from_secs(1))));
    // Abbreviations and plurals.
    assert_eq!(parse_rate("7/h"), Some((7, Duration::from_secs(3600))));
    assert_eq!(parse_rate("3/hours"), Some((3, Duration::from_secs(3600))));
}

#[test]
fn parse_rate_rejects_malformed_strings() {
    // A bad rate must not silently become some default budget.
    assert_eq!(parse_rate("100"), None);
    assert_eq!(parse_rate("100/fortnight"), None);
    assert_eq!(parse_rate("many/hour"), None);
    assert_eq!(parse_rate(""), None);
}

#[tokio::test]
async fn throttle_rejects_once_the_budget_is_exhausted() {
    use bytes::Bytes;
    use hyper::http::{HeaderMap, HeaderValue, Method, Uri};

    let store: Arc<dyn Cache> = Arc::new(djangors_cache::InMemoryCache::new(100));
    let throttle = Throttle::new("test_scope", "2/hour", store).unwrap();

    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", HeaderValue::from_static("10.1.2.3"));
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/things"),
        headers,
        Bytes::new(),
    );

    assert!(throttle.check(&req).await.is_ok());
    assert!(throttle.check(&req).await.is_ok());
    let err = throttle.check(&req).await.unwrap_err();
    assert!(
        matches!(err, DjangorsError::TooManyRequests(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn separate_scopes_do_not_share_a_budget() {
    use bytes::Bytes;
    use hyper::http::{HeaderMap, HeaderValue, Method, Uri};

    let store: Arc<dyn Cache> = Arc::new(djangors_cache::InMemoryCache::new(100));
    let a = Throttle::new("scope_a", "1/hour", store.clone()).unwrap();
    let b = Throttle::new("scope_b", "1/hour", store).unwrap();

    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", HeaderValue::from_static("10.1.2.4"));
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/things"),
        headers,
        Bytes::new(),
    );

    assert!(a.check(&req).await.is_ok());
    assert!(a.check(&req).await.is_err());
    // Same client, different scope: unaffected.
    assert!(b.check(&req).await.is_ok());
}
