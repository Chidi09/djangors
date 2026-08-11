use super::*;
use hyper::http::StatusCode;

#[test]
fn text_response() {
    let resp = Response::text(StatusCode::OK, "hello");
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(&resp.body[..], b"hello");
    assert_eq!(
        resp.headers.get(CONTENT_TYPE).unwrap(),
        "text/plain; charset=utf-8"
    );
}

#[test]
fn html_response() {
    let resp = Response::html(StatusCode::CREATED, "<h1>hi</h1>");
    assert_eq!(resp.status, StatusCode::CREATED);
    assert_eq!(
        resp.headers.get(CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
}

#[test]
fn json_response() {
    #[derive(serde::Serialize)]
    struct Data {
        key: String,
    }
    let resp = Response::json(StatusCode::OK, &Data { key: "val".into() }).unwrap();
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(
        resp.headers.get(CONTENT_TYPE).unwrap(),
        "application/json; charset=utf-8"
    );
    let body_str = String::from_utf8(resp.body.to_vec()).unwrap();
    assert!(body_str.contains("\"key\""));
    assert!(body_str.contains("\"val\""));
}

#[test]
fn bytes_response() {
    let resp = Response::bytes(StatusCode::OK, "image/png", vec![1, 2, 3]);
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(&resp.body[..], &[1, 2, 3]);
    assert_eq!(resp.headers.get(CONTENT_TYPE).unwrap(), "image/png");
}

#[test]
fn redirect_response() {
    let resp = Response::redirect("/login");
    assert_eq!(resp.status, StatusCode::FOUND);
    assert_eq!(resp.headers.get(LOCATION).unwrap(), "/login");
}

#[test]
fn builder_header() {
    let resp = Response::text(StatusCode::OK, "body").header("X-Custom", "myval");
    assert_eq!(resp.headers.get("x-custom").unwrap(), "myval");
}

#[test]
fn into_hyper_conversion() {
    let resp = Response::text(StatusCode::OK, "test");
    let hyper_resp: hyper::Response<http_body_util::Full<Bytes>> = resp.into();
    assert_eq!(hyper_resp.status(), StatusCode::OK);
}
