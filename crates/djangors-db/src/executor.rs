//! Abstraction over where a query runs: a connection pool, or an open
//! transaction.

use sqlx::{PgConnection, PgPool, SqliteConnection, SqlitePool};

use crate::bind::{BindValue, NullKind};
use crate::database::Database;
use crate::dialect::Dialect;
use crate::row::DbRow;

/// A borrowed handle to whatever a query should run against.
pub enum Conn<'a> {
    /// Run against the PostgreSQL connection pool.
    PgPool(&'a PgPool),
    /// Run against a single PostgreSQL connection in a transaction.
    PgTx(&'a mut PgConnection),
    /// Run against the SQLite connection pool.
    SqlitePool(&'a SqlitePool),
    /// Run against a single SQLite connection in a transaction.
    SqliteTx(&'a mut SqliteConnection),
}

fn build_pg_query<'q>(
    sql: &'q str,
    params: &'q [BindValue],
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    for val in params {
        q = match val {
            BindValue::I64(v) => q.bind(*v),
            BindValue::F64(v) => q.bind(*v),
            BindValue::Text(v) => q.bind(v.as_str()),
            BindValue::Bool(v) => q.bind(*v),
            BindValue::DateTime(v) => q.bind(*v),
            BindValue::Null(kind) => match kind {
                NullKind::I64 => q.bind(None::<i64>),
                NullKind::F64 => q.bind(None::<f64>),
                NullKind::Text => q.bind(None::<String>),
                NullKind::Bool => q.bind(None::<bool>),
                NullKind::DateTime => q.bind(None::<chrono::DateTime<chrono::Utc>>),
            },
        };
    }
    q
}

fn build_sqlite_query<'q>(
    sql: &'q str,
    params: &'q [BindValue],
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments> {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    for val in params {
        q = match val {
            BindValue::I64(v) => q.bind(*v),
            BindValue::F64(v) => q.bind(*v),
            BindValue::Text(v) => q.bind(v.as_str()),
            BindValue::Bool(v) => q.bind(*v),
            BindValue::DateTime(v) => q.bind(*v),
            BindValue::Null(kind) => match kind {
                NullKind::I64 => q.bind(None::<i64>),
                NullKind::F64 => q.bind(None::<f64>),
                NullKind::Text => q.bind(None::<String>),
                NullKind::Bool => q.bind(None::<bool>),
                NullKind::DateTime => q.bind(None::<chrono::DateTime<chrono::Utc>>),
            },
        };
    }
    q
}

impl<'a> Conn<'a> {
    /// Execute the query and collect every row.
    pub async fn fetch_all(
        &mut self,
        sql: &str,
        params: &[BindValue],
    ) -> Result<Vec<DbRow>, sqlx::Error> {
        match self {
            Conn::PgPool(pool) => {
                let q = build_pg_query(sql, params);
                let rows = q.fetch_all(*pool).await?;
                Ok(rows.into_iter().map(DbRow::Pg).collect())
            }
            Conn::PgTx(tx) => {
                let q = build_pg_query(sql, params);
                let rows = q.fetch_all(&mut **tx).await?;
                Ok(rows.into_iter().map(DbRow::Pg).collect())
            }
            Conn::SqlitePool(pool) => {
                let q = build_sqlite_query(sql, params);
                let rows = q.fetch_all(*pool).await?;
                Ok(rows.into_iter().map(DbRow::Sqlite).collect())
            }
            Conn::SqliteTx(tx) => {
                let q = build_sqlite_query(sql, params);
                let rows = q.fetch_all(&mut **tx).await?;
                Ok(rows.into_iter().map(DbRow::Sqlite).collect())
            }
        }
    }

    /// Execute the query, requiring exactly one row.
    pub async fn fetch_one(
        &mut self,
        sql: &str,
        params: &[BindValue],
    ) -> Result<DbRow, sqlx::Error> {
        match self {
            Conn::PgPool(pool) => {
                let q = build_pg_query(sql, params);
                let row = q.fetch_one(*pool).await?;
                Ok(DbRow::Pg(row))
            }
            Conn::PgTx(tx) => {
                let q = build_pg_query(sql, params);
                let row = q.fetch_one(&mut **tx).await?;
                Ok(DbRow::Pg(row))
            }
            Conn::SqlitePool(pool) => {
                let q = build_sqlite_query(sql, params);
                let row = q.fetch_one(*pool).await?;
                Ok(DbRow::Sqlite(row))
            }
            Conn::SqliteTx(tx) => {
                let q = build_sqlite_query(sql, params);
                let row = q.fetch_one(&mut **tx).await?;
                Ok(DbRow::Sqlite(row))
            }
        }
    }

    /// Execute the query, returning the row if there is one.
    pub async fn fetch_optional(
        &mut self,
        sql: &str,
        params: &[BindValue],
    ) -> Result<Option<DbRow>, sqlx::Error> {
        match self {
            Conn::PgPool(pool) => {
                let q = build_pg_query(sql, params);
                let row = q.fetch_optional(*pool).await?;
                Ok(row.map(DbRow::Pg))
            }
            Conn::PgTx(tx) => {
                let q = build_pg_query(sql, params);
                let row = q.fetch_optional(&mut **tx).await?;
                Ok(row.map(DbRow::Pg))
            }
            Conn::SqlitePool(pool) => {
                let q = build_sqlite_query(sql, params);
                let row = q.fetch_optional(*pool).await?;
                Ok(row.map(DbRow::Sqlite))
            }
            Conn::SqliteTx(tx) => {
                let q = build_sqlite_query(sql, params);
                let row = q.fetch_optional(&mut **tx).await?;
                Ok(row.map(DbRow::Sqlite))
            }
        }
    }

    /// Execute the query for its side effect, returning the affected-row count.
    pub async fn execute(&mut self, sql: &str, params: &[BindValue]) -> Result<u64, sqlx::Error> {
        match self {
            Conn::PgPool(pool) => {
                let q = build_pg_query(sql, params);
                let res = q.execute(*pool).await?;
                Ok(res.rows_affected())
            }
            Conn::PgTx(tx) => {
                let q = build_pg_query(sql, params);
                let res = q.execute(&mut **tx).await?;
                Ok(res.rows_affected())
            }
            Conn::SqlitePool(pool) => {
                let q = build_sqlite_query(sql, params);
                let res = q.execute(*pool).await?;
                Ok(res.rows_affected())
            }
            Conn::SqliteTx(tx) => {
                let q = build_sqlite_query(sql, params);
                let res = q.execute(&mut **tx).await?;
                Ok(res.rows_affected())
            }
        }
    }

    /// The database dialect for this connection handle.
    pub fn dialect(&self) -> Dialect {
        match self {
            Conn::PgPool(_) | Conn::PgTx(_) => Dialect::Postgres,
            Conn::SqlitePool(_) | Conn::SqliteTx(_) => Dialect::Sqlite,
        }
    }

    /// Whether this handle is inside an open transaction.
    pub fn in_transaction(&self) -> bool {
        matches!(self, Conn::PgTx(_) | Conn::SqliteTx(_))
    }
}

/// A target the ORM can run queries against.
pub trait DbExecutor {
    /// Borrow the underlying execution target.
    fn conn(&mut self) -> Conn<'_>;

    /// Returns the database dialect.
    fn dialect(&mut self) -> Dialect {
        self.conn().dialect()
    }

    /// Record one query for test observability.
    fn record_query(&self) {}
}

impl DbExecutor for &Database {
    fn conn(&mut self) -> Conn<'_> {
        Database::conn(self)
    }

    fn record_query(&self) {
        Database::record_query(self);
    }
}

impl DbExecutor for &mut PgConnection {
    fn conn(&mut self) -> Conn<'_> {
        Conn::PgTx(self)
    }
}

impl DbExecutor for &mut SqliteConnection {
    fn conn(&mut self) -> Conn<'_> {
        Conn::SqliteTx(self)
    }
}

impl DbExecutor for Conn<'_> {
    fn conn(&mut self) -> Conn<'_> {
        match self {
            Conn::PgPool(pool) => Conn::PgPool(pool),
            Conn::PgTx(tx) => Conn::PgTx(tx),
            Conn::SqlitePool(pool) => Conn::SqlitePool(pool),
            Conn::SqliteTx(tx) => Conn::SqliteTx(tx),
        }
    }
}
