#![deny(missing_docs)]
//! Small, in-process testing helpers for Djangors applications.

use bytes::Bytes;
use djangors_core::{AppState, Request, Response, Router};
use djangors_db::{config::DatabaseConfig, Database, DbError};
use djangors_sessions::Session;
use hyper::http::{
    header::CONTENT_TYPE, Extensions, HeaderMap, HeaderValue, Method, StatusCode, Uri,
};
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Errors returned by test database operations.
#[derive(Debug, thiserror::Error)]
pub enum TestError {
    /// JSON deserialization failed.
    #[error("JSON deserialization error: {0}")]
    Json(#[from] serde_json::Error),
    /// A database-level error occurred.
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    /// An ORM-level error occurred.
    #[error("ORM error: {0}")]
    Orm(#[from] djangors_orm::OrmError),
    /// A SQLx-level error occurred.
    #[error("SQLx error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// A generic error.
    #[error("{0}")]
    Other(String),
}

/// A unique database name counter, shared across the process.
static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replaces the database name component in a `postgres://` URL string.
fn replace_db_name(url: &str, new_db: &str) -> String {
    let scheme_end = url.find("://").map(|i| i + 3).unwrap_or(0);
    let after_scheme = &url[scheme_end..];
    if let Some(relative_slash) = after_scheme.rfind('/') {
        let slash_pos = scheme_end + relative_slash;
        let after_slash = &url[slash_pos + 1..];
        match after_slash.find('?') {
            Some(q) => format!("{}/{}{}", &url[..slash_pos], new_db, &after_slash[q..]),
            None => format!("{}/{}", &url[..slash_pos], new_db),
        }
    } else {
        format!("{}/{}", url.trim_end_matches('/'), new_db)
    }
}

fn generate_db_name() -> String {
    let pid = std::process::id();
    let count = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("djangors_test_{pid}_{count}_{nanos}")
}

/// Which backend a test database is provisioned against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestBackend {
    /// PostgreSQL database backend.
    Postgres,
    /// SQLite database backend (in-memory).
    Sqlite,
}

impl TestBackend {
    /// Detects the active test backend based on environment variables:
    /// 1. `TEST_BACKEND=sqlite` -> `TestBackend::Sqlite`
    /// 2. `TEST_BACKEND=postgres` -> `TestBackend::Postgres`
    /// 3. If `TEST_BACKEND` is unset: `DATABASE_URL` set -> `Postgres`, unset -> `Sqlite`.
    pub fn current() -> Self {
        if let Ok(var) = std::env::var("TEST_BACKEND") {
            match var.to_lowercase().as_str() {
                "sqlite" => return TestBackend::Sqlite,
                "postgres" => return TestBackend::Postgres,
                _ => {}
            }
        }
        if std::env::var("DATABASE_URL").is_ok() {
            TestBackend::Postgres
        } else {
            TestBackend::Sqlite
        }
    }
}

/// A thin database fixture. It intentionally does not provide transactional rollback:
/// ORM querysets currently require `&Database` and execute directly through its pool.
///
/// The primary cleanup mechanism for isolated databases is the explicit
/// [`TestDatabase::cleanup`] method. As a defensive fallback, the `Drop` impl will attempt
/// a best-effort `DROP DATABASE` via an ambient tokio runtime handle if one is available.
/// Always prefer calling `cleanup().await` explicitly in test code.
pub struct TestDatabase {
    database: Database,
    db_name: Option<String>,
    admin_base_url: String,
    is_isolated: bool,
}

impl TestDatabase {
    /// Provisions a database for one test, honouring `backend`.
    pub async fn new_for_backend(backend: TestBackend) -> Result<Self, DbError> {
        match backend {
            TestBackend::Sqlite => {
                // SQLite in-memory database: every pooled connection is a separate database when using sqlite::memory:.
                // Therefore max_connections MUST be set to 1 so that setup DDL executed on one connection
                // is visible to subsequent queries on the same pooled connection handle.
                let config = DatabaseConfig::new("sqlite::memory:".to_string())
                    .max_connections(1)
                    .min_connections(1);
                let database = Database::connect(&config).await?;
                Ok(Self {
                    database,
                    db_name: None,
                    admin_base_url: String::new(),
                    is_isolated: true,
                })
            }
            TestBackend::Postgres => {
                let url = std::env::var("DATABASE_URL").map_err(|_| {
                    DbError::ConnectionFailed("DATABASE_URL environment variable is not set".into())
                })?;
                Self::isolated_url(&url).await
            }
        }
    }

