use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::error::DjangorsError;
use crate::router::Router;
use crate::settings::DjangorsSettings;

/// The top-level Djangors application.
///
/// Owns a [`DjangorsSettings`] (host, port, etc.) and a [`Router`] that
/// dispatches incoming HTTP requests to registered handlers.
///
/// Calling [`run`](Self::run) binds the configured address and enters an
/// infinite accept–serve loop, conceptually analogous to Django's
/// `manage.py runserver` command which starts the development WSGI server.
pub struct Djangors {
    settings: DjangorsSettings,
    router: Router,
}

impl Djangors {
    /// Create a new Djangors application from settings and a router.
    ///
    /// No network activity takes place until [`run`](Self::run) is called.
    pub fn new(settings: DjangorsSettings, router: Router) -> Self {
        Djangors { settings, router }
    }

    /// Validate settings and bind the configured TCP address.
    ///
    /// Returns a bound [`TcpListener`] that the caller can hand to
    /// [`serve`](Self::serve).  On failure the [`DjangorsError`] includes
    /// the address that could not be bound.
    pub async fn bind(&self) -> Result<TcpListener, DjangorsError> {
        self.settings.validate()?;

        let addr = format!("{}:{}", self.settings.host, self.settings.port);
        TcpListener::bind(&addr)
            .await
            .map_err(|e| DjangorsError::Internal(format!("Failed to bind to {addr}: {e}")))
    }

    /// Serve incoming connections on a pre-bound [`TcpListener`].
    ///
    /// Each accepted connection is handled in a spawned Tokio task.  The
    /// router (cheaply cloneable — `Arc`-backed) is cloned per connection so
    /// the accept loop is never blocked by a slow handler.
    ///
    /// Per-connection errors are logged via `eprintln!` and the loop
    /// continues; a single bad connection never kills the server.
    pub async fn serve(self, listener: TcpListener) -> Result<(), DjangorsError> {
        self.serve_with_shutdown(listener, os_shutdown_signal())
            .await
    }

    /// Serve incoming connections until `shutdown` resolves, then drain active connections.
    pub async fn serve_with_shutdown<F>(
        self,
        listener: TcpListener,
        shutdown: F,
    ) -> Result<(), DjangorsError>
    where
        F: std::future::Future<Output = ()>,
    {
        let router = self.router;
        let debug = self.settings.debug;
        serve_with_shutdown_loop(listener, shutdown, move |stream| {
            let router = router.clone();
            async move {
                if let Err(e) = serve_connection(stream, router, debug).await {
                    eprintln!("Connection error: {e}");
                }
            }
        })
        .await;
        Ok(())
    }

    /// Bind the configured address and serve connections forever.
    ///
    /// This is the main entry point.  Under normal operation it runs
    /// indefinitely; `Ok(())` is returned only if the accept loop
    /// terminates (which does not happen with the current implementation).
    pub async fn run(self) -> Result<(), DjangorsError> {
        let listener = self.bind().await?;
        self.serve(listener).await
    }

    /// Bind the configured address and serve until `shutdown` resolves.
    pub async fn run_with_shutdown<F>(self, shutdown: F) -> Result<(), DjangorsError>
    where
        F: std::future::Future<Output = ()>,
    {
        let listener = self.bind().await?;
        self.serve_with_shutdown(listener, shutdown).await
    }

    /// Get a reference to the settings.
    pub fn settings(&self) -> &DjangorsSettings {
        &self.settings
    }

    /// Serve incoming connections on a pre-bound [`TcpListener`] using a custom layered service.
    pub async fn serve_service<S>(
        self,
        listener: TcpListener,
        service: S,
    ) -> Result<(), DjangorsError>
    where
        S: tower::Service<
                hyper::Request<hyper::body::Incoming>,
                Response = hyper::Response<http_body_util::Full<bytes::Bytes>>,
                Error = std::convert::Infallible,
            > + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
    {
        self.serve_service_with_shutdown(listener, service, os_shutdown_signal())
            .await
    }

