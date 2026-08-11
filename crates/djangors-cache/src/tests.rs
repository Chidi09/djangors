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
        .set("key", b"value".to_vec(), Some(Duration::from_secs(5)))
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
    let sqlite = matches!(
        std::env::var("TEST_BACKEND").as_deref(),
        Ok(value) if value.eq_ignore_ascii_case("sqlite")
    );
    let config = if sqlite {
        // Each sqlite::memory: connection owns a separate database, so use a
        // single connection for the table setup and cache operations.
        djangors_db::DatabaseConfig::new("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
    } else {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/djangors_test".into());
        djangors_db::DatabaseConfig::new(url)
    };
    let db = djangors_db::Database::connect(&config).await.unwrap();
    let cache = DatabaseCache::new(db);
    cache
        .db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_cache_entries", &[])
        .await
        .unwrap();
    exercise(&cache).await;
    cache
        .db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_cache_entries", &[])
        .await
        .unwrap();
}
