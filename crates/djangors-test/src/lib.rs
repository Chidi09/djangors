#![deny(missing_docs)]
//! Small, in-process testing helpers for Djangors applications.

use bytes::Bytes;
use djangors_core::{AppState, Request, Response, Router};
use djangors_db::{config::DatabaseConfig, Database, DbError};
use djangors_sessions::Session;
use hyper::http::{
    header::CONTENT_TYPE, Extensions, HeaderMap, HeaderValue, Method, StatusCode, Uri,
};

/// An in-process client for a Djangors router.
#[derive(Clone)]
pub struct TestClient {
    router: Router,
}

impl TestClient {
    /// Creates a new `TestClient` wrapping `router`.
    pub fn new(router: Router) -> Self {
        Self { router }
    }

    /// Prepares a GET request to `path`.
    pub fn get(&self, path: &str) -> RequestBuilder {
        RequestBuilder::new(self.router.clone(), Method::GET, path)
    }

    /// Prepares a POST request to `path` with urlencoded form pairs.
    pub fn post_form(&self, path: &str, pairs: &[(&str, &str)]) -> RequestBuilder {
        let body = serde_urlencoded::to_string(pairs).expect("form pairs should encode");
        let mut builder = RequestBuilder::new(self.router.clone(), Method::POST, path);
        builder.headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        builder.body = Bytes::from(body);
        builder
    }
}

/// Builder for constructing in-process test HTTP requests.
pub struct RequestBuilder {
    router: Router,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    extensions: Extensions,
    state: AppState,
}

impl RequestBuilder {
    fn new(router: Router, method: Method, path: &str) -> Self {
        Self {
            router,
            method,
            uri: path.parse().expect("test request path must be a valid URI"),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            extensions: Extensions::new(),
            state: AppState::new(),
        }
    }

    /// Attaches a session extension to the test request.
    pub fn with_session(mut self, session: Session) -> Self {
        self.extensions.insert(session);
        self
    }

    /// Attaches state data to the test request.
    pub fn with_state<T: Send + Sync + 'static>(mut self, state: T) -> Self {
        self.state = self.state.insert(state);
        self
    }

    /// Sends the test request through the router and returns a [`TestResponse`].
    pub async fn send(self) -> TestResponse {
        let request = Request::new(self.method, self.uri, self.headers, self.body)
            .with_extensions(self.extensions)
            .with_state(self.state);
        let response = match self.router.handle(request).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        TestResponse { response }
    }
}

/// Wrapper around an HTTP response for test assertions.
pub struct TestResponse {
    response: Response,
}

impl TestResponse {
    /// Asserts that the response status matches `expected`.
    pub fn assert_status(self, expected: StatusCode) -> Self {
        assert_eq!(
            self.response.status(),
            expected,
            "expected status {expected}, got {}; body: {}",
            self.response.status(),
            self.body_str()
        );
        self
    }

    /// Asserts that the response body text contains `needle`.
    pub fn assert_contains(&self, needle: &str) -> &Self {
        let body = self.body_str();
        assert!(
            body.contains(needle),
            "expected response body to contain {needle:?}; body: {body}"
        );
        self
    }

    /// Returns the response body as a UTF-8 string.
    pub fn body_str(&self) -> String {
        String::from_utf8_lossy(self.response.body()).into_owned()
    }

    /// Returns the HTTP status code of the response.
    pub fn status(&self) -> StatusCode {
        self.response.status()
    }
}

/// A thin database fixture. It intentionally does not provide transactional rollback:
/// ORM querysets currently require `&Database` and execute directly through its pool.
pub struct TestDatabase {
    database: Database,
}

impl TestDatabase {
    /// Connects to the database specified by the `DATABASE_URL` environment variable.
    pub async fn connect() -> Result<Self, DbError> {
        let url = std::env::var("DATABASE_URL").map_err(|_| {
            DbError::ConnectionFailed("DATABASE_URL environment variable is not set".into())
        })?;
        Self::connect_url(&url).await
    }

    /// Connects to a database at the specified URL string.
    pub async fn connect_url(url: &str) -> Result<Self, DbError> {
        Ok(Self {
            database: Database::connect(&DatabaseConfig::new(url.to_string())).await?,
        })
    }

    /// Returns a reference to the underlying [`Database`].
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Executes raw DDL SQL to create a table.
    pub async fn create_table(&self, sql: &str) -> Result<(), sqlx::Error> {
        sqlx::QueryBuilder::<sqlx::Postgres>::new(sql)
            .build()
            .execute(self.database.pool())
            .await
            .map(|_| ())
    }

    /// Drops a table by name if it exists.
    pub async fn drop_table(&self, name: &str) -> Result<(), sqlx::Error> {
        sqlx::QueryBuilder::<sqlx::Postgres>::new(format!("DROP TABLE IF EXISTS {name}"))
            .build()
            .execute(self.database.pool())
            .await
            .map(|_| ())
    }

    /// Drops each table in `tables` sequentially.
    pub async fn reset(&self, tables: &[&str]) -> Result<(), sqlx::Error> {
        for table in tables {
            self.drop_table(table).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get_handler(
        _: Request,
        _: djangors_core::PathParams,
    ) -> Result<Response, djangors_core::DjangorsError> {
        Ok(Response::text(StatusCode::OK, "hello from test"))
    }

    async fn post_handler(
        req: Request,
        _: djangors_core::PathParams,
    ) -> Result<Response, djangors_core::DjangorsError> {
        let session = req.ext::<Session>().expect("session missing");
        let body = String::from_utf8_lossy(req.body_bytes().await);
        Ok(Response::text(
            StatusCode::CREATED,
            &format!(
                "{}:{}",
                session.get::<String>("user").unwrap_or_default(),
                body
            ),
        ))
    }

    fn client() -> TestClient {
        TestClient::new(
            Router::new()
                .get("/hello", get_handler)
                .post("/submit", post_handler),
        )
    }

    #[tokio::test]
    async fn get_and_post_helpers_work() {
        client()
            .get("/hello")
            .send()
            .await
            .assert_status(StatusCode::OK)
            .assert_contains("hello from test");
        let session = Session::new_empty();
        session.set("user", "Ada");
        client()
            .post_form("/submit", &[("name", "Grace Hopper")])
            .with_session(session)
            .send()
            .await
            .assert_status(StatusCode::CREATED)
            .assert_contains("Ada:name=Grace+Hopper");
    }

    #[tokio::test]
    #[should_panic(expected = "expected response body to contain")]
    async fn assert_contains_panics_on_mismatch() {
        client()
            .get("/hello")
            .send()
            .await
            .assert_contains("missing");
    }
}
