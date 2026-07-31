use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Executor, PgConnection, PgPool, SqlitePool};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::DatabaseConfig;
use crate::dialect::Dialect;
use crate::error::DbError;
use crate::executor::Conn;

/// A pinned, boxed future that sends across threads, used for async callbacks
/// that borrow the database connection reference.
#[doc(hidden)]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Connection pool manager for Djangors backends (PostgreSQL or SQLite).
#[derive(Debug, Clone)]
pub enum Database {
    /// PostgreSQL connection pool wrapper.
    Pg {
        /// Underlying SQLx PostgreSQL pool.
        pool: PgPool,
        /// Recorded query counter.
        query_count: Arc<AtomicUsize>,
    },
    /// SQLite connection pool wrapper.
    Sqlite {
        /// Underlying SQLx SQLite pool.
        pool: SqlitePool,
        /// Recorded query counter.
        query_count: Arc<AtomicUsize>,
    },
}

/// Supported isolation levels for database transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    /// Read Committed isolation level (PostgreSQL default).
    ReadCommitted,
    /// Repeatable Read isolation level.
    RepeatableRead,
    /// Serializable isolation level.
    Serializable,
}

/// Helper function to map [`IsolationLevel`] to the corresponding SQL command.
#[doc(hidden)]
pub fn isolation_level_sql(level: IsolationLevel) -> &'static str {
    match level {
        IsolationLevel::ReadCommitted => "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
        IsolationLevel::RepeatableRead => "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
        IsolationLevel::Serializable => "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
    }
}

impl Database {
    /// Connect to the database using the given configuration, building a connection pool.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, DbError> {
        let url = config.url.trim();
        let is_sqlite = matches!(Dialect::from_url(url), Dialect::Sqlite);

        if is_sqlite {
            let mut options = SqlitePoolOptions::new()
                .max_connections(config.max_connections)
                .min_connections(config.min_connections)
                .acquire_timeout(Duration::from_secs(config.connect_timeout_secs));

            if let Some(idle) = config.idle_timeout_secs {
                options = options.idle_timeout(Duration::from_secs(idle));
            }

            let pool = options.connect(url).await.map_err(|e| {
                if matches!(e, sqlx::Error::PoolTimedOut) {
                    DbError::PoolExhausted
                } else {
                    DbError::ConnectionFailed(e.to_string())
                }
            })?;

            Ok(Self::Sqlite {
                pool,
                query_count: Arc::new(AtomicUsize::new(0)),
            })
        } else {
            let mut options = PgPoolOptions::new()
                .max_connections(config.max_connections)
                .min_connections(config.min_connections)
                .acquire_timeout(Duration::from_secs(config.connect_timeout_secs));

            if let Some(idle) = config.idle_timeout_secs {
                options = options.idle_timeout(Duration::from_secs(idle));
            }

            let pool = options.connect(url).await.map_err(|e| {
                if matches!(e, sqlx::Error::PoolTimedOut) {
                    DbError::PoolExhausted
                } else {
                    DbError::ConnectionFailed(e.to_string())
                }
            })?;

            Ok(Self::Pg {
                pool,
                query_count: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    /// Access the underlying PostgreSQL connection pool directly.
    ///
    /// # Panics
    /// Panics if this `Database` handle is connected to a SQLite database.
    pub fn pool(&self) -> &PgPool {
        match self {
            Self::Pg { pool, .. } => pool,
            Self::Sqlite { .. } => {
                panic!("Attempted to access Postgres PgPool on a SQLite Database handle")
            }
        }
    }

    /// Access the underlying SQLite connection pool directly if connected to SQLite.
    pub fn sqlite_pool(&self) -> Option<&SqlitePool> {
        match self {
            Self::Sqlite { pool, .. } => Some(pool),
            Self::Pg { .. } => None,
        }
    }

    /// Returns a [`Conn`] execution handle for this database pool.
    pub fn conn(&self) -> Conn<'_> {
        match self {
            Self::Pg { pool, .. } => Conn::PgPool(pool),
            Self::Sqlite { pool, .. } => Conn::SqlitePool(pool),
        }
    }

    /// Returns the database dialect.
    pub fn dialect(&self) -> Dialect {
        match self {
            Self::Pg { .. } => Dialect::Postgres,
            Self::Sqlite { .. } => Dialect::Sqlite,
        }
    }

    /// Records one SQL query for test observability.
    #[doc(hidden)]
    pub fn record_query(&self) {
        let counter = match self {
            Self::Pg { query_count, .. } => query_count,
            Self::Sqlite { query_count, .. } => query_count,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the number of SQL queries recorded by this database handle.
    #[doc(hidden)]
    pub fn query_count(&self) -> usize {
        let counter = match self {
            Self::Pg { query_count, .. } => query_count,
            Self::Sqlite { query_count, .. } => query_count,
        };
        counter.load(Ordering::Relaxed)
    }

    /// Resets the recorded SQL query count to zero.
    #[doc(hidden)]
    pub fn reset_query_count(&self) {
        let counter = match self {
            Self::Pg { query_count, .. } => query_count,
            Self::Sqlite { query_count, .. } => query_count,
        };
        counter.store(0, Ordering::Relaxed);
    }

    /// Run `f` inside a PostgreSQL database transaction.
    pub async fn transaction<F, T, E>(&self, f: F) -> Result<T, DbError>
    where
        F: for<'c> FnOnce(&'c mut PgConnection) -> BoxFuture<'c, Result<T, E>> + Send,
        T: Send,
        E: Into<DbError> + Send,
    {
        match self {
            Self::Pg { pool, .. } => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                let res = f(&mut tx).await;
                match res {
                    Ok(val) => {
                        tx.commit()
                            .await
                            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = tx.rollback().await;
                        Err(err.into())
                    }
                }
            }
            Self::Sqlite { .. } => Err(DbError::TransactionFailed(
                "PostgreSQL PgConnection transaction called on a SQLite Database".to_string(),
            )),
        }
    }

    /// Run `f` inside a PostgreSQL database transaction with an explicit transaction isolation level.
    pub async fn transaction_with_isolation<F, T, E>(
        &self,
        level: IsolationLevel,
        f: F,
    ) -> Result<T, DbError>
    where
        F: for<'c> FnOnce(&'c mut PgConnection) -> BoxFuture<'c, Result<T, E>> + Send,
        T: Send,
        E: Into<DbError> + Send,
    {
        match self {
            Self::Pg { pool, .. } => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                let sql = isolation_level_sql(level);
                tx.execute(sql)
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;

                let res = f(&mut tx).await;
                match res {
                    Ok(val) => {
                        tx.commit()
                            .await
                            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = tx.rollback().await;
                        Err(err.into())
                    }
                }
            }
            Self::Sqlite { .. } => Err(DbError::TransactionFailed(
                "PostgreSQL transaction_with_isolation called on a SQLite Database".to_string(),
            )),
        }
    }

