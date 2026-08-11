use super::*;
use crate::response::Response as CoreResponse;
use crate::router::Router;
use bytes::Bytes;
use hyper::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use std::str::FromStr;

fn req_from_ip(ip: &str) -> Request {
    req_from_ip_and_path(ip, "/anything")
}

fn req_from_ip_and_path(ip: &str, path: &str) -> Request {
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", HeaderValue::from_str(ip).unwrap());
    Request::new(
        Method::GET,
        Uri::from_str(path).unwrap(),
        headers,
        Bytes::new(),
    )
}

#[tokio::test]
async fn test_rate_limiter_never_allows_unbounded_requests_under_concurrency() {
    let store: Arc<dyn Cache> = Arc::new(djangors_cache::InMemoryCache::new(1000));
    let limiter = Arc::new(RateLimiter::new(
        "concurrency_test",
        ByIp,
        5,
        Duration::from_secs(60),
        store,
    ));

    let mut handles = Vec::new();
    for _ in 0..20 {
        let limiter = limiter.clone();
        handles.push(tokio::spawn(async move {
            let req = req_from_ip("10.0.0.1");
            limiter.check(&req).await.is_ok()
        }));
    }
    let mut allowed = 0;
    for h in handles {
        if h.await.unwrap() {
            allowed += 1;
        }
    }
    // Best-effort under concurrency (no atomic increment): we don't assert an exact count,
    // but a broken implementation that always reads a stale zero would let all 20 through.
    assert!(
        allowed < 20,
        "20 concurrent requests against max_attempts=5 must not all be allowed \
             (got {allowed} allowed, which would mean the limiter never actually limits)"
    );
    assert!(
        allowed >= 5,
        "at least max_attempts requests should be allowed (got {allowed})"
    );
}

#[tokio::test]
async fn test_rate_limiter_scoping_is_isolated_by_name() {
    let store: Arc<dyn Cache> = Arc::new(djangors_cache::InMemoryCache::new(1000));
    let limiter_a = RateLimiter::new(
        "endpoint_a",
        ByIp,
        2,
        Duration::from_secs(60),
        store.clone(),
    );
    let limiter_b = RateLimiter::new("endpoint_b", ByIp, 2, Duration::from_secs(60), store);

    let req = req_from_ip("10.0.0.2");
    // Exhaust limiter_a completely.
    assert!(limiter_a.check(&req).await.is_ok());
    assert!(limiter_a.check(&req).await.is_ok());
    assert!(limiter_a.check(&req).await.is_err());

    // limiter_b, same underlying key (same IP), different name: must be completely
    // unaffected by limiter_a's exhausted budget.
    assert!(limiter_b.check(&req).await.is_ok());
    assert!(limiter_b.check(&req).await.is_ok());
    assert!(limiter_b.check(&req).await.is_err());
}

#[tokio::test]
async fn test_rate_limiter_window_expiry_against_real_elapsed_time() {
    let store: Arc<dyn Cache> = Arc::new(djangors_cache::InMemoryCache::new(1000));
    let limiter = RateLimiter::new("expiry_test", ByIp, 2, Duration::from_millis(200), store);
    let req = req_from_ip("10.0.0.3");

    assert!(limiter.check(&req).await.is_ok());
    assert!(limiter.check(&req).await.is_ok());
    assert!(limiter.check(&req).await.is_err());

    tokio::time::sleep(Duration::from_millis(250)).await;

    assert!(
        limiter.check(&req).await.is_ok(),
        "a new request after the window has elapsed must succeed"
    );
}

async fn ok_handler(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(CoreResponse::text(StatusCode::OK, "ok"))
}

#[tokio::test]
async fn test_rate_limited_route_returns_429_over_the_real_dispatch_path() {
    let store: Arc<dyn Cache> = Arc::new(djangors_cache::InMemoryCache::new(1000));
    let limiter = Arc::new(RateLimiter::new(
        "route_test",
        ByIp,
        1,
        Duration::from_secs(60),
        store,
    ));
    let router = Router::new().get("/limited", rate_limited(limiter, ok_handler));

    let req1 = req_from_ip_and_path("10.0.0.4", "/limited");
    let res1 = router.handle(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);

    let req2 = req_from_ip_and_path("10.0.0.4", "/limited");
    let res2 = router.handle(req2).await;
    assert!(matches!(res2, Err(DjangorsError::TooManyRequests(_))));
    if let Err(e) = res2 {
        assert_eq!(e.status_code(), StatusCode::TOO_MANY_REQUESTS);
    }
}
