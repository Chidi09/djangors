#![no_main]
//! Fuzz target for `djangors_core::extract::Multipart`.
//!
//! We use a fixed boundary string ("------------------------fuzzboundary123") in the
//! `Content-Type` header (`multipart/form-data; boundary=------------------------fuzzboundary123`)
//! and pass the raw fuzz bytes as the HTTP request body. Using a fixed boundary allows libFuzzer
//! to effectively learn and mutate the structure of multipart bodies (headers, CRLF boundaries,
//! field values, and file uploads) matching the seed corpus, rather than spending iterations
//! trying to align a dynamically generated header boundary with body delimiters.

use libfuzzer_sys::fuzz_target;
use djangors_core::extract::Multipart;
use djangors_core::Request;
use hyper::http::{HeaderMap, HeaderValue, Method, Uri};
use bytes::Bytes;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

const FIXED_BOUNDARY: &str = "------------------------fuzzboundary123";

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
    let rt = runtime();

    let content_type = format!("multipart/form-data; boundary={FIXED_BOUNDARY}");
    let header_val = match HeaderValue::from_str(&content_type) {
        Ok(v) => v,
        Err(_) => return,
    };

    let mut headers = HeaderMap::new();
    headers.insert("content-type", header_val);

    let uri = match Uri::try_from("http://localhost/upload") {
        Ok(u) => u,
        Err(_) => return,
    };

    let req = Request::new(
        Method::POST,
        uri,
        headers,
        Bytes::copy_from_slice(data),
    );

    rt.block_on(async {
        // Use a tight constraint limit so fuzzer memory remains strictly bounded (64KB stream, 32KB per field)
        let constraints = multer::Constraints::new().size_limit(
            multer::SizeLimit::new()
                .whole_stream(64 * 1024)
                .per_field(32 * 1024),
        );
        let _ = Multipart::from_request_with_constraints(&req, constraints).await;
    });
});
