use std::fmt;

use hyper::StatusCode;

use crate::response::Response;

/// Errors that can occur during request handling.
#[derive(Debug)]
pub enum DjangorsError {
    /// The requested resource was not found (404).
    NotFound,
    /// The request was malformed (400).
    BadRequest(String),
    /// An internal server error occurred (500).
    Internal(String),
}

impl fmt::Display for DjangorsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DjangorsError::NotFound => write!(f, "Not Found"),
            DjangorsError::BadRequest(msg) => write!(f, "Bad Request: {msg}"),
            DjangorsError::Internal(msg) => write!(f, "Internal Error: {msg}"),
        }
    }
}

impl std::error::Error for DjangorsError {}

impl DjangorsError {
    /// Convert this error into an HTTP response.
    pub fn into_response(self) -> Response {
        match self {
            DjangorsError::NotFound => Response::text(StatusCode::NOT_FOUND, "404 Not Found"),
            DjangorsError::BadRequest(msg) => {
                Response::text(StatusCode::BAD_REQUEST, &format!("400 Bad Request: {msg}"))
            }
            DjangorsError::Internal(msg) => Response::text(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("500 Internal Server Error: {msg}"),
            ),
        }
    }
}