    /// Connects to the database specified by the environment configuration (`TEST_BACKEND` or `DATABASE_URL`).
    pub async fn connect() -> Result<Self, DbError> {
        match TestBackend::current() {
            TestBackend::Sqlite => Self::new_for_backend(TestBackend::Sqlite).await,
            TestBackend::Postgres => {
                let url = std::env::var("DATABASE_URL").map_err(|_| {
                    DbError::ConnectionFailed("DATABASE_URL environment variable is not set".into())
                })?;
                Self::connect_url(&url).await
            }
        }
    }

    /// Connects to a database at the specified URL string.
    pub async fn connect_url(url: &str) -> Result<Self, DbError> {
        Ok(Self {
            database: Database::connect(&DatabaseConfig::new(url.to_string())).await?,
            db_name: None,
            admin_base_url: String::new(),
            is_isolated: false,
        })
    }

    /// Returns a reference to the underlying [`Database`].
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Executes raw DDL SQL to create a table.
    pub async fn create_table(&self, sql: &str) -> Result<(), sqlx::Error> {
        self.database.conn().execute(sql, &[]).await.map(|_| ())
    }

    /// Drops a table by name if it exists.
    pub async fn drop_table(&self, name: &str) -> Result<(), sqlx::Error> {
        let sql = format!("DROP TABLE IF EXISTS \"{name}\"");
        self.database.conn().execute(&sql, &[]).await.map(|_| ())
    }

    /// Drops each table in `tables` sequentially.
    pub async fn reset(&self, tables: &[&str]) -> Result<(), sqlx::Error> {
        for table in tables {
            self.drop_table(table).await?;
        }
        Ok(())
    }

    /// Creates a uniquely-named throwaway database, connects to it, and returns a
    /// [`TestDatabase`] backed by that isolated database.
    ///
    /// The base connection is determined by `TEST_BACKEND` or `DATABASE_URL`.
    pub async fn isolated() -> Result<Self, DbError> {
        match TestBackend::current() {
            TestBackend::Sqlite => Self::new_for_backend(TestBackend::Sqlite).await,
            TestBackend::Postgres => {
                let url = std::env::var("DATABASE_URL").map_err(|_| {
                    DbError::ConnectionFailed("DATABASE_URL environment variable is not set".into())
                })?;
                Self::isolated_url(&url).await
            }
        }
    }

    /// Like [`TestDatabase::isolated`] but connects to the given URL string for the
    /// base connection instead of reading `DATABASE_URL`.
    pub async fn isolated_url(url: &str) -> Result<Self, DbError> {
        if matches!(
            djangors_db::Dialect::from_url(url),
            djangors_db::Dialect::Sqlite
        ) {
            let config = DatabaseConfig::new("sqlite::memory:".to_string())
                .max_connections(1)
                .min_connections(1);
            let database = Database::connect(&config).await?;
            return Ok(Self {
                database,
                db_name: None,
                admin_base_url: String::new(),
                is_isolated: true,
            });
        }

        let db_name = generate_db_name();

        let admin_url = replace_db_name(url, "postgres");
        let admin_db = Database::connect(&DatabaseConfig::new(admin_url)).await?;

        let create_sql = format!("CREATE DATABASE \"{db_name}\"");
        sqlx::query(sqlx::AssertSqlSafe(create_sql))
            .execute(admin_db.pool())
            .await
            .map_err(|e| DbError::ConnectionFailed(e.to_string()))?;

        drop(admin_db);

        let isolated_url = replace_db_name(url, &db_name);
        let database = Database::connect(&DatabaseConfig::new(isolated_url)).await?;

        Ok(Self {
            database,
            db_name: Some(db_name),
            admin_base_url: url.to_string(),
            is_isolated: true,
        })
    }

