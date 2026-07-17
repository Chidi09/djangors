#![no_main]
use libfuzzer_sys::fuzz_target;
use djangors_sessions::{SessionLayer, SignedCookieStore, Session};
use djangors_core::middleware::{CsrfLayer, CsrfToken};
use hyper::header::{HeaderValue, COOKIE};
use http_body_util::Full;
use bytes::Bytes;
use tower::{Layer, Service};
use std::sync::OnceLock;
use tokio::runtime::Runtime;

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    })
}

fuzz_target!(|data: &[u8]| {
    let header_val = match HeaderValue::from_bytes(data) {
        Ok(v) => v,
        Err(_) => return,
    };

    let rt = runtime();
    
    rt.block_on(async {
        // 1. Test SessionLayer
        let secret = b"super-secret-key-for-testing-purposes-only-32bytes-long";
        let store = SignedCookieStore::new(secret);
        let session_layer = SessionLayer::new(store);
        
        let inner_session_svc = tower::service_fn(|req: hyper::Request<Full<Bytes>>| async move {
            let _session = req.extensions().get::<Session>();
            Ok::<_, std::convert::Infallible>(hyper::Response::new(Full::new(Bytes::new())))
        });
        
        let mut session_svc = session_layer.layer(inner_session_svc);
        
        let req1 = hyper::Request::builder()
            .method("GET")
            .uri("/")
            .header(COOKIE, header_val.clone())
            .body(Full::new(Bytes::new()))
            .unwrap();
            
        let _ = session_svc.call(req1).await;
        
        // 2. Test CsrfLayer
        let csrf_layer = CsrfLayer::new();
        let inner_csrf_svc = tower::service_fn(|req: hyper::Request<Full<Bytes>>| async move {
            let _csrf = req.extensions().get::<CsrfToken>();
            Ok::<_, std::convert::Infallible>(hyper::Response::new(Full::new(Bytes::new())))
        });
        
        let mut csrf_svc = csrf_layer.layer(inner_csrf_svc);
        
        let req2 = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .header(COOKIE, header_val)
            .body(Full::new(Bytes::new()))
            .unwrap();
            
        let _ = csrf_svc.call(req2).await;
    });
});
