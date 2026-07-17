#![no_main]
use libfuzzer_sys::fuzz_target;
use djangors_core::Request;
use hyper::http::{Method, Uri, HeaderMap};
use bytes::Bytes;

fuzz_target!(|data: &[u8]| {
    let q = String::from_utf8_lossy(data);
    let uri_str = format!("http://x/p?{}", q);
    
    if let Ok(uri) = Uri::try_from(uri_str.as_str()) {
        let req = Request::new(Method::GET, uri, HeaderMap::new(), Bytes::new());
        let _ = req.query("q");
        let _ = req.query("name");
        let _ = req.query("page");
        let _ = req.query(&q);
    }
});
