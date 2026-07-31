# Databases and Backends

Djangors supports both PostgreSQL and SQLite backends. This dual-backend architecture allows you to run your application against a lightweight SQLite database for local development, testing, and single-writer deployments, while deploying to a highly concurrent PostgreSQL database in production.

---

## Selecting the Backend

The active database backend is determined dynamically from the `DATABASE_URL` environment variable at runtime. Djangors parses this URL to determine the appropriate `Dialect` without making an active connection.

In code, this is handled by `Dialect::from_url`. The configuration parses the URL according to the following rules:

* **SQLite backend**: Selected if the URL starts with `sqlite://`, ends with `.db` or `.sqlite`, or is exactly `:memory:` or `sqlite::memory:`.
* **PostgreSQL backend**: Selected for any other URL format (e.g. `postgres://` or `postgresql://`).

The connection string comes from the `DATABASE_URL` environment variable. Note that it is *not* part
of `djangors.toml` — [`DjangorsSettings`](settings.md) covers `debug`, `allowed_hosts`,
`secret_key`, `host`, and `port` only, and the database URL is read from the environment directly:

```bash
# Dev environment using SQLite
export DATABASE_URL="sqlite://db.sqlite3"

# An in-memory database, which is what the SQLite tests use
export DATABASE_URL="sqlite::memory:"

# Production using PostgreSQL
export DATABASE_URL="postgres://postgres:password@localhost:5432/my_app"
```

Pool sizing (`max_connections`, timeouts) is configured in code through
`djangors_db::config::DatabaseConfig`, not through a file:

```rust,illustrative
let config = djangors_db::config::DatabaseConfig::new(std::env::var("DATABASE_URL")?);
let db = djangors_db::Database::connect(&config).await?;
```

---

## Dialect-Aware Architecture

Rather than forcing a single SQL syntax across different databases, Djangors uses a dialect-aware architecture. The system checks the active `Dialect` to generate and execute database-specific SQL.

Dialect awareness spans across:
1. **ORM Query Path**: The generated SQL for filtering, limits, offsets, and operators is tailored to the active database dialect.
2. **Raw SQL Execution**: All production raw SQL built using the database executor uses the correct syntax and placeholder formats.
3. **Migrations**: When generating migration files via `dj makemigrations` and executing them via `dj migrate`, Djangors generates SQL operations tailored to Postgres or SQLite.

### The `Dialect` API

The `Dialect` enum resides in `djangors-db` and exposes the differences in SQL syntax:

```rust,illustrative
pub enum Dialect {
    Postgres,
    Sqlite,
}
```

It provides the following helper methods to bridge dialect differences:

* **`placeholder(index)`**: Returns `"$1"`, `"$2"`, etc., for PostgreSQL, and `"?"` for SQLite.
* **`ilike()`**: Returns `"ILIKE"` for PostgreSQL. For SQLite, it returns `"LIKE"` because SQLite's standard `LIKE` operator is already ASCII-case-insensitive.
* **`quote_ident(name)`**: Wraps the identifier in double quotes (e.g. `"name"`). Supported by both Postgres and SQLite.
* **`cast_float(expr)`**: Casts an expression to a float type: `expr::float8` on PostgreSQL, and `CAST(expr AS REAL)` on SQLite.
* **`extract_date_part(part, col)`**: Extracts parts of a date (`Year`, `Month`, `Day`). Generates `EXTRACT(part FROM col)::int` for Postgres and `CAST(strftime(format, col) AS INTEGER)` for SQLite.
* **`bytea_type()`**: Returns `"BYTEA"` for PostgreSQL, and `"BLOB"` for SQLite.
* **`auto_pk_type()`**: Returns `"SERIAL PRIMARY KEY"` for PostgreSQL, and `"INTEGER PRIMARY KEY AUTOINCREMENT"` for SQLite.
* **`timestamp_type()`**: Returns `"TIMESTAMPTZ"` for PostgreSQL, and `"TEXT"` for SQLite.
* **`current_timestamp()`**: Returns `"now()"` for PostgreSQL, and `"CURRENT_TIMESTAMP"` for SQLite.
* **`from_url(url)`**: Static helper that infers the dialect from a URL string.

---

## Honest Limitations

While the dual-backend architecture simplifies local development, there are key differences and limitations to keep in mind:

### 1. Testing is PostgreSQL-Only
The testing framework in `djangors-test` is strictly PostgreSQL-only. This is because its transaction isolation and test database management system relies on PostgreSQL session-level advisory locks to run tests concurrently without interference. SQLite cannot be used to run the Djangors test suite.

