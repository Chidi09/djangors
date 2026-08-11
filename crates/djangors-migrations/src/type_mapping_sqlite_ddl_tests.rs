use super::*;
use crate::operation::{ColumnDef, Operation};
use djangors_orm::FieldKind;

/// The generated `CREATE TABLE` must actually execute on SQLite.
///
/// Added during review of the SQLite port. The existing coverage tested
/// `sql_type_for` in isolation (a string comparison) and separately did an ORM
/// round-trip against a *hand-written* `CREATE TABLE` — so nothing checked that
/// what the migration generator emits is valid SQLite.
///
/// That gap mattered specifically here: on SQLite an auto primary key is
/// expressed as the type string `INTEGER PRIMARY KEY AUTOINCREMENT`, so if
/// `column_sql` also appended its usual `NOT NULL` / `PRIMARY KEY` for that
/// column, SQLite would reject the statement with "table has more than one
/// primary key". Only executing the real generated DDL proves it doesn't.
#[tokio::test]
async fn generated_create_table_executes_on_sqlite() {
    let pk_type = sql_type_for(&FieldKind::BigInt, None, true, "id", Dialect::Sqlite).unwrap();
    let name_type =
        sql_type_for(&FieldKind::Char, Some(200), false, "name", Dialect::Sqlite).unwrap();
    let flag_type =
        sql_type_for(&FieldKind::Boolean, None, false, "flag", Dialect::Sqlite).unwrap();

    let op = Operation::CreateTable {
        table_name: "review_sqlite_ddl".to_string(),
        columns: vec![
            ColumnDef {
                name: "id".to_string(),
                sql_type: pk_type,
                nullable: false,
                primary_key: true,
                unique: false,
                default_sql: None,
                references: None,
            },
            ColumnDef {
                name: "name".to_string(),
                sql_type: name_type,
                nullable: false,
                primary_key: false,
                unique: false,
                default_sql: None,
                references: None,
            },
            ColumnDef {
                name: "flag".to_string(),
                sql_type: flag_type,
                nullable: true,
                primary_key: false,
                unique: false,
                default_sql: None,
                references: None,
            },
        ],
        check_constraints: vec![],
    };

    let sql = op.to_sql(Dialect::Sqlite).unwrap();
    assert!(
        !sql.contains("PRIMARY KEY AUTOINCREMENT NOT NULL"),
        "the auto-PK column must not get an extra NOT NULL appended: {sql}"
    );

    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    sqlx::query(sqlx::AssertSqlSafe(sql.clone()))
        .execute(&pool)
        .await
        .unwrap_or_else(|e| panic!("generated DDL rejected by SQLite: {e}\n---\n{sql}"));

    // AUTOINCREMENT must actually assign a rowid rather than the column being a
    // plain INTEGER that requires an explicit value.
    sqlx::query(sqlx::AssertSqlSafe(
        "INSERT INTO \"review_sqlite_ddl\" (\"name\") VALUES ('x')".to_string(),
    ))
    .execute(&pool)
    .await
    .expect("insert without explicit pk");

    let (id,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(
        "SELECT \"id\" FROM \"review_sqlite_ddl\"".to_string(),
    ))
    .fetch_one(&pool)
    .await
    .expect("read back generated pk");
    assert_eq!(id, 1, "AUTOINCREMENT should have assigned rowid 1");
}
