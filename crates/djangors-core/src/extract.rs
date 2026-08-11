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

use std::collections::HashMap;

use bytes::Bytes;

use crate::error::DjangorsError;
use crate::path_params::PathParams;
use crate::request::Request;

/// A single uploaded file from a multipart request.
///
/// Mirrors Django's `request.FILES` — present only for actual file parts,
/// absent for plain text fields.
#[derive(Debug, Clone)]
pub struct UploadedFile {
    /// The form field name (e.g. `"avatar"`).
    pub field_name: String,
    /// The original filename supplied by the client (e.g. `"photo.jpg"`).
    pub file_name: String,
    /// The MIME type from the part's `Content-Type` header, if present.
    pub content_type: Option<String>,
    /// The raw file bytes.
    pub bytes: Bytes,
}

/// The result of parsing a `multipart/form-data` request body.
///
/// Splits fields into two collections, mirroring Django's `request.FILES`
/// vs `request.POST` distinction.
#[derive(Debug, Clone)]
pub struct MultipartData {
    /// File parts — fields that included a `filename` parameter.
    pub files: Vec<UploadedFile>,
    /// Plain text fields — fields without a filename, keyed by field name.
    pub texts: HashMap<String, String>,
}

/// Extractor for parsing `multipart/form-data` request bodies.
///
/// Parses the `Content-Type` header for the `boundary` parameter, feeds
/// the buffered body bytes through `multer`, and yields a [`MultipartData`]
/// with files and texts separated.
///
/// # Size limits
///
/// A default size limit (10 MB whole-stream, 5 MB per-field) is applied
/// to prevent memory-exhaustion DoS. Use
/// [`from_request_with_constraints`](Self::from_request_with_constraints)
/// for custom limits.
#[derive(Debug)]
pub struct Multipart(pub MultipartData);

const DEFAULT_WHOLE_STREAM_LIMIT: u64 = 10 * 1024 * 1024;
const DEFAULT_PER_FIELD_LIMIT: u64 = 5 * 1024 * 1024;

#[async_trait::async_trait]
impl FromRequest for Multipart {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let constraints = multer::Constraints::new().size_limit(
            multer::SizeLimit::new()
                .whole_stream(DEFAULT_WHOLE_STREAM_LIMIT)
                .per_field(DEFAULT_PER_FIELD_LIMIT),
        );
        Self::from_request_with_constraints(req, constraints).await
    }
}

impl Multipart {
    /// Parse a multipart request with custom [`multer::Constraints`].
    ///
    /// Useful for overriding the default size limits.
    pub async fn from_request_with_constraints(
        req: &Request,
        constraints: multer::Constraints,
    ) -> Result<Self, DjangorsError> {
        let content_type = req
            .header("content-type")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                DjangorsError::BadRequest("missing Content-Type header for multipart".into())
            })?;

        let boundary = multer::parse_boundary(content_type)
            .map_err(|e| DjangorsError::BadRequest(format!("invalid multipart Content-Type: {e}")))?
            .to_string();

        let body = req.body_bytes().await;
        let body_bytes = Bytes::copy_from_slice(body);

        let stream =
            futures_util::stream::once(async move { Ok::<Bytes, std::io::Error>(body_bytes) });

        let mut multipart = multer::Multipart::with_constraints(stream, &boundary, constraints);

        let mut files = Vec::new();
        let mut texts = HashMap::new();

        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|e| DjangorsError::BadRequest(format!("multipart parse error: {e}")))?
        {
            let name = field.name().unwrap_or("").to_string();
            let file_name = field.file_name().map(|s| s.to_string());
            let content_type = field.content_type().map(|m| m.to_string());

            let data = field
                .bytes()
                .await
                .map_err(|e| DjangorsError::BadRequest(format!("multipart field error: {e}")))?;

            if let Some(file_name) = file_name {
                files.push(UploadedFile {
                    field_name: name,
                    file_name,
                    content_type,
                    bytes: data,
                });
            } else {
                let text = String::from_utf8(data.to_vec()).map_err(|_| {
                    DjangorsError::BadRequest("non-UTF-8 text field in multipart".into())
                })?;
                texts.insert(name, text);
            }
        }

        Ok(Multipart(MultipartData { files, texts }))
    }
}

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
#[path = "extract_tests.rs"]
mod tests;
