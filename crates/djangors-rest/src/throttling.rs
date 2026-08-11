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
#[path = "throttling_tests.rs"]
mod tests;