    /// Explicitly drops the throwaway database backing this [`TestDatabase`].
    ///
    /// This is the **primary cleanup mechanism** for isolated databases.
    pub async fn cleanup(self) -> Result<(), TestError> {
        if self.database.dialect() == djangors_db::Dialect::Sqlite {
            // SQLite in-memory database dies automatically with the connection handle.
            return Ok(());
        }

        let db_name = match self.db_name.as_ref() {
            Some(name) => name.clone(),
            None => return Ok(()),
        };
        let admin_base_url = self.admin_base_url.clone();

        drop(self);

        let admin_url = replace_db_name(&admin_base_url, "postgres");
        let admin_db = Database::connect(&DatabaseConfig::new(admin_url))
            .await
            .map_err(TestError::Db)?;

        sqlx::query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
        )
        .bind(&db_name)
        .execute(admin_db.pool())
        .await?;

        let drop_sql = format!("DROP DATABASE IF EXISTS \"{db_name}\"");
        sqlx::query(sqlx::AssertSqlSafe(drop_sql))
            .execute(admin_db.pool())
            .await?;

        Ok(())
    }

    /// Loads rows into a freshly-created table from a JSON string.
    ///
    /// Deserializes `json` as a `Vec<T>` and calls [`QuerySet::bulk_create`] to insert
    /// every element. `T` must implement both [`djangors_orm::Model`] (to satisfy the ORM
    /// layer) and [`serde::de::DeserializeOwned`] (to parse the JSON).
    ///
    /// Returns the deserialized items. Primary-key fields that are auto-generated will
    /// not be populated in the returned values; use the normal `QuerySet` API to query
    /// the rows back from the database if PKs are needed.
    pub async fn load_fixtures<
        T: djangors_orm::Model + djangors_orm::FromRow + serde::de::DeserializeOwned,
    >(
        &self,
        json: &str,
    ) -> Result<Vec<T>, TestError> {
        let items: Vec<T> = serde_json::from_str(json)?;
        djangors_orm::QuerySet::<T>::bulk_create(self.database(), &items).await?;
        Ok(items)
    }
}

impl Drop for TestDatabase {
    /// Defensive fallback: if this is an isolated database and a tokio runtime handle
    /// is available, spawn a best-effort `DROP DATABASE`. The explicit
    /// [`TestDatabase::cleanup`] method is the primary mechanism; this is only a
    /// safety net for tests that forget to call it.
    fn drop(&mut self) {
        if !self.is_isolated || self.database.dialect() == djangors_db::Dialect::Sqlite {
            return;
        }
        if let Some(ref db_name) = self.db_name {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let db_name = db_name.clone();
                let admin_base_url = self.admin_base_url.clone();
                handle.spawn(async move {
                    let admin_url = replace_db_name(&admin_base_url, "postgres");
                    if let Ok(admin_db) =
                        Database::connect(&DatabaseConfig::new(admin_url)).await
                    {
                        let _ = sqlx::query(
                            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
                        )
                        .bind(&db_name)
                        .execute(admin_db.pool())
                        .await;
                        let drop_sql = format!("DROP DATABASE IF EXISTS \"{db_name}\"");
                        let _ = sqlx::query(sqlx::AssertSqlSafe(drop_sql)).execute(admin_db.pool()).await;
                    }
                });
            }
        }
    }
}

/// A held Postgres session-level advisory lock, acquired via
/// [`acquire_cross_process_lock`]. Released by calling [`CrossProcessLock::release`]
/// explicitly (Rust can't run async code in `Drop`, so this can't release itself
/// automatically the way [`TestDatabase`]'s in-process guards can) - an
/// un-released lock is held until its underlying connection closes, which
/// happens naturally when the test process exits, but explicit release lets a
/// long-running process (a real `dj test` run covering many tests) free the
/// lock promptly instead of holding it for the rest of the run.
pub struct CrossProcessLock {
    conn: sqlx::pool::PoolConnection<sqlx::Postgres>,
    key_name: String,
}

