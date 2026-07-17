pub mod error;
pub mod operation;
pub mod plan;
pub mod type_mapping;

pub use error::MigrationError;
pub use operation::{ColumnDef, ForeignKeyRef, Operation};
pub use plan::build_create_all_plan;

pub async fn migrate(db: &djangors_db::Database) -> Result<(), MigrationError> {
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
