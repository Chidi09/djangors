# How to Cache a View Response

## Problem
You want to cache HTTP GET view responses to reduce database load and speed up response times using in-memory or Redis backends.

## Solution
`djangors_cache::CacheLayer` is a `tower::Layer` that caches GET responses under two strict
security requirements:
1. **Explicit Opt-in**: the response must carry a `CacheableResponse` marker extension.
2. **No `Set-Cookie`**: responses setting cookies (e.g. session cookies) are **never** cached,
   regardless of the opt-in marker.

## ⚠️ Current integration limitation

`CacheLayer` operates on the raw `hyper::Response<Full<Bytes>>` that comes out of the connection-
serving loop, not on `djangors_core::Response` — the framework's own `Response` type has no
`extensions_mut()` (or any extensions API at all). This means there is currently **no way for an
ordinary Djangors view handler to opt a response into caching** — the marker has to be inserted on
the raw hyper response, which only code operating below the framework's own dispatch layer can do.
This is the same "primitive exists, framework-level wiring doesn't yet" gap several other Phase 7
crates (`djangors-contrib-messages`, `-guardian`, `-otp`) document rather than paper over — see
those guides/crates' own docs for the same pattern.

`CacheLayer` is still directly usable **today** if you're composing your own `tower::Service` stack
below `djangors_core`'s dispatch (i.e. you construct the `CacheableResponse`-marked
`hyper::Response` yourself, the way `djangors-cache`'s own test suite does):

```rust
use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response};
use tower::{service_fn, Layer, Service, ServiceExt};
use djangors_cache::{InMemoryCache, CacheLayer, CacheableResponse};

async fn build_and_call_cached_service() {
    let handler = service_fn(|_req: Request<Full<Bytes>>| async move {
        let mut r = Response::new(Full::new(Bytes::from_static(b"dashboard body")));
        // Opt-in: only code with direct access to the raw hyper::Response can set this today.
        r.extensions_mut().insert(CacheableResponse);
        Ok::<_, std::convert::Infallible>(r)
    });

    let cache = Arc::new(InMemoryCache::new(10_000));
    let mut svc = CacheLayer::new(cache, Some(Duration::from_secs(300))).layer(handler);

    let request = Request::builder()
        .method("GET")
        .uri("/dashboard")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let _response = svc.ready().await.unwrap().call(request).await.unwrap();
}
```

Wiring `CacheableResponse` into an ordinary `djangors_core::Router`-based view handler would
require a core `djangors_core::Response` change (an extensions API, or a dedicated
`Response::cacheable()` constructor) — not attempted here since it's out of scope for a how-to;
track it as a real, open Phase-9-discovered gap if you need it.
