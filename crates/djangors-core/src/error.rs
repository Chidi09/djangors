use std::fmt;

use hyper::StatusCode;

use crate::request::Request;
use crate::response::Response;

/// A project-supplied override for how [`DjangorsError`] values are rendered
/// into HTTP responses.
pub trait ErrorRenderer: Send + Sync {
    /// Render an error for the given request.
    fn render(&self, err: &DjangorsError, req: &Request) -> Response;
}

/// A JSON error renderer suitable for JSON APIs.
pub struct JsonErrorRenderer;

impl ErrorRenderer for JsonErrorRenderer {
    fn render(&self, err: &DjangorsError, _req: &Request) -> Response {
        #[derive(serde::Serialize)]
        struct Envelope<'a> {
            error: ErrorBody<'a>,
        }
        #[derive(serde::Serialize)]
        struct ErrorBody<'a> {
            status: u16,
            code: &'static str,
            message: &'a str,
        }

        let message = err.message();
        Response::json(
            err.status_code(),
            &Envelope {
                error: ErrorBody {
                    status: err.status_code().as_u16(),
                    code: err.code(),
                    message: &message,
                },
            },
        )
        .expect("JSON error envelope is serializable")
    }
}

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
    /// The request is forbidden (403).
    Forbidden(String),
    /// The request exceeded a configured rate limit (429).
    TooManyRequests(String),
}

impl fmt::Display for DjangorsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DjangorsError::NotFound => write!(f, "Not Found"),
            DjangorsError::BadRequest(msg) => write!(f, "Bad Request: {msg}"),
            DjangorsError::Internal(msg) => write!(f, "Internal Error: {msg}"),
            DjangorsError::Panicked(msg) => write!(f, "Handler panicked: {msg}"),
            DjangorsError::Unauthorized(msg) => write!(f, "Unauthorized: {msg}"),
            DjangorsError::Forbidden(msg) => write!(f, "Forbidden: {msg}"),
            DjangorsError::TooManyRequests(msg) => write!(f, "Too Many Requests: {msg}"),
        }
    }
}

impl std::error::Error for DjangorsError {}

impl DjangorsError {
    fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::BadRequest(_) => "bad_request",
            Self::Internal(_) => "internal",
            Self::Panicked(_) => "panicked",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::TooManyRequests(_) => "too_many_requests",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::NotFound => "Not Found".to_string(),
            Self::BadRequest(msg)
            | Self::Internal(msg)
            | Self::Panicked(msg)
            | Self::Unauthorized(msg)
            | Self::Forbidden(msg) => msg.clone(),
            Self::TooManyRequests(msg) => msg.clone(),
        }
    }

    /// Render this error with a project-registered renderer, if present.
    pub fn try_custom_render(&self, req: &Request) -> Option<Response> {
        req.state::<std::sync::Arc<dyn ErrorRenderer>>()
            .map(|renderer| renderer.render(self, req))
    }

    /// Return the HTTP status code for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            DjangorsError::NotFound => StatusCode::NOT_FOUND,
            DjangorsError::BadRequest(_) => StatusCode::BAD_REQUEST,
            DjangorsError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            DjangorsError::Panicked(_) => StatusCode::INTERNAL_SERVER_ERROR,
            DjangorsError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            DjangorsError::Forbidden(_) => StatusCode::FORBIDDEN,
            DjangorsError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
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
            DjangorsError::Forbidden(msg) => {
                Response::text(status, &format!("403 Forbidden: {msg}"))
            }
            DjangorsError::TooManyRequests(msg) => {
                Response::text(status, &format!("429 Too Many Requests: {msg}"))
            }
        }
    }
}