impl CrossProcessLock {
    /// Releases the lock. The connection this lock was acquired on must be the
    /// one that releases it - `pg_advisory_unlock` is a per-session operation,
    /// not per-pool - which is exactly why this type holds a single dedicated
    /// [`sqlx::pool::PoolConnection`] instead of borrowing from the pool anew for
    /// each query the way most of this codebase's other queries do.
    pub async fn release(mut self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
            .bind(&self.key_name)
            .execute(&mut *self.conn)
            .await?;
        Ok(())
    }
}

/// Acquires a Postgres session-level advisory lock keyed by `name`, blocking
/// until any other session holding the same-named lock releases it.
///
/// Unlike an in-process `std::sync::Mutex`/`tokio::sync::Mutex` (which only
/// coordinates tasks within one compiled test *binary*), this coordinates
/// across entirely separate OS processes connected to the same
/// Postgres server - the actual mechanism needed to prevent two different
/// crates' test binaries (e.g. `djangors-admin` and `djangors-auth`, both racing
/// to `DROP`/`CREATE` the same `auth_user` table) from colliding when `cargo
/// test --workspace` runs them as concurrent processes. Confirmed reproducible
/// without this: running `djangors-admin` and `djangors-auth`'s suites
/// concurrently produced a real `relation "auth_user" already exists` (42P07)
/// in one and `relation "auth_user" does not exist` (42P01) in the other.
///
/// Deliberately holds a single dedicated connection (via
/// [`sqlx::Pool::acquire`]) for the lock's entire lifetime rather than using
/// `db.pool()` directly - a session-level advisory lock is tied to the specific
/// connection that acquired it, and a connection pool hands out whichever
/// connection happens to be free for each query, so acquiring/releasing through
/// the pool directly would not reliably lock/unlock the same session.
pub async fn acquire_cross_process_lock(
    db: &Database,
    name: &str,
) -> Result<CrossProcessLock, sqlx::Error> {
    let mut conn = db.pool().acquire().await?;
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(name)
        .execute(&mut *conn)
        .await?;
    Ok(CrossProcessLock {
        conn,
        key_name: name.to_string(),
    })
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

    // --- Isolated database tests (require DATABASE_URL to be set) ---

    #[tokio::test]
    async fn isolated_databases_are_separate() {
        if TestBackend::current() == TestBackend::Sqlite {
            // Requires Postgres current_database() function and separate named database catalog.
            return;
        }

        let db1 = TestDatabase::isolated()
            .await
            .expect("db1 isolation failed");
        let db2 = TestDatabase::isolated()
            .await
            .expect("db2 isolation failed");

        let name1: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(db1.database().pool())
            .await
            .expect("query current_database from db1");
        let name2: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(db2.database().pool())
            .await
            .expect("query current_database from db2");

        assert_ne!(name1, name2, "isolated databases must have different names");

        db1.create_table(
            "CREATE TABLE IF NOT EXISTS shared_test (id SERIAL PRIMARY KEY, val TEXT)",
        )
        .await
        .expect("create table in db1");
        db2.create_table(
            "CREATE TABLE IF NOT EXISTS shared_test (id SERIAL PRIMARY KEY, val TEXT)",
        )
        .await
        .expect("create table in db2");

        sqlx::query("INSERT INTO shared_test (val) VALUES ($1)")
            .bind("from db1")
            .execute(db1.database().pool())
            .await
            .expect("insert into db1");
        sqlx::query("INSERT INTO shared_test (val) VALUES ($1)")
            .bind("from db2")
            .execute(db2.database().pool())
            .await
            .expect("insert into db2");

        let count1: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM shared_test")
            .fetch_one(db1.database().pool())
            .await
            .expect("count in db1");
        let count2: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM shared_test")
            .fetch_one(db2.database().pool())
            .await
            .expect("count in db2");

        assert_eq!(count1, 1, "db1 must see exactly 1 row");
        assert_eq!(count2, 1, "db2 must see exactly 1 row");

        db1.cleanup().await.expect("cleanup db1");
        db2.cleanup().await.expect("cleanup db2");
    }

    #[tokio::test]
    async fn cleanup_actually_drops_database() {
        if TestBackend::current() == TestBackend::Sqlite {
            // Requires Postgres catalog pg_database and server drop database functionality.
            return;
        }

        let db = TestDatabase::isolated()
            .await
            .expect("isolated for cleanup test");
        let db_name = db.db_name.as_ref().cloned().unwrap();
        let admin_base_url = db.admin_base_url.clone();

        let admin_url = replace_db_name(&admin_base_url, "postgres");
        let admin_db = Database::connect(&DatabaseConfig::new(admin_url))
            .await
            .expect("admin connect for cleanup pre-check");

        let exists_before: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
                .bind(&db_name)
                .fetch_one(admin_db.pool())
                .await
                .expect("check database exists before cleanup");
        assert!(exists_before, "database must exist before cleanup");

        drop(admin_db);
        db.cleanup().await.expect("cleanup should succeed");

        let admin_url = replace_db_name(&admin_base_url, "postgres");
        let admin_db = Database::connect(&DatabaseConfig::new(admin_url))
            .await
            .expect("admin connect for cleanup post-check");

        let exists_after: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
                .bind(&db_name)
                .fetch_one(admin_db.pool())
                .await
                .expect("check database exists after cleanup");
        assert!(!exists_after, "database must be gone after cleanup");
    }

    #[tokio::test]
    async fn fixtures_loader_inserts_queryable_rows() {
        use djangors_orm::error::FromRow;
        use djangors_orm::expr::Value;
        use djangors_orm::{DefaultValue, FieldKind, FieldMeta, Model, ModelMeta, QuerySet};
        use serde::Deserialize;

        #[derive(Debug, Clone, Deserialize)]
        struct FixturesTestModel {
            #[serde(default)]
            id: i64,
            name: String,
            value: i64,
        }

        impl Model for FixturesTestModel {
            fn meta() -> &'static ModelMeta {
                use std::sync::OnceLock;
                static META: OnceLock<ModelMeta> = OnceLock::new();
                META.get_or_init(|| ModelMeta {
                    struct_name: "FixturesTestModel",
                    app_label: "test_app",
                    table_name: "test_fixtures_model",
                    fields: &[
                        FieldMeta {
                            name: "id",
                            column_name: "id",
                            kind: FieldKind::BigInt,
                            primary_key: true,
                            auto: true,
                            nullable: false,
                            unique: true,
                            db_index: false,
                            default: DefaultValue::None,
                            max_length: None,
                            verbose_name: None,
                            help_text: None,
                            choices: &[],
                        },
                        FieldMeta {
                            name: "name",
                            column_name: "name",
                            kind: FieldKind::Text,
                            primary_key: false,
                            auto: false,
                            nullable: false,
                            unique: false,
                            db_index: false,
                            default: DefaultValue::None,
                            max_length: None,
                            verbose_name: None,
                            help_text: None,
                            choices: &[],
                        },
                        FieldMeta {
                            name: "value",
                            column_name: "value",
                            kind: FieldKind::BigInt,
                            primary_key: false,
                            auto: false,
                            nullable: false,
                            unique: false,
                            db_index: false,
                            default: DefaultValue::None,
                            max_length: None,
                            verbose_name: None,
                            help_text: None,
                            choices: &[],
                        },
                    ],
                    relations: &[],
                    indexes: &[],
                    unique_together: &[],
                    ordering: &[],
                })
            }

            fn field_values(&self) -> Vec<(&'static str, Value)> {
                vec![
                    ("id", Value::I64(self.id)),
                    ("name", Value::Text(self.name.clone())),
                    ("value", Value::I64(self.value)),
                ]
            }

            fn field_names() -> Vec<&'static str> {
                vec!["id", "name", "value"]
            }
        }

        impl FromRow for FixturesTestModel {
            fn from_row(row: &djangors_orm::DbRow) -> Result<Self, djangors_orm::OrmError> {
                Ok(Self {
                    id: row.try_i64_by_name("id")?.ok_or_else(|| {
                        djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound)
                    })?,
                    name: row.try_string_by_name("name")?.ok_or_else(|| {
                        djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound)
                    })?,
                    value: row.try_i64_by_name("value")?.ok_or_else(|| {
                        djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound)
                    })?,
                })
            }
        }

        let db = TestDatabase::isolated()
            .await
            .expect("isolated for fixtures test");

        let auto_pk = db.database().dialect().auto_pk_type();
        db.create_table(
            &format!("CREATE TABLE test_fixtures_model (id {auto_pk}, name TEXT NOT NULL, value BIGINT NOT NULL)"),
        )
        .await
        .expect("create fixtures table");

        let json = r#"[
            {"name": "Alice", "value": 100},
            {"name": "Bob", "value": 200}
        ]"#;

        let items: Vec<FixturesTestModel> = db
            .load_fixtures(json)
            .await
            .expect("load_fixtures should succeed");
        assert_eq!(items.len(), 2, "must deserialize 2 items");

        let queried: Vec<FixturesTestModel> =
            QuerySet::<FixturesTestModel>::all(&QuerySet::new(), db.database())
                .await
                .expect("query all from fixtures table");

        assert_eq!(queried.len(), 2, "must read back 2 rows");
        assert_eq!(queried[0].name, "Alice");
        assert_eq!(queried[0].value, 100);
        assert_eq!(queried[1].name, "Bob");
        assert_eq!(queried[1].value, 200);

        db.cleanup().await.expect("cleanup fixtures db");
    }

    #[tokio::test]
    async fn cross_process_lock_genuinely_serializes_two_separate_connections() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };
        // Two independent Database instances (their own separate connection
        // pools, matching two separate OS processes at the level that actually
        // matters here - Postgres session identity) racing for the SAME named
        // lock. A `tokio::sync::Barrier` forces both to request the lock at the
        // same instant, matching the rigor used elsewhere in this codebase to
        // rule out scheduling luck (see djangors-tasks' SKIP LOCKED
        // investigation) - not just "usually passes in practice".
        let db_a = Database::connect(&DatabaseConfig::new(url.clone()))
            .await
            .expect("connect db_a");
        let db_b = Database::connect(&DatabaseConfig::new(url))
            .await
            .expect("connect db_b");

        let lock_name = format!("test_cross_process_lock_{}", generate_db_name());
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));

        let events = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<&'static str>::new()));

        let barrier_a = barrier.clone();
        let events_a = events.clone();
        let lock_name_a = lock_name.clone();
        let task_a = tokio::spawn(async move {
            barrier_a.wait().await;
            let lock = acquire_cross_process_lock(&db_a, &lock_name_a)
                .await
                .unwrap();
            events_a.lock().await.push("a_acquired");
            // Hold the lock long enough that, if the other connection were NOT
            // genuinely blocked, it would race in and acquire during this
            // window instead of waiting.
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            events_a.lock().await.push("a_released");
            lock.release().await.unwrap();
        });

        let barrier_b = barrier.clone();
        let events_b = events.clone();
        let lock_name_b = lock_name.clone();
        let task_b = tokio::spawn(async move {
            barrier_b.wait().await;
            let lock = acquire_cross_process_lock(&db_b, &lock_name_b)
                .await
                .unwrap();
            events_b.lock().await.push("b_acquired");
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            events_b.lock().await.push("b_released");
            lock.release().await.unwrap();
        });

        task_a.await.unwrap();
        task_b.await.unwrap();

        let log = events.lock().await;
        // Whichever of A/B actually wins the race for the lock is legitimately
        // non-deterministic (that's the point - both requested it at the same
        // instant), but real mutual exclusion means the two connections' held
        // periods can never overlap: the log must be exactly
        // [X_acquired, X_released, Y_acquired, Y_released], never anything
        // where the second acquire appears before the first release.
        assert_eq!(log.len(), 4, "log: {log:?}");
        let first = log[0].strip_suffix("_acquired").expect("log: {log:?}");
        assert_eq!(log[1], format!("{first}_released"), "log: {log:?}");
        let second = log[2].strip_suffix("_acquired").expect("log: {log:?}");
        assert_eq!(log[3], format!("{second}_released"), "log: {log:?}");
        assert_ne!(
            first, second,
            "log should show both connections, not the same one twice - log: {log:?}"
        );
    }
}
