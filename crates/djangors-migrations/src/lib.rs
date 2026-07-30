#![deny(missing_docs)]
//! Schema migration planning, execution, and SQL DDL generation for Djangors.
//!
//! # Database Connection Architecture: Enum Dispatch vs Generics
//! In Djangors, the database connection (`Conn`) and query row (`DbRow`) abstractions are designed using
//! runtime enum dispatch rather than trait generics (e.g., being generic over `<DB: sqlx::Database>`).
//! If generics were used, bounds like `DB: Database, for<'r> i64: Decode<'r, DB> + Type<DB>` would have to
//! propagate through every struct, trait method, derive macro, and downstream application crate.
//! Instead, `Conn` and `DbRow` wrap driver-specific types internally, resolving dialect differences (Postgres vs SQLite)
//! via runtime `match` statements in a single unified crate.
//!
//! Submodules:
//! - [`error`]: Migration error types.
//! - [`operation`]: DDL operation definitions and SQL generation logic.
//! - [`plan`]: Topological sorting and migration plan building.
//! - [`type_mapping`]: Mapping of ORM field types to SQL column types.
/// Migration error types.
pub mod error;
/// DDL operation definitions and SQL generation logic.
pub mod operation;
/// Topological sorting and migration plan building.
pub mod plan;
/// Mapping of ORM field types to SQL column types.
pub mod type_mapping;

pub use error::MigrationError;
pub use operation::{ColumnDef, ForeignKeyRef, Operation};
pub use plan::build_create_all_plan;
pub use plan::build_create_plan_from_snapshots;

use djangors_db::BindValue;
use std::path::{Path, PathBuf};

/// Apply migration files in filename order. Files use `-- up` and `-- down` markers;
/// `-- no-down` explicitly records an unavailable reverse migration.
pub async fn migrate_from_dir(
    db: &djangors_db::Database,
    dir: &Path,
) -> Result<(), MigrationError> {
    ensure_history(db).await?;
    let dialect = db.dialect();
    let mut files = migration_files(dir)?;
    files.sort();
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let query = format!(
            "SELECT 1 FROM djangors_migrations WHERE name = {}",
            dialect.placeholder(1)
        );
        let params = [BindValue::Text(name.clone())];
        let applied = db.conn().fetch_optional(&query, &params).await?.is_some();
        if applied {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        let up = section(&content, "up");
        db.transaction_conn(|conn| {
            Box::pin(async move {
                for sql in up.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                    let stmt = format!("{};", sql);
                    conn.execute(&stmt, &[]).await?;
                }
                let dialect = conn.dialect();
                let insert_sql = format!(
                    "INSERT INTO djangors_migrations (name) VALUES ({})",
                    dialect.placeholder(1)
                );
                conn.execute(&insert_sql, &[BindValue::Text(name)]).await?;
                Ok::<(), djangors_db::DbError>(())
            })
        })
        .await
        .map_err(MigrationError::Database)?;
    }
    Ok(())
}

