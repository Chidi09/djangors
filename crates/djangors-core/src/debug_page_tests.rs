use super::*;
use bytes::Bytes;
use hyper::http::{HeaderMap, HeaderValue, Method, Uri};

fn make_request(method: Method, path: &str) -> Request {
    let uri = Uri::try_from(path).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("X-Test-Header", HeaderValue::from_static("test-value"));
    Request::new(method, uri, headers, Bytes::new())
}

#[test]
fn test_render_debug_page() {
    let req = make_request(Method::GET, "/some/path");
    let err = DjangorsError::Panicked("something exploded".to_string());
    let resp = render_debug_page(&err, &req);

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert!(body.contains("something exploded"));
    assert!(body.contains("&#x2F;some&#x2F;path"));
    assert!(body.contains("x-test-header"));
    assert!(body.contains("test-value"));
    assert!(body.contains("DEBUG = true"));
}

#[test]
fn test_render_production_error_page() {
    let err_resp = render_production_error_page(StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(err_resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = String::from_utf8(err_resp.body().to_vec()).unwrap();
    assert!(body.contains("500"));
    assert!(body.contains("Internal Server Error"));
    // Check no leak of debug note
    assert!(!body.contains("DEBUG = true"));

    let not_found_resp = render_production_error_page(StatusCode::NOT_FOUND);
    assert_eq!(not_found_resp.status(), StatusCode::NOT_FOUND);
    let body_nf = String::from_utf8(not_found_resp.body().to_vec()).unwrap();
    assert!(body_nf.contains("404"));
    assert!(body_nf.contains("Not Found"));
}
