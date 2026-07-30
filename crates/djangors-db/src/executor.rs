//! Abstraction over *where* a query runs: a connection pool, or an open
//! transaction.
//!
//! Before this existed, every ORM method took `&Database` and reached for
//! `Database::pool()`, which meant ORM calls could never participate in a
//! transaction — atomic work had to drop down to raw SQLx and lose the
//! `QuerySet` API entirely. [`DbExecutor`] closes that gap: the ORM is generic
//! over its execution target, so the same call works against a pool or inside
//! [`Database::transaction`](crate::Database::transaction).
//!
//! Against the pool, exactly as before:
//!
//! ```ignore
//! Account::objects().all(db).await?;
//! ```
//!
//! Or inside a transaction, where `conn` is the `&mut PgConnection` the closure
//! receives. Both writes commit, or neither does:
//!
//! ```ignore
//! db.transaction(|conn| {
//!     Box::pin(async move {
//!         Account::objects()
//!             .filter(q!(id = from_id))?
//!             .update(&mut *conn, vec![("balance", SetExpr::from(new_from))])
//!             .await?;
//!         Account::objects()
//!             .filter(q!(id = to_id))?
//!             .update(&mut *conn, vec![("balance", SetExpr::from(new_to))])
//!             .await?;
//!         Ok::<_, OrmError>(())
//!     })
//! })
//! .await?;
//! ```

use sqlx::postgres::{PgArguments, PgQueryResult, PgRow};
use sqlx::query::Query;
use sqlx::{IntoArguments, PgConnection, PgPool, Postgres};

use crate::database::Database;

/// A borrowed handle to whatever a query should run against.
///
/// Obtained from [`DbExecutor::conn`]. This exists so the pool-versus-transaction
/// branch is written once here rather than at every call site in the ORM.
pub enum Conn<'a> {
    /// Run against the shared connection pool; each query gets its own
    /// connection and commits independently.
    Pool(&'a PgPool),
    /// Run against a single connection with an open transaction; queries share
    /// the transaction and commit or roll back together.
    Tx(&'a mut PgConnection),
}

impl<'a> Conn<'a> {
    /// Execute the query and collect every row.
    pub async fn fetch_all<'q, A>(
        &mut self,
        query: Query<'q, Postgres, A>,
    ) -> Result<Vec<PgRow>, sqlx::Error>
    where
        A: 'q + IntoArguments<Postgres>,
    {
        match self {
            Conn::Pool(pool) => query.fetch_all(*pool).await,
            Conn::Tx(conn) => query.fetch_all(&mut **conn).await,
        }
    }

    /// Execute the query, requiring exactly one row.
    pub async fn fetch_one<'q, A>(
        &mut self,
        query: Query<'q, Postgres, A>,
    ) -> Result<PgRow, sqlx::Error>
    where
        A: 'q + IntoArguments<Postgres>,
    {
        match self {
            Conn::Pool(pool) => query.fetch_one(*pool).await,
            Conn::Tx(conn) => query.fetch_one(&mut **conn).await,
        }
    }

    /// Execute the query, returning the row if there is one.
    pub async fn fetch_optional<'q, A>(
        &mut self,
        query: Query<'q, Postgres, A>,
    ) -> Result<Option<PgRow>, sqlx::Error>
    where
        A: 'q + IntoArguments<Postgres>,
    {
        match self {
            Conn::Pool(pool) => query.fetch_optional(*pool).await,
            Conn::Tx(conn) => query.fetch_optional(&mut **conn).await,
        }
    }

    /// Execute the query for its side effect, returning the affected-row count.
    pub async fn execute<'q, A>(
        &mut self,
        query: Query<'q, Postgres, A>,
    ) -> Result<PgQueryResult, sqlx::Error>
    where
        A: 'q + IntoArguments<Postgres>,
    {
        match self {
            Conn::Pool(pool) => query.execute(*pool).await,
            Conn::Tx(conn) => query.execute(&mut **conn).await,
        }
    }

    /// Whether this handle is inside an open transaction.
    pub fn in_transaction(&self) -> bool {
        matches!(self, Conn::Tx(_))
    }
}

/// A target the ORM can run queries against.
///
/// Implemented for `&Database` (the connection pool) and for
/// `&mut PgConnection` (an open transaction, as handed to the
/// [`Database::transaction`] closure). ORM methods are generic over this trait,
/// so the same `QuerySet` call works in both contexts.
pub trait DbExecutor {
    /// Borrow the underlying execution target.
    fn conn(&mut self) -> Conn<'_>;

    /// Record one query for test observability.
    ///
    /// Only the pool-backed [`Database`] keeps a counter; running inside a
    /// transaction is a no-op.
    fn record_query(&self) {}
}

impl DbExecutor for &Database {
    fn conn(&mut self) -> Conn<'_> {
        Conn::Pool(self.pool())
    }

    fn record_query(&self) {
        Database::record_query(self);
    }
}

impl DbExecutor for &mut PgConnection {
    fn conn(&mut self) -> Conn<'_> {
        Conn::Tx(self)
    }
}

impl DbExecutor for Conn<'_> {
    fn conn(&mut self) -> Conn<'_> {
        match self {
            Conn::Pool(pool) => Conn::Pool(pool),
            Conn::Tx(conn) => Conn::Tx(conn),
        }
    }
}

/// Convenience alias for the concrete query type the ORM builds.
pub type PgQuery<'q> = Query<'q, Postgres, PgArguments>;
