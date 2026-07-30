//! DRF-style request throttling for API endpoints.
//!
//! Builds on [`djangors_core::ratelimit`], which already provides the
//! cache-backed sliding window and the `429` error. What this module adds is
//! the DRF ergonomics: rate strings like `"100/hour"`, an identity that prefers
//! the authenticated user over the client IP, and a [`Throttle`] that a ViewSet
//! can hold and apply to every action.
//!
//! ```ignore
//! let throttle = Throttle::new("articles", "100/hour", cache).unwrap();
//! let options = ViewSetOptions::new(config).with_throttle(throttle);
//! ```
//!
//! Accounting is best-effort under concurrency for the same reason as the
//! underlying limiter: the cache API has no atomic increment.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use djangors_cache::Cache;
use djangors_core::error::DjangorsError;
use djangors_core::ratelimit::{ByIp, RateLimitKey, RateLimiter};
use djangors_core::request::Request;

/// Parses a DRF-style rate string such as `"100/hour"` into a count and window.
///
/// Accepts `second`, `minute`, `hour`, and `day`, along with their `s`/`m`/`h`/`d`
/// abbreviations and plural forms. Returns `None` for anything else, so a
/// malformed rate is a configuration error the caller must handle rather than a
/// silently-applied default.
pub fn parse_rate(rate: &str) -> Option<(u32, Duration)> {
    let (count, period) = rate.split_once('/')?;
    let count: u32 = count.trim().parse().ok()?;

    let period = period.trim();
    // Strip one trailing plural "s", but only when something remains, so the
    // "s" abbreviation for seconds survives.
    let period = match period.strip_suffix('s') {
        Some(singular) if !singular.is_empty() => singular,
        _ => period,
    };

    let window = match period {
        "s" | "sec" | "second" => Duration::from_secs(1),
        "m" | "min" | "minute" => Duration::from_secs(60),
        "h" | "hr" | "hour" => Duration::from_secs(60 * 60),
        "d" | "day" => Duration::from_secs(24 * 60 * 60),
        _ => return None,
    };
    Some((count, window))
}

/// Keys by authenticated user when there is one, falling back to client IP.
///
/// This is DRF's `UserRateThrottle` and `AnonRateThrottle` collapsed into a
/// single strategy: an anonymous request is still counted (by IP) rather than
/// being rejected outright, which is what a bare by-user key would do.
pub struct ByUserOrIp;

#[async_trait]
impl RateLimitKey for ByUserOrIp {
    async fn key(&self, req: &Request) -> Result<String, DjangorsError> {
        if let Some(user) = crate::permissions::current_user(req).await {
            return Ok(format!("user:{}", user.id));
        }
        let ip = ByIp.key(req).await?;
        Ok(format!("anon:{}", ip))
    }
}

/// A configured throttle that a ViewSet applies to each action.
///
/// Cloneable and cheap to clone: the underlying limiter is shared, so every
/// clone counts against the same budget.
#[derive(Clone)]
pub struct Throttle {
    limiter: Arc<RateLimiter<ByUserOrIp>>,
}

impl Throttle {
    /// Builds a throttle named `scope` at `rate` (e.g. `"100/hour"`), counting
    /// per authenticated user and falling back to client IP.
    ///
    /// The `scope` isolates this budget from every other throttle sharing the
    /// same cache, so two endpoints at the same rate do not consume each
    /// other's allowance.
    ///
    /// Returns `None` when `rate` is not a valid rate string.
    pub fn new(scope: &'static str, rate: &str, store: Arc<dyn Cache>) -> Option<Self> {
        let (max_attempts, window) = parse_rate(rate)?;
        Some(Self {
            limiter: Arc::new(RateLimiter::new(
                scope,
                ByUserOrIp,
                max_attempts,
                window,
                store,
            )),
        })
    }

    /// Records one request, returning
    /// [`DjangorsError::TooManyRequests`] when the budget is exhausted.
    pub async fn check(&self, req: &Request) -> Result<(), DjangorsError> {
        self.limiter.check(req).await
    }
}

#[cfg(test)]
mod tests {
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
}