### 2. Unsupported Migrations on SQLite
SQLite has very limited support for altering existing table schemas. Because of this, certain schema operations are not supported on SQLite. For instance:
* `Operation::AlterColumnType` returns a `MigrationError::UnsupportedOnDialect` rather than emitting broken or corrupt SQL.

### 3. SQLite Concurrency (Single Writer)
SQLite permits a single writer at any given time. If your application has highly concurrent write traffic, SQLite will serialize these writes and may return database locked errors (`database is locked`) if writes timeout. Postgres handles highly concurrent writes out of the box using fine-grained row locks.

### 4. Database Handles and the Connection Pool
When interacting with the connection pool:
* Calling `Database::pool()` returns a reference to the underlying `PgPool`. **This will panic** if the database handle is connected to SQLite.
* To write code that runs on both backends, use `Database::conn()` instead. It returns a `Conn` enum wrapper that can execute queries on both PostgreSQL and SQLite pools.

---

## When to Choose Which

* **SQLite** is ideal for:
  * Local development to avoid running a local PostgreSQL container.
  * Simple staging environments or internal tools with minimal write concurrency.
  * Single-writer, read-heavy workloads.
* **PostgreSQL** is required for:
  * Production deployments with concurrent writes or high traffic.
  * Running the automated test suite (`djangors-test`).
  * Advanced operations such as alter column type migrations.

---

## Row Locking and Savepoints

For banking and financial applications requiring atomic transactions, Djangors provides explicit row locking (`select_for_update`) and savepoints (`savepoint`, `rollback_to_savepoint`, `release_savepoint`).

### Row Locking with `select_for_update`

`QuerySet::select_for_update` locks matching rows for the duration of a transaction:

```rust,illustrative
Account::objects()
    .filter(q!(id = sender_id))?
    .select_for_update()
    .get(&mut *conn)
    .await?;
```

Modifiers allow controlling wait and table locking behavior:

* **`.nowait()`**: Returns an error immediately if a row lock cannot be acquired instead of blocking.
* **`.skip_locked()`**: Skips rows locked by concurrent transactions.
* **`.lock_of(&["account"])`**: Limits locking to specific tables in Postgres (`FOR UPDATE OF`).

> [!NOTE]
> `select_for_update()` requires an active transaction; calling it outside a transaction returns `OrmError::SelectForUpdateOutsideTransaction`. On SQLite, `select_for_update()` degrades gracefully to a no-op because SQLite serializes writes at the database level, but modifiers (`nowait`, `skip_locked`, `lock_of`) return `OrmError::UnsupportedOnDialect`.

### Savepoints

Savepoints allow partial rollback within a transaction handle (`Conn`):

```rust,illustrative
conn.savepoint("transfer_checkpoint").await?;
// Perform risky operation
if let Err(_) = process_fee(&mut conn).await {
    conn.rollback_to_savepoint("transfer_checkpoint").await?;
} else {
    conn.release_savepoint("transfer_checkpoint").await?;
}
```

### Double-Entry Ledger Example

Below is a banking transfer example that debits one account and credits another under row locking with savepoint error recovery:

```rust,illustrative
pub async fn transfer_funds(
    db: &Database,
    sender_id: i64,
    receiver_id: i64,
    amount: i64,
) -> Result<(), DbError> {
    db.transaction_conn(|conn| {
        Box::pin(async move {
            // Lock sender account
            let mut sender = Account::objects()
                .filter(q!(id = sender_id))
                .map_err(OrmError::from)?
                .select_for_update()
                .get(&mut **conn)
                .await?;

            if sender.balance < amount {
                return Err(DbError::TransactionFailed("Insufficient funds".into()));
            }

            // Lock receiver account
            let mut receiver = Account::objects()
                .filter(q!(id = receiver_id))
                .map_err(OrmError::from)?
                .select_for_update()
                .get(&mut **conn)
                .await?;

            // Create a savepoint before attempting ledger operations
            conn.savepoint("ledger_entry").await?;

            sender.balance -= amount;
            receiver.balance += amount;

            // Execute debit & credit updates
            sender.save(&mut **conn).await?;
            receiver.save(&mut **conn).await?;

            // Commit savepoint checkpoint
            conn.release_savepoint("ledger_entry").await?;

            Ok(())
        })
    })
    .await
}
```
