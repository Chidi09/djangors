use std::future::Future;
use std::pin::Pin;

use crate::error::DjangorsError;
use crate::path_params::PathParams;
use crate::request::Request;
use crate::response::Response;

/// Trait for types that can handle an HTTP request and produce a response.
///
/// Implementations receive the request and matched path parameters by value,
/// enabling the returned future to be `'static` and allowing a blanket impl
/// over plain `async fn` closures.
pub trait Handler: Send + Sync {
    /// Handle an incoming request and return a response (or error).
    fn call(
        &self,
        req: Request,
        params: PathParams,
    ) -> Pin<Box<dyn Future<Output = Result<Response, DjangorsError>> + Send>>;
}

impl<F, Fut> Handler for F
where
    F: Fn(Request, PathParams) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Response, DjangorsError>> + Send + 'static,
{
    fn call(
        &self,
        req: Request,
        params: PathParams,
    ) -> Pin<Box<dyn Future<Output = Result<Response, DjangorsError>> + Send>> {
        Box::pin(self(req, params))
    }
}

/// Trait for types that can handle an HTTP request and produce a [`StreamingResponse`].
pub trait StreamingHandler: Send + Sync {
    /// Handle an incoming request and return a streaming response (or error).
    fn call(
        &self,
        req: Request,
        params: PathParams,
    ) -> Pin<Box<dyn Future<Output = Result<crate::sse::StreamingResponse, DjangorsError>> + Send>>;
}

impl<F, Fut> StreamingHandler for F
where
    F: Fn(Request, PathParams) -> Fut + Send + Sync,
    Fut: Future<Output = Result<crate::sse::StreamingResponse, DjangorsError>> + Send + 'static,
{
    fn call(
        &self,
        req: Request,
        params: PathParams,
    ) -> Pin<Box<dyn Future<Output = Result<crate::sse::StreamingResponse, DjangorsError>> + Send>>
    {
        Box::pin(self(req, params))
    }
}
