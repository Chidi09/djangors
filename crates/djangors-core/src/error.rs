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
    /// The handler panicked (500).
    Panicked(String),
    /// The request is unauthorized (401).
    Unauthorized(String),
}

impl fmt::Display for DjangorsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DjangorsError::NotFound => write!(f, "Not Found"),
            DjangorsError::BadRequest(msg) => write!(f, "Bad Request: {msg}"),
            DjangorsError::Internal(msg) => write!(f, "Internal Error: {msg}"),
            DjangorsError::Panicked(msg) => write!(f, "Handler panicked: {msg}"),
            DjangorsError::Unauthorized(msg) => write!(f, "Unauthorized: {msg}"),
        }
    }
}

impl std::error::Error for DjangorsError {}

impl DjangorsError {
    /// Return the HTTP status code for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            DjangorsError::NotFound => StatusCode::NOT_FOUND,
            DjangorsError::BadRequest(_) => StatusCode::BAD_REQUEST,
            DjangorsError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            DjangorsError::Panicked(_) => StatusCode::INTERNAL_SERVER_ERROR,
            DjangorsError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
        }
    }

    /// Convert this error into an HTTP response.
    pub fn into_response(self) -> Response {
        let status = self.status_code();
        match self {
            DjangorsError::NotFound => Response::text(status, "404 Not Found"),
            DjangorsError::BadRequest(msg) => {
                Response::text(status, &format!("400 Bad Request: {msg}"))
            }
            DjangorsError::Internal(msg) => {
                Response::text(status, &format!("500 Internal Server Error: {msg}"))
            }
            DjangorsError::Panicked(msg) => Response::text(
                status,
                &format!("500 Internal Server Error: Handler panicked: {msg}"),
            ),
            DjangorsError::Unauthorized(msg) => {
                Response::text(status, &format!("401 Unauthorized: {msg}"))
            }
        }
    }
}