/// Roll back the most recent `count` applied migration files.
pub async fn rollback_from_dir(
    db: &djangors_db::Database,
    dir: &Path,
    count: u32,
) -> Result<(), MigrationError> {
    ensure_history(db).await?;
    // Scope strictly to migrations whose file still exists in `dir` - `djangors_migrations` is
    // a single shared tracking table, and without this filter "most recently applied" would be
    // read globally across any other migration set that happens to share the same table
    // (this matters for this crate's own concurrent test suite, and defensively protects any
    // real deployment where more than one migrations directory might share a database).
    let known_names: Vec<String> = migration_files(dir)?
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();

    if known_names.is_empty() {
        return Ok(());
    }

    let dialect = db.dialect();
    let placeholders: Vec<String> = (1..=known_names.len())
        .map(|i| dialect.placeholder(i))
        .collect();
    let limit_placeholder = dialect.placeholder(known_names.len() + 1);
    let sql = format!(
        "SELECT name FROM djangors_migrations WHERE name IN ({}) ORDER BY id DESC LIMIT {}",
        placeholders.join(", "),
        limit_placeholder
    );

    let mut params: Vec<BindValue> = known_names
        .iter()
        .map(|n| BindValue::Text(n.clone()))
        .collect();
    params.push(BindValue::I64(count as i64));

    let rows = db.conn().fetch_all(&sql, &params).await?;
    let mut names = Vec::new();
    for row in rows {
        if let Some(n) = row.try_string(0)? {
            names.push(n);
        }
    }

    let mut downs = Vec::new();
    for name in &names {
        let path = dir.join(format!("{name}.sql"));
        let text = std::fs::read_to_string(&path)
            .map_err(|_| MigrationError::NonInvertible { name: name.clone() })?;
        let down = section(&text, "down");
        if down.trim().is_empty() || text.contains("-- no-down") {
            return Err(MigrationError::NonInvertible { name: name.clone() });
        }
        downs.push((name.clone(), down));
    }
    for (name, down) in downs {
        db.transaction_conn(|conn| {
            Box::pin(async move {
                for sql in down.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                    let stmt = format!("{};", sql);
                    conn.execute(&stmt, &[]).await?;
                }
                let dialect = conn.dialect();
                let delete_sql = format!(
                    "DELETE FROM djangors_migrations WHERE name = {}",
                    dialect.placeholder(1)
                );
                conn.execute(&delete_sql, &[BindValue::Text(name)]).await?;
                Ok::<(), djangors_db::DbError>(())
            })
        })
        .await
        .map_err(MigrationError::Database)?;
    }
    Ok(())
}

async fn ensure_history(db: &djangors_db::Database) -> Result<(), MigrationError> {
    let dialect = db.dialect();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS djangors_migrations (\
            id {}, \
            name TEXT UNIQUE NOT NULL, \
            applied_at {} NOT NULL DEFAULT {}\
        )",
        dialect.auto_pk_type(),
        dialect.timestamp_type(),
        dialect.current_timestamp()
    );
    db.conn().execute(&sql, &[]).await?;
    Ok(())
}

fn migration_files(dir: &Path) -> Result<Vec<PathBuf>, MigrationError> {
    Ok(std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "sql")
                && p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .get(..4)
                    .and_then(|x| x.parse::<u32>().ok())
                    .is_some()
        })
        .collect())
}

fn section(text: &str, wanted: &str) -> String {
    let mut current = "";
    let mut start = 0;
    for (i, line) in text.lines().enumerate() {
        if line.trim() == format!("-- {wanted}") {
            current = wanted;
            start = text.lines().take(i + 1).map(|l| l.len() + 1).sum();
        } else if (line.trim() == "-- down" || line.trim() == "-- up") && current == wanted {
            return text[start..text.lines().take(i).map(|l| l.len() + 1).sum::<usize>()]
                .to_string();
        }
    }
    if current == wanted {
        text[start..].to_string()
    } else {
        String::new()
    }
}

/// Applies initial database schema migrations if not already applied.
pub async fn migrate(db: &djangors_db::Database) -> Result<(), MigrationError> {
    if Path::new("migrations").is_dir() {
        return migrate_from_dir(db, Path::new("migrations")).await;
    }
    // 1. Ensure tracking table exists
    ensure_history(db).await?;

    let dialect = db.dialect();

    // 2. Check if 0001_initial is applied
    let query = format!(
        "SELECT 1 FROM djangors_migrations WHERE name = {}",
        dialect.placeholder(1)
    );
    let params = [BindValue::Text("0001_initial".to_string())];
    let row = db.conn().fetch_optional(&query, &params).await?;

    if row.is_some() {
        return Ok(());
    }

    // 3. Build plan and execute in transaction
    let plan = build_create_all_plan(dialect)?;
    let mut sqls = Vec::new();
    for op in &plan {
        sqls.push(op.to_sql(dialect)?);
    }

    db.transaction_conn(|conn| {
        let sqls = sqls.clone();
        Box::pin(async move {
            for sql in sqls {
                conn.execute(&sql, &[]).await?;
            }
            let dialect = conn.dialect();
            let insert_sql = format!(
                "INSERT INTO djangors_migrations (name) VALUES ({})",
                dialect.placeholder(1)
            );
            conn.execute(&insert_sql, &[BindValue::Text("0001_initial".to_string())])
                .await?;
            Ok::<(), djangors_db::DbError>(())
        })
    })
    .await
    .map_err(MigrationError::Database)?;

    Ok(())
}