    /// Run `f` inside a transaction on either backend.
    pub async fn transaction_conn<F, T, E>(&self, f: F) -> Result<T, DbError>
    where
        F: for<'c> FnOnce(&'c mut Conn<'c>) -> BoxFuture<'c, Result<T, E>> + Send,
        T: Send,
        E: Into<DbError> + Send,
    {
        match self {
            Self::Pg { pool, .. } => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                let mut conn = Conn::PgTx(&mut tx);
                let res = f(&mut conn).await;
                match res {
                    Ok(val) => {
                        tx.commit()
                            .await
                            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = tx.rollback().await;
                        Err(err.into())
                    }
                }
            }
            Self::Sqlite { pool, .. } => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                let mut conn = Conn::SqliteTx(&mut tx);
                let res = f(&mut conn).await;
                match res {
                    Ok(val) => {
                        tx.commit()
                            .await
                            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = tx.rollback().await;
                        Err(err.into())
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DB_URL: &str = "postgres://postgres:postgres@localhost/djangors_test";

    #[test]
    fn test_config_builder() {
        let config = DatabaseConfig::new(TEST_DB_URL)
            .max_connections(5)
            .min_connections(2)
            .connect_timeout_secs(15)
            .idle_timeout_secs(Some(30));

        assert_eq!(config.url, TEST_DB_URL);
        assert_eq!(config.max_connections, 5);
        assert_eq!(config.min_connections, 2);
        assert_eq!(config.connect_timeout_secs, 15);
        assert_eq!(config.idle_timeout_secs, Some(30));
    }

    #[test]
    fn test_isolation_level_mapping() {
        assert_eq!(
            isolation_level_sql(IsolationLevel::ReadCommitted),
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED"
        );
        assert_eq!(
            isolation_level_sql(IsolationLevel::RepeatableRead),
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ"
        );
        assert_eq!(
            isolation_level_sql(IsolationLevel::Serializable),
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE"
        );
    }

    #[tokio::test]
    async fn test_database_connect_and_query() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            // Requires Postgres server and DATABASE_URL.
            return;
        };
        let config = DatabaseConfig::new(&url);
        let db = Database::connect(&config).await.expect("Failed to connect");

        let row: (i32,) = sqlx::query_as("SELECT 1")
            .fetch_one(db.pool())
            .await
            .expect("Failed query");
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    async fn test_sqlite_connect_in_memory() {
        let config = DatabaseConfig::new(":memory:");
        let db = Database::connect(&config)
            .await
            .expect("Failed to connect SQLite");
        assert_eq!(db.dialect(), Dialect::Sqlite);
        assert!(db.sqlite_pool().is_some());
    }

    // Restored during review of dispatch 13.1: these three transaction tests were
    // deleted by the SQLite port even though `transaction` and
    // `transaction_with_isolation` both still exist. They cover the exact code the
    // port rewrote, so losing them would have removed coverage precisely where the
    // risk was highest. The rollback test's regression note is retained verbatim.

    #[tokio::test]
    async fn test_transaction_commit() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            // Requires Postgres PgConnection transaction features.
            return;
        };
        let config = DatabaseConfig::new(&url);
        let db = Database::connect(&config).await.expect("Failed to connect");

        let val = db
            .transaction(|conn| {
                Box::pin(async move {
                    sqlx::query("CREATE TEMPORARY TABLE test_tx_commit (id INT)")
                        .execute(&mut *conn)
                        .await
                        .map_err(DbError::QueryFailed)?;

                    sqlx::query("INSERT INTO test_tx_commit VALUES (42)")
                        .execute(&mut *conn)
                        .await
                        .map_err(DbError::QueryFailed)?;

                    let row: (i32,) = sqlx::query_as("SELECT id FROM test_tx_commit")
                        .fetch_one(&mut *conn)
                        .await
                        .map_err(DbError::QueryFailed)?;

                    Ok::<i32, DbError>(row.0)
                })
            })
            .await
            .expect("Transaction failed");

        assert_eq!(val, 42);
    }

