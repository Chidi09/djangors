//! Raw-byte caches, common backends, and explicitly opted-in response caching.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use http_body_util::Full;
use hyper::{Request, Response};
use moka::future::Cache as MokaCache;
use serde::{de::DeserializeOwned, Serialize};
use std::{convert::Infallible, future::Future, sync::Arc, time::Duration};
use thiserror::Error;
use tower::{Layer, Service};

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("cache backend error: {0}")]
    Backend(String),
    #[error("cache serialization error: {0}")]
    Serialization(String),
}

/// An object-safe cache of raw byte values.
#[async_trait]
pub trait Cache: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError>;
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>)
        -> Result<(), CacheError>;
    async fn delete(&self, key: &str) -> Result<(), CacheError>;
}

#[async_trait]
pub trait CacheExt: Cache {
    async fn get_or_set<F, Fut>(
        &self,
        key: &str,
        ttl: Option<Duration>,
        f: F,
    ) -> Result<Vec<u8>, CacheError>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<Vec<u8>, CacheError>> + Send,
    {
        if let Some(value) = self.get(key).await? {
            return Ok(value);
        }
        let value = f().await?;
        self.set(key, value.clone(), ttl).await?;
        Ok(value)
    }

    async fn get_or_set_json<T, F, Fut>(
        &self,
        key: &str,
        ttl: Option<Duration>,
        f: F,
    ) -> Result<T, CacheError>
    where
        T: Serialize + DeserializeOwned + Send,
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, CacheError>> + Send,
    {
        let bytes = self
            .get_or_set(key, ttl, || async {
                let value = f().await?;
                serde_json::to_vec(&value).map_err(|e| CacheError::Serialization(e.to_string()))
            })
            .await?;
        serde_json::from_slice(&bytes).map_err(|e| CacheError::Serialization(e.to_string()))
    }
}
impl<T: Cache + ?Sized> CacheExt for T {}

pub async fn get_or_set_fragment<F, Fut>(
    cache: &dyn Cache,
    key: &str,
    ttl: Option<Duration>,
    f: F,
) -> Result<Vec<u8>, CacheError>
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<Vec<u8>, CacheError>> + Send,
{
    cache.get_or_set(key, ttl, f).await
}

#[derive(Clone)]
pub struct InMemoryCache {
    inner: MokaCache<String, MemoryEntry>,
}
#[derive(Clone)]
struct MemoryEntry {
    value: Vec<u8>,
    expires_at: Option<std::time::Instant>,
}
impl InMemoryCache {
    pub fn new(max_capacity: u64) -> Self {
        Self {
            inner: MokaCache::builder().max_capacity(max_capacity).build(),
        }
    }
}
impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new(10_000)
    }
}
#[async_trait]
impl Cache for InMemoryCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        Ok(self.inner.get(key).await.and_then(|entry| {
            if entry
                .expires_at
                .is_none_or(|e| e > std::time::Instant::now())
            {
                Some(entry.value)
            } else {
                None
            }
        }))
    }
    async fn set(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        self.inner
            .insert(
                key.to_owned(),
                MemoryEntry {
                    value,
                    expires_at: ttl.map(|t| std::time::Instant::now() + t),
                },
            )
            .await;
        Ok(())
    }
    async fn delete(&self, key: &str) -> Result<(), CacheError> {
        self.inner.invalidate(key).await;
        Ok(())
    }
}