#[cfg(test)]
mod migrate_from_dir_tests {
    use super::*;

    const TEST_DB_URL: &str = "postgres://postgres:postgres@localhost/djangors_test";

    // Each test uses its own uniquely-named table, but all of them share the single
    // `djangors_migrations` bookkeeping table via `ensure_history()`'s
    // `CREATE TABLE IF NOT EXISTS`. That statement is not atomic under real concurrency -
    // two tests creating it at the same instant can hit Postgres's own catalog-level
    // uniqueness constraints (pg_class_relname_nsp_index / pg_type_typname_nsp_index),
    // the same class of cross-test DDL race documented in PLAN.md's Phase 11 (task #61).
    static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Every test gets its own migrations directory and its own uniquely-named table, so
    /// tests can run concurrently against the one real shared `djangors_test` database
    /// without colliding (djangors-test's `TestDatabase` does not yet provide per-test
    /// isolation - see PLAN.md Phase 11, "TestDatabase transactional rollback-per-test").
    /// The shared `djangors_migrations` tracking table is safe to share too, since every
    /// migration name embeds the same unique suffix.
    fn unique_suffix() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{nanos}_{:?}", std::thread::current().id())
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect()
    }

    async fn connect() -> djangors_db::Database {
        let config = djangors_db::config::DatabaseConfig::new(TEST_DB_URL);
        djangors_db::Database::connect(&config)
            .await
            .expect("failed to connect to djangors_test - is Postgres running?")
    }

    fn write_migration(dir: &std::path::Path, n: u32, name: &str, up: &str, down: Option<&str>) {
        let body = match down {
            Some(down) => format!("-- up\n{up}\n-- down\n{down}\n"),
            None => format!("-- up\n{up}\n-- down\n-- no-down\n"),
        };
        std::fs::write(dir.join(format!("{n:04}_{name}.sql")), body).unwrap();
    }

    /// Regression test for the bug this design doc (11.1) fixes: `dj migrate` used to only
    /// ever check a single hardcoded `'0001_initial'` flag and never actually read/apply any
    /// `migrations/NNNN_*.sql` file from disk, so an `AddColumn` migration generated by
    /// `makemigrations` was never really executed. This must fail on the old `migrate()`/
    /// `migrate_with_plan()` path and pass via `migrate_from_dir`.
    #[tokio::test]
    async fn multi_migration_sequence_add_column_actually_applies() {
        let db = connect().await;
        let _guard = TEST_DB_LOCK.lock().await;
        let suffix = unique_suffix();
        let table = format!("mig_test_{suffix}");
        let dir = std::env::temp_dir().join(format!("djangors_migtest_{suffix}"));
        std::fs::create_dir_all(&dir).unwrap();

        write_migration(
            &dir,
            1,
            &format!("create_{suffix}"),
            &format!("CREATE TABLE {table} (id SERIAL PRIMARY KEY)"),
            Some(&format!("DROP TABLE {table}")),
        );
        write_migration(
            &dir,
            2,
            &format!("addcol_{suffix}"),
            &format!("ALTER TABLE {table} ADD COLUMN label TEXT"),
            Some(&format!("ALTER TABLE {table} DROP COLUMN label")),
        );

        migrate_from_dir(&db, &dir)
            .await
            .expect("migrate_from_dir should apply both migrations");

        // The column must genuinely exist now - this is exactly what the old code never did.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} (label) VALUES ('hello')"
        )))
        .execute(db.pool())
        .await
        .expect("label column should exist and accept a value");

        // Cleanup.
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
            .execute(db.pool())
            .await
            .ok();
        sqlx::query("DELETE FROM djangors_migrations WHERE name LIKE $1")
            .bind(format!("%{suffix}%"))
            .execute(db.pool())
            .await
            .ok();
    }

    #[tokio::test]
    async fn rollback_reverses_a_real_schema_change() {
        let db = connect().await;
        let _guard = TEST_DB_LOCK.lock().await;
        let suffix = unique_suffix();
        let table = format!("mig_rb_{suffix}");
        let dir = std::env::temp_dir().join(format!("djangors_migtest_rb_{suffix}"));
        std::fs::create_dir_all(&dir).unwrap();

        write_migration(
            &dir,
            1,
            &format!("create_{suffix}"),
            &format!("CREATE TABLE {table} (id SERIAL PRIMARY KEY)"),
            Some(&format!("DROP TABLE {table}")),
        );
        write_migration(
            &dir,
            2,
            &format!("addcol_{suffix}"),
            &format!("ALTER TABLE {table} ADD COLUMN label TEXT"),
            Some(&format!("ALTER TABLE {table} DROP COLUMN label")),
        );
        migrate_from_dir(&db, &dir).await.unwrap();

        rollback_from_dir(&db, &dir, 1)
            .await
            .expect("rolling back the last migration should succeed");

        // The column must genuinely be gone now.
        let err = sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} (label) VALUES ('nope')"
        )))
        .execute(db.pool())
        .await
        .unwrap_err();
        assert!(
            format!("{err}").to_lowercase().contains("label")
                || format!("{err}").to_lowercase().contains("column"),
            "expected a missing-column error after rollback, got: {err}"
        );

        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM djangors_migrations WHERE name LIKE $1")
                .bind(format!("%{suffix}%"))
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(
            remaining, 1,
            "only the create-table migration should remain recorded as applied"
        );

        sqlx::query(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
            .execute(db.pool())
            .await
            .ok();
        sqlx::query("DELETE FROM djangors_migrations WHERE name LIKE $1")
            .bind(format!("%{suffix}%"))
            .execute(db.pool())
            .await
            .ok();
    }

    #[tokio::test]
    async fn rollback_refuses_a_non_invertible_migration_without_partial_effect() {
        let db = connect().await;
        let _guard = TEST_DB_LOCK.lock().await;
        let suffix = unique_suffix();
        let table = format!("mig_ni_{suffix}");
        let dir = std::env::temp_dir().join(format!("djangors_migtest_ni_{suffix}"));
        std::fs::create_dir_all(&dir).unwrap();

        write_migration(
            &dir,
            1,
            &format!("create_{suffix}"),
            &format!("CREATE TABLE {table} (id SERIAL PRIMARY KEY, doomed TEXT)"),
            Some(&format!("DROP TABLE {table}")),
        );
        // A DropColumn migration has no safe down - written with no down section at all.
        write_migration(
            &dir,
            2,
            &format!("dropcol_{suffix}"),
            &format!("ALTER TABLE {table} DROP COLUMN doomed"),
            None,
        );
        migrate_from_dir(&db, &dir).await.unwrap();

        let result = rollback_from_dir(&db, &dir, 1).await;
        assert!(
            matches!(result, Err(MigrationError::NonInvertible { .. })),
            "expected NonInvertible, got {result:?}"
        );

        // Both migrations must still be recorded as applied - the refusal must not have
        // partially rolled anything back.
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM djangors_migrations WHERE name LIKE $1")
                .bind(format!("%{suffix}%"))
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(remaining, 2, "no migration should have been rolled back");

        sqlx::query(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
            .execute(db.pool())
            .await
            .ok();
        sqlx::query("DELETE FROM djangors_migrations WHERE name LIKE $1")
            .bind(format!("%{suffix}%"))
            .execute(db.pool())
            .await
            .ok();
    }

    #[tokio::test]
    async fn migrate_from_dir_is_idempotent() {
        let db = connect().await;
        let _guard = TEST_DB_LOCK.lock().await;
        let suffix = unique_suffix();
        let table = format!("mig_idem_{suffix}");
        let dir = std::env::temp_dir().join(format!("djangors_migtest_idem_{suffix}"));
        std::fs::create_dir_all(&dir).unwrap();

        write_migration(
            &dir,
            1,
            &format!("create_{suffix}"),
            &format!("CREATE TABLE {table} (id SERIAL PRIMARY KEY)"),
            Some(&format!("DROP TABLE {table}")),
        );

        migrate_from_dir(&db, &dir).await.unwrap();
        // Running it again must be a clean no-op, not a "relation already exists" error.
        migrate_from_dir(&db, &dir)
            .await
            .expect("re-running migrate_from_dir must be idempotent");

        let applied: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM djangors_migrations WHERE name LIKE $1")
                .bind(format!("%{suffix}%"))
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(applied, 1, "the migration must be recorded exactly once");

        sqlx::query(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
            .execute(db.pool())
            .await
            .ok();
        sqlx::query("DELETE FROM djangors_migrations WHERE name LIKE $1")
            .bind(format!("%{suffix}%"))
            .execute(db.pool())
            .await
            .ok();
    }

    #[tokio::test]
    async fn test_sqlite_plan_execution_from_snapshots() {
        use djangors_db::{Database, DatabaseConfig, Dialect};
        use djangors_orm::{FieldKind, FieldSnapshot, ModelSnapshot, SnapshotDefault};

        let snapshot = ModelSnapshot {
            app_label: "testapp".to_string(),
            table_name: "testapp_item".to_string(),
            struct_name: "Item".to_string(),
            fields: vec![
                FieldSnapshot {
                    name: "id".to_string(),
                    column_name: "id".to_string(),
                    kind: FieldKind::BigInt,
                    nullable: false,
                    primary_key: true,
                    unique: false,
                    db_index: false,
                    default: SnapshotDefault::None,
                    max_length: None,
                    auto: true,
                },
                FieldSnapshot {
                    name: "title".to_string(),
                    column_name: "title".to_string(),
                    kind: FieldKind::Char,
                    nullable: false,
                    primary_key: false,
                    unique: false,
                    db_index: false,
                    default: SnapshotDefault::None,
                    max_length: Some(100),
                    auto: false,
                },
            ],
            relations: vec![],
            unique_together: vec![],
            ordering: vec![],
        };

        let plan = build_create_plan_from_snapshots(&[snapshot], Dialect::Sqlite).unwrap();
        let db = Database::connect(&DatabaseConfig::new("sqlite::memory:"))
            .await
            .unwrap();

        for op in plan {
            let sql = op.to_sql(Dialect::Sqlite).unwrap();
            db.conn().execute(&sql, &[]).await.unwrap();
        }

        db.conn()
            .execute(
                "INSERT INTO testapp_item (title) VALUES ('hello_sqlite')",
                &[],
            )
            .await
            .unwrap();

        let row = db
            .conn()
            .fetch_one("SELECT title FROM testapp_item WHERE id = 1", &[])
            .await
            .unwrap();
        assert_eq!(row.try_string(0).unwrap().unwrap(), "hello_sqlite");
    }

    #[tokio::test]
    async fn test_sqlite_migrate_from_dir_cycle() {
        use djangors_db::{Database, DatabaseConfig};

        let db = Database::connect(&DatabaseConfig::new("sqlite::memory:"))
            .await
            .unwrap();

        let dir = std::env::temp_dir().join(format!(
            "djangors_sqlite_migtest_{:?}",
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let migration_body = "-- up\nCREATE TABLE test_cycle_sqlite (id INTEGER PRIMARY KEY AUTOINCREMENT, val TEXT);\n-- down\nDROP TABLE test_cycle_sqlite;\n";
        std::fs::write(dir.join("0001_initial.sql"), migration_body).unwrap();

        migrate_from_dir(&db, &dir).await.unwrap();

        let row = db
            .conn()
            .fetch_one(
                "SELECT name FROM djangors_migrations WHERE name = '0001_initial'",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(row.try_string(0).unwrap().unwrap(), "0001_initial");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_alter_column_type_dialect_support() {
        use djangors_db::Dialect;

        let op = Operation::AlterColumnType {
            table_name: "users".to_string(),
            column_name: "age".to_string(),
            new_sql_type: "BIGINT".to_string(),
        };

        let pg_sql = op.to_sql(Dialect::Postgres).unwrap();
        assert_eq!(
            pg_sql,
            "ALTER TABLE \"users\" ALTER COLUMN \"age\" TYPE BIGINT USING \"age\"::BIGINT;"
        );

        let sqlite_res = op.to_sql(Dialect::Sqlite);
        assert!(matches!(
            sqlite_res,
            Err(MigrationError::UnsupportedOnDialect { operation, dialect })
            if operation == "AlterColumnType" && dialect == "Sqlite"
        ));
    }
}
