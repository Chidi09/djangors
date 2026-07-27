//! Typed request extractors for the Djangors web framework.
//!
//! Handlers currently receive raw requests and path parameters.
//! This module provides a set of typed extractors (`Json`, `Query`, `Form`)
//! and a `FromRequest` trait to extract typed information directly from the request.
//!
//! # Usage
//!
//! Today, you can manually use these extractors inside a handler body:
//!
//! ```rust
//! # use djangors_core::Request;
//! # use djangors_core::error::DjangorsError;
//! # use djangors_core::extract::{FromRequest, Json};
//! # use serde::Deserialize;
//! #
//! # #[derive(Deserialize)]
//! # struct CreateUser { username: String }
//! #
//! # async fn handle(req: Request) -> Result<(), DjangorsError> {
//! let Json(payload) = Json::<CreateUser>::from_request(&req).await?;
//! // Use payload...
//! # Ok(())
//! # }
//! ```
//!
//! In the future, these extractors will be integrated into the main `Handler` trait
//! so handlers can declare them as function parameters.

use crate::error::DjangorsError;
use crate::path_params::PathParams;
use crate::request::Request;

/// A trait for types that can be extracted from an HTTP request.
#[async_trait::async_trait]
pub trait FromRequest: Sized {
    /// Extract `Self` from the request.
    async fn from_request(req: &Request) -> Result<Self, DjangorsError>;
}

/// Extractor for deserializing JSON request bodies.
///
/// If the request lacks a JSON body, contains invalid JSON, or has an invalid
/// content-type header, a [`DjangorsError::BadRequest`] will be returned.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

#[async_trait::async_trait]
impl<T: serde::de::DeserializeOwned + Send> FromRequest for Json<T> {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        // Validate Content-Type if present and clearly not JSON.
        if let Some(content_type) = req.header("content-type") {
            if let Ok(ct_str) = content_type.to_str() {
                let ct_lower = ct_str.to_ascii_lowercase();
                if ct_lower.starts_with("text/") || ct_lower.starts_with("multipart/") {
                    return Err(DjangorsError::BadRequest(format!(
                        "invalid Content-Type: {ct_str}"
                    )));
                }
            }
        }

        let body = req.body_bytes().await;
        let value = serde_json::from_slice(body)
            .map_err(|e| DjangorsError::BadRequest(format!("failed to parse JSON: {e}")))?;
        Ok(Json(value))
    }
}

/// Extractor for deserializing query parameters from the request URI.
///
/// If the query string is missing or is not matching the structure of `T`,
/// a [`DjangorsError::BadRequest`] is returned.
#[derive(Debug, Clone, Copy, Default)]
pub struct Query<T>(pub T);

#[async_trait::async_trait]
impl<T: serde::de::DeserializeOwned + Send> FromRequest for Query<T> {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let raw_query = req.raw_query().unwrap_or("");
        let value = serde_urlencoded::from_str(raw_query)
            .map_err(|e| DjangorsError::BadRequest(format!("failed to parse query string: {e}")))?;
        Ok(Query(value))
    }
}

/// Extractor for deserializing URL-encoded form bodies.
///
/// If the body bytes do not form a valid form representation matching `T`,
/// a [`DjangorsError::BadRequest`] is returned.
#[derive(Debug, Clone, Copy, Default)]
pub struct Form<T>(pub T);

#[async_trait::async_trait]
impl<T: serde::de::DeserializeOwned + Send> FromRequest for Form<T> {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let body = req.body_bytes().await;
        let value = serde_urlencoded::from_bytes(body)
            .map_err(|e| DjangorsError::BadRequest(format!("failed to parse form body: {e}")))?;
        Ok(Form(value))
    }
}

/// A thin wrapper function around [`PathParams::get_as`] for path extraction.
///
/// This provides consistent naming alongside the other extractors.
pub fn extract_path_param<T: std::str::FromStr>(
    params: &PathParams,
    key: &str,
) -> Result<T, DjangorsError> {
    params.get_as(key)
}

#[cfg(test)]
mod tests {
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
}
