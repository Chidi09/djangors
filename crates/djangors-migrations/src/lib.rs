#![deny(missing_docs)]
//! Schema migration planning, execution, and SQL DDL generation for Djangors.

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

use std::path::{Path, PathBuf};

/// Apply migration files in filename order. Files use `-- up` and `-- down` markers;
/// `-- no-down` explicitly records an unavailable reverse migration.
pub async fn migrate_from_dir(
    db: &djangors_db::Database,
    dir: &Path,
) -> Result<(), MigrationError> {
    ensure_history(db).await?;
    let mut files = migration_files(dir)?;
    files.sort();
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let applied = sqlx::query("SELECT 1 FROM djangors_migrations WHERE name=$1")
            .bind(&name)
            .fetch_optional(db.pool())
            .await?
            .is_some();
        if applied {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        let up = section(&content, "up");
        db.transaction(|conn| {
            Box::pin(async move {
                for sql in up.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                    sqlx::query(sqlx::AssertSqlSafe(format!("{};", sql)))
                        .execute(&mut *conn)
                        .await?;
                }
                sqlx::query("INSERT INTO djangors_migrations (name) VALUES ($1)")
                    .bind(&name)
                    .execute(&mut *conn)
                    .await?;
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
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT name FROM djangors_migrations WHERE name = ANY($1) ORDER BY id DESC LIMIT $2",
    )
    .bind(&known_names)
    .bind(count as i64)
    .fetch_all(db.pool())
    .await?;
    let mut downs = Vec::new();
    for (name,) in &rows {
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
        db.transaction(|conn| {
            Box::pin(async move {
                for sql in down.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                    sqlx::query(sqlx::AssertSqlSafe(format!("{};", sql)))
                        .execute(&mut *conn)
                        .await?;
                }
                sqlx::query("DELETE FROM djangors_migrations WHERE name=$1")
                    .bind(&name)
                    .execute(&mut *conn)
                    .await?;
                Ok::<(), djangors_db::DbError>(())
            })
        })
        .await
        .map_err(MigrationError::Database)?;
    }
    Ok(())
}

async fn ensure_history(db: &djangors_db::Database) -> Result<(), MigrationError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS djangors_migrations (id SERIAL PRIMARY KEY, name TEXT UNIQUE NOT NULL, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())").execute(db.pool()).await?;
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
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS djangors_migrations (
            id SERIAL PRIMARY KEY,
            name TEXT UNIQUE NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )",
    )
    .execute(db.pool())
    .await?;

    // 2. Check if 0001_initial is applied
    let row = sqlx::query("SELECT 1 FROM djangors_migrations WHERE name = $1")
        .bind("0001_initial")
        .fetch_optional(db.pool())
        .await?;

    if row.is_some() {
        return Ok(());
    }

    // 3. Build plan and execute in transaction
    let plan = build_create_all_plan()?;
    let sqls: Vec<String> = plan.iter().map(|op| op.to_sql()).collect();

    db.transaction(|conn| {
        let sqls = sqls.clone();
        Box::pin(async move {
            for sql in sqls {
                sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                    .execute(&mut *conn)
                    .await?;
            }
            sqlx::query("INSERT INTO djangors_migrations (name) VALUES ('0001_initial')")
                .execute(&mut *conn)
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
}
