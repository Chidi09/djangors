use tokio::net::TcpListener;

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
        loop {
            match listener.accept().await {
                Ok((stream, _peer_addr)) => {
                    let router = self.router.clone();
                    tokio::task::spawn(async move {
                        if let Err(e) = serve_connection(stream, router).await {
                            eprintln!("Connection error: {e}");
                        }
                    });
                }
                Err(e) => {
                    eprintln!("Accept error: {e}");
                }
            }
        }
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
}

/// Serve a single TCP connection using the router.
///
/// Wraps the stream in [`hyper_util::rt::TokioIo`], adapts the router
/// (a `tower::Service`) to a `hyper::service::Service` via
/// [`hyper_util::service::TowerToHyperService`], and drives it with
/// hyper's HTTP/1.1 connection builder.
async fn serve_connection(
    stream: tokio::net::TcpStream,
    router: Router,
) -> Result<(), DjangorsError> {
    let io = hyper_util::rt::TokioIo::new(stream);
    let svc = hyper_util::service::TowerToHyperService::new(router);

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