#[derive(Clone)]
pub struct DatabaseCache {
    db: djangors_db::Database,
}
impl DatabaseCache {
    pub fn new(db: djangors_db::Database) -> Self {
        Self { db }
    }
    async fn ensure_table(&self) -> Result<(), CacheError> {
        sqlx::query("CREATE TABLE IF NOT EXISTS djangors_cache_entries (key TEXT PRIMARY KEY, value BYTEA NOT NULL, expires_at TIMESTAMPTZ)").execute(self.db.pool()).await.map(|_| ()).map_err(|e| CacheError::Backend(e.to_string()))
    }
}
#[async_trait]
impl Cache for DatabaseCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        self.ensure_table().await?;
        let row = sqlx::query_as::<_, (Vec<u8>, Option<DateTime<Utc>>)>(
            "SELECT value, expires_at FROM djangors_cache_entries WHERE key = $1",
        )
        .bind(key)
        .fetch_optional(self.db.pool())
        .await
        .map_err(|e| CacheError::Backend(e.to_string()))?;
        if let Some((value, expiry)) = row {
            if expiry.is_none_or(|e| e > Utc::now()) {
                return Ok(Some(value));
            }
            self.delete(key).await?;
        }
        Ok(None)
    }
    async fn set(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        self.ensure_table().await?;
        let expires = ttl.map(|t| Utc::now() + chrono::Duration::from_std(t).unwrap_or_default());
        sqlx::query("INSERT INTO djangors_cache_entries (key, value, expires_at) VALUES ($1, $2, $3) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, expires_at = EXCLUDED.expires_at").bind(key).bind(value).bind(expires).execute(self.db.pool()).await.map(|_| ()).map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn delete(&self, key: &str) -> Result<(), CacheError> {
        self.ensure_table().await?;
        sqlx::query("DELETE FROM djangors_cache_entries WHERE key = $1")
            .bind(key)
            .execute(self.db.pool())
            .await
            .map(|_| ())
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
}

#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisCache {
    client: redis::Client,
}
#[cfg(feature = "redis")]
impl RedisCache {
    pub fn new(url: &str) -> Result<Self, CacheError> {
        redis::Client::open(url)
            .map(|client| Self { client })
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn connection(&self) -> Result<redis::aio::MultiplexedConnection, CacheError> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
}
#[cfg(feature = "redis")]
#[async_trait]
impl Cache for RedisCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        let mut c = self.connection().await?;
        redis::cmd("GET")
            .arg(key)
            .query_async(&mut c)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn set(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        let mut c = self.connection().await?;
        let mut cmd = redis::cmd("SET");
        cmd.arg(key).arg(value);
        if let Some(t) = ttl {
            cmd.arg("PX").arg(t.as_millis() as u64);
        }
        cmd.query_async::<()>(&mut c)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
    async fn delete(&self, key: &str) -> Result<(), CacheError> {
        let mut c = self.connection().await?;
        redis::cmd("DEL")
            .arg(key)
            .query_async::<()>(&mut c)
            .await
            .map_err(|e| CacheError::Backend(e.to_string()))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CacheableResponse;
#[derive(Clone)]
pub struct CacheLayer {
    cache: Arc<dyn Cache>,
    ttl: Option<Duration>,
}
impl CacheLayer {
    pub fn new(cache: Arc<dyn Cache>, ttl: Option<Duration>) -> Self {
        Self { cache, ttl }
    }
}
impl<S> Layer<S> for CacheLayer
where
    S: Service<
            Request<Full<bytes::Bytes>>,
            Response = Response<Full<bytes::Bytes>>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Service = CacheService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        CacheService {
            inner,
            cache: self.cache.clone(),
            ttl: self.ttl,
        }
    }
}
#[derive(Clone)]
pub struct CacheService<S> {
    inner: S,
    cache: Arc<dyn Cache>,
    ttl: Option<Duration>,
}
impl<S> Service<Request<Full<bytes::Bytes>>> for CacheService<S>
where
    S: Service<
            Request<Full<bytes::Bytes>>,
            Response = Response<Full<bytes::Bytes>>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<Full<bytes::Bytes>>;
    type Error = Infallible;
    type Future =
        std::pin::Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }
    fn call(&mut self, req: Request<Full<bytes::Bytes>>) -> Self::Future {
        let mut inner = self.inner.clone();
        let cache = self.cache.clone();
        let ttl = self.ttl;
        Box::pin(async move {
            if req.method() != hyper::Method::GET {
                return inner.call(req).await;
            }
            let key = format!("{}?{}", req.uri().path(), req.uri().query().unwrap_or(""));
            if let Some(bytes) = cache.get(&key).await.unwrap_or(None) {
                if let Ok(cached) = serde_json::from_slice::<CachedResponse>(&bytes) {
                    return Ok(cached.into_response());
                }
            }
            let response = inner.call(req).await?;
            if response.extensions().get::<CacheableResponse>().is_some()
                && !response.headers().contains_key(hyper::header::SET_COOKIE)
            {
                let cached = CachedResponse::from_response(&response);
                if let Ok(bytes) = serde_json::to_vec(&cached) {
                    let _ = cache.set(&key, bytes, ttl).await;
                }
            }
            Ok(response)
        })
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedResponse {
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
}
impl CachedResponse {
    fn from_response(r: &Response<Full<bytes::Bytes>>) -> Self {
        Self {
            status: r.status().as_u16(),
            headers: r
                .headers()
                .iter()
                .map(|(n, v)| (n.to_string(), v.as_bytes().to_vec()))
                .collect(),
            body: r.body().clone().into_inner().unwrap_or_default().to_vec(),
        }
    }
    fn into_response(self) -> Response<Full<bytes::Bytes>> {
        let mut r = Response::builder()
            .status(self.status)
            .body(Full::new(bytes::Bytes::from(self.body)))
            .unwrap();
        for (n, v) in self.headers {
            if let (Ok(n), Ok(v)) = (
                n.parse::<hyper::header::HeaderName>(),
                hyper::header::HeaderValue::from_bytes(&v),
            ) {
                r.headers_mut().append(n, v);
            }
        }
        r.extensions_mut().insert(CacheableResponse);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::{
        convert::Infallible,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };
    use tower::{service_fn, ServiceExt};

    async fn exercise(cache: &dyn Cache) {
        assert_eq!(cache.get("missing").await.unwrap(), None);
        cache
            .set("key", b"value".to_vec(), Some(Duration::from_millis(50)))
            .await
            .unwrap();
        assert_eq!(cache.get("key").await.unwrap(), Some(b"value".to_vec()));
        assert_eq!(
            cache
                .get_or_set("key", None, || async { Ok(b"new".to_vec()) })
                .await
                .unwrap(),
            b"value"
        );
        cache.delete("key").await.unwrap();
        assert_eq!(
            cache
                .get_or_set("key", None, || async { Ok(b"new".to_vec()) })
                .await
                .unwrap(),
            b"new"
        );
        cache
            .set("ttl", b"gone".to_vec(), Some(Duration::from_millis(50)))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(cache.get("ttl").await.unwrap(), None);
    }
    #[tokio::test]
    async fn in_memory_cache_operations_and_ttl() {
        exercise(&InMemoryCache::default()).await;
    }
    #[tokio::test]
    async fn cache_layer_requires_response_opt_in() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let handler = service_fn(move |_req: Request<Full<Bytes>>| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                let mut r = Response::new(Full::new(Bytes::from_static(b"cached")));
                r.extensions_mut().insert(CacheableResponse);
                Ok::<_, Infallible>(r)
            }
        });
        let mut svc = CacheLayer::new(Arc::new(InMemoryCache::default()), None).layer(handler);
        let request = || {
            Request::builder()
                .method("GET")
                .uri("/fragment?x=1")
                .body(Full::new(Bytes::new()))
                .unwrap()
        };
        assert_eq!(
            svc.ready()
                .await
                .unwrap()
                .call(request())
                .await
                .unwrap()
                .body()
                .clone()
                .into_inner(),
            Some(Bytes::from_static(b"cached"))
        );
        svc.ready().await.unwrap().call(request()).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
    #[cfg(feature = "redis")]
    #[tokio::test]
    async fn redis_cache_operations_and_ttl() {
        exercise(&RedisCache::new("redis://127.0.0.1:6379").unwrap()).await;
    }
    #[tokio::test]
    async fn database_cache_operations_and_ttl() {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/djangors_test".into());
        let db = djangors_db::Database::connect(&djangors_db::DatabaseConfig::new(url))
            .await
            .unwrap();
        let cache = DatabaseCache::new(db);
        sqlx::query("DROP TABLE IF EXISTS djangors_cache_entries")
            .execute(cache.db.pool())
            .await
            .unwrap();
        exercise(&cache).await;
        sqlx::query("DROP TABLE IF EXISTS djangors_cache_entries")
            .execute(cache.db.pool())
            .await
            .unwrap();
    }
}
