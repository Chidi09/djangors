use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::Full;

use crate::Router;

impl<B> tower::Service<hyper::Request<B>> for Router
where
    B: hyper::body::Body<Data = Bytes> + Send + 'static,
    B::Error: std::fmt::Display + Send + 'static,
{
    type Response = hyper::Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: hyper::Request<B>) -> Self::Future {
        let this = self.clone();
        Box::pin(async move { Ok(this.dispatch(req).await) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use bytes::Bytes;
    use hyper::http::{Method, StatusCode};
    use tower::Service;
    use tower::ServiceExt;

    use crate::error::DjangorsError;
    use crate::path_params::PathParams;
    use crate::request::Request;
    use crate::response::Response;

    async fn hello_handler(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::OK, "hello from tower"))
    }

    #[tokio::test]
    async fn service_oneshot_direct() {
        let router = Router::new().get("/hello", hello_handler);

        let hyper_req = hyper::Request::builder()
            .method(Method::GET)
            .uri("/hello")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();

        let resp = router.oneshot(hyper_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = String::from_utf8(
            http_body_util::BodyExt::collect(resp.into_body())
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert_eq!(body, "hello from tower");
    }

    #[tokio::test]
    async fn service_with_tower_middleware() {
        use crate::middleware::logging_layer;
        use tower::service_fn;
        use tower::ServiceBuilder;

        let router = Router::new().get("/hello", hello_handler);

        let svc_fn = service_fn(move |req: hyper::Request<http_body_util::Full<Bytes>>| {
            let router = router.clone();
            async move { Ok::<_, Infallible>(router.dispatch(req).await) }
        });

        let mut svc = ServiceBuilder::new().layer(logging_layer()).service(svc_fn);

        let hyper_req = hyper::Request::builder()
            .method(Method::GET)
            .uri("/hello")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();

        let resp = svc.ready().await.unwrap().call(hyper_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = String::from_utf8(
            http_body_util::BodyExt::collect(resp.into_body())
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert_eq!(body, "hello from tower");
    }

    #[tokio::test]
    async fn service_not_found() {
        let router = Router::new().get("/hello", hello_handler);

        let hyper_req = hyper::Request::builder()
            .method(Method::GET)
            .uri("/nonexistent")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();

        let resp = router.oneshot(hyper_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
