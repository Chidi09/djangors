use super::*;
use bytes::Bytes;
use hyper::http::{HeaderMap, HeaderValue, Method, Uri};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct User {
    username: String,
    age: i32,
}

#[tokio::test]
async fn test_json_extractor_success() {
    let body = Bytes::from(r#"{"username": "alice", "age": 30}"#);
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let req = Request::new(Method::POST, Uri::from_static("/"), headers, body);

    let Json(user) = Json::<User>::from_request(&req).await.unwrap();
    assert_eq!(user.username, "alice");
    assert_eq!(user.age, 30);
}

#[tokio::test]
async fn test_json_extractor_malformed() {
    let body = Bytes::from(r#"{"username": "alice", "age": "thirty"}"#);
    let req = Request::new(Method::POST, Uri::from_static("/"), HeaderMap::new(), body);

    let res = Json::<User>::from_request(&req).await;
    assert!(res.is_err());
    if let Err(DjangorsError::BadRequest(msg)) = res {
        assert!(msg.contains("failed to parse JSON"));
    } else {
        panic!("Expected BadRequest error");
    }
}

#[tokio::test]
async fn test_json_extractor_invalid_content_type() {
    let body = Bytes::from(r#"{"username": "alice", "age": 30}"#);
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("text/plain"));
    let req = Request::new(Method::POST, Uri::from_static("/"), headers, body);

    let res = Json::<User>::from_request(&req).await;
    assert!(res.is_err());
    if let Err(DjangorsError::BadRequest(msg)) = res {
        assert!(msg.contains("invalid Content-Type"));
    } else {
        panic!("Expected BadRequest error");
    }
}

#[tokio::test]
async fn test_query_extractor_success() {
    let req = Request::new(
        Method::GET,
        Uri::from_static("/?username=bob&age=25"),
        HeaderMap::new(),
        Bytes::new(),
    );

    let Query(user) = Query::<User>::from_request(&req).await.unwrap();
    assert_eq!(user.username, "bob");
    assert_eq!(user.age, 25);
}

#[tokio::test]
async fn test_query_extractor_missing_field() {
    let req = Request::new(
        Method::GET,
        Uri::from_static("/?username=bob"),
        HeaderMap::new(),
        Bytes::new(),
    );

    let res = Query::<User>::from_request(&req).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_form_extractor_success() {
    let body = Bytes::from("username=charlie&age=40");
    let req = Request::new(Method::POST, Uri::from_static("/"), HeaderMap::new(), body);

    let Form(user) = Form::<User>::from_request(&req).await.unwrap();
    assert_eq!(user.username, "charlie");
    assert_eq!(user.age, 40);
}

#[tokio::test]
async fn test_form_extractor_failure() {
    let body = Bytes::from("username=charlie&age=forty");
    let req = Request::new(Method::POST, Uri::from_static("/"), HeaderMap::new(), body);

    let res = Form::<User>::from_request(&req).await;
    assert!(res.is_err());
}

#[test]
fn test_raw_query_accessor() {
    let req1 = Request::new(
        Method::GET,
        Uri::from_static("/search?q=rust&page=2"),
        HeaderMap::new(),
        Bytes::new(),
    );
    assert_eq!(req1.raw_query(), Some("q=rust&page=2"));

    let req2 = Request::new(
        Method::GET,
        Uri::from_static("/search"),
        HeaderMap::new(),
        Bytes::new(),
    );
    assert_eq!(req2.raw_query(), None);
}

// ---------------------------------------------------------------------------
// Multipart / file-upload extractor tests
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
fn build_multipart_body(boundary: &str, fields: &[(&str, Option<(&str, &str)>, &str)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, file_info, value) in fields {
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"").as_bytes(),
        );
        if let Some((filename, content_type)) = file_info {
            body.extend_from_slice(format!("; filename=\"{filename}\"").as_bytes());
            body.extend_from_slice(b"\r\n");
            body.extend_from_slice(format!("Content-Type: {content_type}").as_bytes());
        }
        body.extend_from_slice(b"\r\n\r\n");
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");
    body
}

#[tokio::test]
async fn test_multipart_text_and_file() {
    let boundary = "----TESTBOUNDARY";
    let body_bytes = build_multipart_body(
        boundary,
        &[
            ("title", None, "Hello World"),
            (
                "document",
                Some(("readme.txt", "text/plain")),
                "This is the file content.",
            ),
        ],
    );

    let content_type_val = format!("multipart/form-data; boundary={boundary}");
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_str(&content_type_val).unwrap(),
    );

    let req = Request::new(
        Method::POST,
        Uri::from_static("/"),
        headers,
        Bytes::from(body_bytes),
    );

    let Multipart(data) = Multipart::from_request(&req).await.unwrap();

    // Check text field
    assert_eq!(data.texts.get("title").unwrap(), "Hello World");

    // Check file field
    assert_eq!(data.files.len(), 1);
    let file = &data.files[0];
    assert_eq!(file.field_name, "document");
    assert_eq!(file.file_name, "readme.txt");
    assert_eq!(file.content_type.as_deref(), Some("text/plain"));
    assert_eq!(file.bytes.as_ref(), b"This is the file content.");
}

#[tokio::test]
async fn test_multipart_oversized_body_rejected() {
    let boundary = "----OVERSIZE";
    // Build a body with a large text value
    let large_value = "A".repeat(200);
    let body_bytes = build_multipart_body(boundary, &[("data", None, &large_value)]);

    let content_type_val = format!("multipart/form-data; boundary={boundary}");
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_str(&content_type_val).unwrap(),
    );

    let req = Request::new(
        Method::POST,
        Uri::from_static("/"),
        headers,
        Bytes::from(body_bytes),
    );

    // Set an extremely tight per-field limit (50 bytes) so our 200-byte field is rejected
    let tight = multer::Constraints::new()
        .size_limit(multer::SizeLimit::new().whole_stream(10_000).per_field(50));
    let result = Multipart::from_request_with_constraints(&req, tight).await;
    assert!(result.is_err(), "expected oversized body to be rejected");
    match result {
        Err(DjangorsError::BadRequest(msg)) => {
            assert!(
                msg.contains("multipart") || msg.contains("size") || msg.contains("limit"),
                "error message should mention the size issue: {msg}"
            );
        }
        other => panic!("expected BadRequest error, got: {other:?}"),
    }
}