    /// Serve layered-service connections until `shutdown` resolves, then drain active connections.
    pub async fn serve_service_with_shutdown<S, F>(
        self,
        listener: TcpListener,
        service: S,
        shutdown: F,
    ) -> Result<(), DjangorsError>
    where
        S: tower::Service<
                hyper::Request<hyper::body::Incoming>,
                Response = hyper::Response<http_body_util::Full<bytes::Bytes>>,
                Error = std::convert::Infallible,
            > + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
        F: std::future::Future<Output = ()>,
    {
        serve_with_shutdown_loop(listener, shutdown, move |stream| {
            let service = service.clone();
            async move {
                if let Err(e) = serve_connection_service(stream, service).await {
                    eprintln!("Connection error: {e}");
                }
            }
        })
        .await;
        Ok(())
    }

    /// Bind the configured address and serve connections forever using a custom layered service.
    pub async fn run_service<S>(self, service: S) -> Result<(), DjangorsError>
    where
        S: tower::Service<
                hyper::Request<hyper::body::Incoming>,
                Response = hyper::Response<http_body_util::Full<bytes::Bytes>>,
                Error = std::convert::Infallible,
            > + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
    {
        let listener = self.bind().await?;
        self.serve_service(listener, service).await
    }

    /// Bind the configured address and serve layered-service connections until `shutdown` resolves.
    pub async fn run_service_with_shutdown<S, F>(
        self,
        service: S,
        shutdown: F,
    ) -> Result<(), DjangorsError>
    where
        S: tower::Service<
                hyper::Request<hyper::body::Incoming>,
                Response = hyper::Response<http_body_util::Full<bytes::Bytes>>,
                Error = std::convert::Infallible,
            > + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
        F: std::future::Future<Output = ()>,
    {
        let listener = self.bind().await?;
        self.serve_service_with_shutdown(listener, service, shutdown)
            .await
    }
}

async fn os_shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        signal.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn serve_with_shutdown_loop<F, H, Fut>(listener: TcpListener, shutdown: F, handler: H)
where
    F: std::future::Future<Output = ()>,
    H: Fn(tokio::net::TcpStream) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let mut join_set = JoinSet::new();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _peer_addr)) => {
                        let connection_handler = handler.clone();
                        join_set.spawn(async move {
                            connection_handler(stream).await;
                        });
                    }
                    Err(e) => eprintln!("Accept error: {e}"),
                }
            }
            _ = &mut shutdown => {
                println!("[djangors] shutdown signal received, draining in-flight connections...");
                break;
            }
        }
    }

    let drain = async { while join_set.join_next().await.is_some() {} };
    if tokio::time::timeout(std::time::Duration::from_secs(30), drain)
        .await
        .is_err()
    {
        eprintln!(
            "[djangors] graceful shutdown timed out after 30s, aborting remaining connections"
        );
        join_set.abort_all();
    }
}

/// Serve a single TCP connection using the router.
///
/// Wraps the stream in [`hyper_util::rt::TokioIo`] and drives it with
/// hyper's HTTP/1.1 connection builder against a `hyper::service::service_fn`
/// that calls [`Router::dispatch_debug`] — this (rather than the
/// `tower::Service` impl used for middleware composition) is what makes the
/// Django-style debug page / production error page actually get served,
/// based on `settings.debug`.
async fn serve_connection(
    stream: tokio::net::TcpStream,
    router: Router,
    debug: bool,
) -> Result<(), DjangorsError> {
    let io = hyper_util::rt::TokioIo::new(stream);
    let svc = hyper::service::service_fn(move |req| {
        let router = router.clone();
        async move { Ok::<_, std::convert::Infallible>(router.dispatch_debug(req, debug).await) }
    });

    hyper::server::conn::http1::Builder::new()
        .serve_connection(io, svc)
        .await
        .map_err(|e| DjangorsError::Internal(format!("Connection error: {e}")))?;

    Ok(())
}