    /// Regression note: this test previously used a `TEMPORARY TABLE`, created on a
    /// separately-acquired connection and then written to (and checked) from a
    /// second, distinct pooled connection inside `db.transaction(...)`. Postgres
    /// temp tables are strictly session-scoped, so the transaction's `INSERT` was
    /// actually failing with "relation does not exist" (SQLSTATE 42P01) — not the
    /// intentional `Err` this test meant to trigger — meaning the test passed
    /// regardless of whether `Database::transaction` ever called `.rollback()` at
    /// all. Confirmed empirically by instrumenting the error and inspecting it
    /// directly. Fixed by using a real (non-temporary) table, which — unlike a
    /// temp table — is visible from every connection in the pool, so the INSERT
    /// genuinely lands inside the transaction and the subsequent rollback is what
    /// the final count actually proves.
    #[tokio::test]
    async fn test_transaction_rollback() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            // Requires Postgres PgConnection transaction features.
            return;
        };
        let config = DatabaseConfig::new(&url);
        let db = Database::connect(&config).await.expect("Failed to connect");

        sqlx::query("DROP TABLE IF EXISTS test_tx_rollback")
            .execute(db.pool())
            .await
            .expect("Failed to drop pre-existing table");
        sqlx::query("CREATE TABLE test_tx_rollback (id INT)")
            .execute(db.pool())
            .await
            .expect("Failed to create table");

        let tx_res = db
            .transaction(|conn| {
                Box::pin(async move {
                    sqlx::query("INSERT INTO test_tx_rollback VALUES (100)")
                        .execute(&mut *conn)
                        .await
                        .map_err(DbError::QueryFailed)?;

                    Err::<(), DbError>(DbError::TransactionFailed(
                        "Intentional rollback".to_string(),
                    ))
                })
            })
            .await;

        assert!(tx_res.is_err());
        assert!(
            matches!(&tx_res, Err(DbError::TransactionFailed(msg)) if msg == "Intentional rollback"),
            "expected the intentional error to propagate unchanged, got: {tx_res:?}"
        );

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM test_tx_rollback")
            .fetch_one(db.pool())
            .await
            .expect("Failed to query table");

        assert_eq!(
            row.0, 0,
            "row must not be visible — the transaction should have rolled back"
        );

        sqlx::query("DROP TABLE test_tx_rollback")
            .execute(db.pool())
            .await
            .expect("Failed to clean up table");
    }

    #[tokio::test]
    async fn test_transaction_isolation_levels() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            // Requires Postgres-specific transaction isolation level features (SHOW transaction_isolation).
            return;
        };
        let config = DatabaseConfig::new(&url);
        let db = Database::connect(&config).await.expect("Failed to connect");

        for level in &[
            IsolationLevel::ReadCommitted,
            IsolationLevel::RepeatableRead,
            IsolationLevel::Serializable,
        ] {
            let actual_level = db
                .transaction_with_isolation(*level, |conn| {
                    Box::pin(async move {
                        let row: (String,) = sqlx::query_as("SHOW transaction_isolation")
                            .fetch_one(&mut *conn)
                            .await
                            .map_err(DbError::QueryFailed)?;
                        Ok::<String, DbError>(row.0)
                    })
                })
                .await
                .expect("Transaction with isolation failed");

            let expected = match level {
                IsolationLevel::ReadCommitted => "read committed",
                IsolationLevel::RepeatableRead => "repeatable read",
                IsolationLevel::Serializable => "serializable",
            };
            assert_eq!(actual_level.to_lowercase(), expected);
        }
    }

    #[tokio::test]
    async fn test_transaction_conn_commit_and_rollback_both_backends() {
        // 1. Postgres backend (if DATABASE_URL is set)
        if let Ok(pg_url) = std::env::var("DATABASE_URL") {
            let pg_config = DatabaseConfig::new(&pg_url);
            let pg_db = Database::connect(&pg_config).await.expect("connect pg");

            sqlx::query("DROP TABLE IF EXISTS test_tx_conn_pg")
                .execute(pg_db.pool())
                .await
                .ok();
            sqlx::query("CREATE TABLE test_tx_conn_pg (id INT)")
                .execute(pg_db.pool())
                .await
                .expect("create pg table");

            let res: Result<i32, DbError> = pg_db
                .transaction_conn(|conn| {
                    Box::pin(async move {
                        conn.execute("INSERT INTO test_tx_conn_pg VALUES (10)", &[])
                            .await
                            .map_err(DbError::QueryFailed)?;
                        Ok::<i32, DbError>(10)
                    })
                })
                .await;
            assert_eq!(res.unwrap(), 10);

            let err_res: Result<(), DbError> = pg_db
                .transaction_conn(|conn| {
                    Box::pin(async move {
                        conn.execute("INSERT INTO test_tx_conn_pg VALUES (20)", &[])
                            .await
                            .map_err(DbError::QueryFailed)?;
                        Err::<(), DbError>(DbError::TransactionFailed("abort".to_string()))
                    })
                })
                .await;
            assert!(err_res.is_err());

            let row = pg_db
                .conn()
                .fetch_one("SELECT COUNT(*) FROM test_tx_conn_pg", &[])
                .await
                .unwrap();
            assert_eq!(row.try_i64(0).unwrap().unwrap(), 1);

            sqlx::query("DROP TABLE test_tx_conn_pg")
                .execute(pg_db.pool())
                .await
                .ok();
        }

        // 2. SQLite backend
        let sqlite_config = DatabaseConfig::new(":memory:");
        let sqlite_db = Database::connect(&sqlite_config)
            .await
            .expect("connect sqlite");

        sqlite_db
            .conn()
            .execute("CREATE TABLE test_tx_conn_sqlite (id INT)", &[])
            .await
            .expect("create sqlite table");

        let res: Result<i32, DbError> = sqlite_db
            .transaction_conn(|conn| {
                Box::pin(async move {
                    conn.execute("INSERT INTO test_tx_conn_sqlite VALUES (100)", &[])
                        .await
                        .map_err(DbError::QueryFailed)?;
                    Ok::<i32, DbError>(100)
                })
            })
            .await;
        assert_eq!(res.unwrap(), 100);

        let err_res: Result<(), DbError> = sqlite_db
            .transaction_conn(|conn| {
                Box::pin(async move {
                    conn.execute("INSERT INTO test_tx_conn_sqlite VALUES (200)", &[])
                        .await
                        .map_err(DbError::QueryFailed)?;
                    Err::<(), DbError>(DbError::TransactionFailed("abort".to_string()))
                })
            })
            .await;
        assert!(err_res.is_err());

        let row = sqlite_db
            .conn()
            .fetch_one("SELECT COUNT(*) FROM test_tx_conn_sqlite", &[])
            .await
            .unwrap();
        assert_eq!(row.try_i64(0).unwrap().unwrap(), 1);
    }
}
