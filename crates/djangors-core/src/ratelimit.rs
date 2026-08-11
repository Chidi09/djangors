//! Opt-in, named endpoint rate limiting.
//!
//! Cache check-and-record is necessarily read-then-write because the cache API has no
//! atomic increment. It is best effort under concurrency: it does not promise exact
//! accounting at a concurrency boundary, but a normal implementation cannot allow an
//! unbounded stream by repeatedly treating every request as a fresh zero.

use crate::{DjangorsError, Handler, PathParams, Request, Response};
use async_trait::async_trait;
use djangors_cache::Cache;
use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Derives the identity counted by a rate limiter.
///
/// Returns `Err` (propagated directly by [`RateLimiter::check`]) when this request has no
/// derivable identity for the strategy in question — e.g. an unauthenticated request under a
/// by-user strategy — rather than falling back to some shared/empty key.
#[async_trait]
pub trait RateLimitKey: Send + Sync {
    /// Derive a stable key for this request.
    async fn key(&self, req: &Request) -> Result<String, DjangorsError>;
}

/// Keys by proxy-supplied IP headers. This is not secure unless a trusted reverse proxy
/// sets and strips these headers; otherwise clients can spoof them.
pub struct ByIp;
#[async_trait]
impl RateLimitKey for ByIp {
    async fn key(&self, req: &Request) -> Result<String, DjangorsError> {
        Ok(req
            .header("x-forwarded-for")
            .or_else(|| req.header("x-real-ip"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .split(',')
            .next()
            .unwrap_or("unknown")
            .trim()
            .to_owned())
    }
}

/// A named, scoped, cache-backed endpoint limiter.
pub struct RateLimiter<K: RateLimitKey> {
    name: &'static str,
    key_fn: K,
    max_attempts: u32,
    window: Duration,
    store: Arc<dyn Cache>,
}
impl<K: RateLimitKey> RateLimiter<K> {
    /// Creates a limiter.
    pub fn new(
        name: &'static str,
        key_fn: K,
        max_attempts: u32,
        window: Duration,
        store: Arc<dyn Cache>,
    ) -> Self {
        Self {
            name,
            key_fn,
            max_attempts,
            window,
            store,
        }
    }
    /// Checks and records one attempt.
    pub async fn check(&self, req: &Request) -> Result<(), DjangorsError> {
        let identity = self.key_fn.key(req).await?;
        let key = format!("ratelimit:{}:{}", self.name, identity);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let state = self
            .store
            .get(&key)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .map(|v| {
                serde_json::from_slice::<(u32, u64)>(&v)
                    .map_err(|e| DjangorsError::Internal(e.to_string()))
            })
            .transpose()?;
        let (count, start) = state
            .filter(|(_, s)| now.saturating_sub(*s) < self.window.as_millis() as u64)
            .unwrap_or((0, now));
        if count >= self.max_attempts {
            return Err(DjangorsError::TooManyRequests("rate limit exceeded".into()));
        }
        self.store
            .set(
                &key,
                serde_json::to_vec(&(count + 1, start)).unwrap(),
                Some(self.window),
            )
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))
    }
}

/// Wraps one handler with an explicitly configured limiter.
pub fn rate_limited<K, F, Fut>(limiter: Arc<RateLimiter<K>>, handler: F) -> impl Handler
where
    K: RateLimitKey + 'static,
    F: Fn(Request, PathParams) -> Fut + Copy + Send + Sync + 'static,
    Fut: Future<Output = Result<Response, DjangorsError>> + Send + 'static,
{
    move |req, params| {
        let limiter = limiter.clone();
        async move {
            limiter.check(&req).await?;
            handler(req, params).await
        }
    }
}

#[cfg(test)]
#[path = "ratelimit_tests.rs"]
mod tests;