/// Serve a single TCP connection using a custom layered service.
async fn serve_connection_service<S>(
    stream: tokio::net::TcpStream,
    service: S,
) -> Result<(), DjangorsError>
where
    S: tower::Service<
            hyper::Request<hyper::body::Incoming>,
            Response = hyper::Response<http_body_util::Full<bytes::Bytes>>,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let io = hyper_util::rt::TokioIo::new(stream);
    let svc = hyper::service::service_fn(move |req| {
        let mut service = service.clone();
        async move {
            use tower::ServiceExt;
            let ready_svc = service.ready().await.unwrap();
            ready_svc.call(req).await
        }
    });

    hyper::server::conn::http1::Builder::new()
        .serve_connection(io, svc)
        .await
        .map_err(|e| DjangorsError::Internal(format!("Connection error: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_params::PathParams;
    use crate::request::Request;
    use crate::response::Response;
    use hyper::StatusCode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn hello_handler(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::OK, "Hello from Djangors!"))
    }

    async fn slow_handler(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        Ok(Response::text(StatusCode::OK, "Slow response"))
    }

    #[tokio::test]
    async fn real_socket_request_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let settings = DjangorsSettings {
            host: "127.0.0.1".to_string(),
            port: addr.port(),
            ..Default::default()
        };
        let router = Router::new().get("/hello", hello_handler);
        let app = Djangors::new(settings, router);

        tokio::spawn(async move {
            app.serve(listener).await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let request = "GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        stream.write_all(request.as_bytes()).await.unwrap();

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8(buf).unwrap();

        assert!(
            response.contains("200 OK"),
            "Expected 200 OK, got: {response}"
        );
        assert!(
            response.contains("Hello from Djangors!"),
            "Expected body text, got: {response}"
        );
    }

    #[tokio::test]
    async fn real_socket_layered_middleware_response() {
        use crate::middleware::{CsrfLayer, SecurityHeadersLayer};
        use crate::router::RouterService;
        use tower::ServiceBuilder;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let settings = DjangorsSettings {
            host: "127.0.0.1".to_string(),
            port: addr.port(),
            ..Default::default()
        };
        let router = Router::new().get("/hello", hello_handler);

        let router_service = RouterService::new(router, settings.debug);

        let service = ServiceBuilder::new()
            .layer(SecurityHeadersLayer)
            .layer(CsrfLayer::new())
            .service(router_service);

        let app = Djangors::new(settings, Router::new());

        tokio::spawn(async move {
            app.serve_service(listener, service).await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let request = "GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        stream.write_all(request.as_bytes()).await.unwrap();

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8(buf).unwrap();

        assert!(
            response.contains("200 OK"),
            "Expected 200 OK, got: {response}"
        );
        assert!(
            response.contains("Hello from Djangors!"),
            "Expected body text, got: {response}"
        );

        let response_lower = response.to_lowercase();
        assert!(
            response_lower.contains("x-frame-options: deny"),
            "Expected 'x-frame-options: deny' header, got: {response}"
        );
        assert!(
            response_lower.contains("set-cookie: csrftoken="),
            "Expected 'set-cookie: csrftoken=' header, got: {response}"
        );
    }

    #[tokio::test]
    async fn graceful_shutdown_drains_in_flight_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        let settings = DjangorsSettings {
            host: "127.0.0.1".to_string(),
            port: addr.port(),
            ..Default::default()
        };
        let router = Router::new().get("/slow", slow_handler);
        let app = Djangors::new(settings, router);

        let server = tokio::spawn(async move {
            app.serve_with_shutdown(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let response_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            String::from_utf8(buf).unwrap()
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let shutdown_started = tokio::time::Instant::now();
        shutdown_tx.send(()).unwrap();
        server.await.unwrap();
        let shutdown_elapsed = shutdown_started.elapsed();

        assert!(
            shutdown_elapsed >= std::time::Duration::from_millis(250),
            "server returned before the in-flight request drained: {shutdown_elapsed:?}"
        );
        let response = response_task.await.unwrap();
        assert!(
            response.contains("200 OK"),
            "Expected 200 OK, got: {response}"
        );
        assert!(
            response.contains("Slow response"),
            "Expected complete response body, got: {response}"
        );
    }

    #[tokio::test]
    async fn bind_fails_on_invalid_settings() {
        let settings = DjangorsSettings {
            port: 0,
            ..Default::default()
        };
        let router = Router::new();
        let app = Djangors::new(settings, router);
        let result = app.bind().await;
        assert!(result.is_err());
    }
}
