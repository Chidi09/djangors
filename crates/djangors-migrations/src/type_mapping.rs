use crate::error::MigrationError;
use djangors_db::Dialect;
use djangors_orm::{FieldKind, FieldMeta};

/// Maps an ORM [`FieldMeta`] definition to its corresponding SQL type string.
pub fn field_meta_to_sql_type(
    field: &FieldMeta,
    dialect: Dialect,
) -> Result<String, MigrationError> {
    sql_type_for(
        &field.kind,
        field.max_length,
        field.auto,
        field.name,
        dialect,
    )
}

/// Maps a field's kind/max_length/auto to its corresponding SQL type string for a given dialect.
pub fn sql_type_for(
    kind: &FieldKind,
    max_length: Option<u32>,
    auto: bool,
    field_name: &str,
    dialect: Dialect,
) -> Result<String, MigrationError> {
    match dialect {
        Dialect::Postgres => match kind {
            FieldKind::Char | FieldKind::Email | FieldKind::Url | FieldKind::Slug => {
                if let Some(n) = max_length {
                    Ok(format!("VARCHAR({})", n))
                } else {
                    Err(MigrationError::MissingMaxLength {
                        field: field_name.to_string(),
                    })
                }
            }
            FieldKind::Text | FieldKind::FileField => Ok("TEXT".to_string()),
            FieldKind::Integer => {
                if auto {
                    Ok("SERIAL".to_string())
                } else {
                    Ok("INTEGER".to_string())
                }
            }
            FieldKind::BigInt => {
                if auto {
                    Ok("BIGSERIAL".to_string())
                } else {
                    Ok("BIGINT".to_string())
                }
            }
            FieldKind::Float => Ok("DOUBLE PRECISION".to_string()),
            FieldKind::Decimal {
                max_digits,
                decimal_places,
            } => Ok(format!("NUMERIC({}, {})", max_digits, decimal_places)),
            FieldKind::Boolean => Ok("BOOLEAN".to_string()),
            FieldKind::Date => Ok("DATE".to_string()),
            FieldKind::DateTime => Ok("TIMESTAMPTZ".to_string()),
            FieldKind::Time => Ok("TIME".to_string()),
            FieldKind::Duration => Ok("INTERVAL".to_string()),
            FieldKind::Uuid => Ok("UUID".to_string()),
            FieldKind::Ip => Ok("INET".to_string()),
            FieldKind::Binary => Ok("BYTEA".to_string()),
            FieldKind::Json => Ok("JSONB".to_string()),
        },
        Dialect::Sqlite => match kind {
            FieldKind::Char | FieldKind::Email | FieldKind::Url | FieldKind::Slug => {
                Ok("TEXT".to_string())
            }
            FieldKind::Text | FieldKind::FileField => Ok("TEXT".to_string()),
            FieldKind::Integer | FieldKind::BigInt => {
                if auto {
                    Ok("INTEGER PRIMARY KEY AUTOINCREMENT".to_string())
                } else {
                    Ok("INTEGER".to_string())
                }
            }
            FieldKind::Float => Ok("REAL".to_string()),
            FieldKind::Decimal { .. } => Ok("NUMERIC".to_string()),
            FieldKind::Boolean => Ok("INTEGER".to_string()),
            FieldKind::Date
            | FieldKind::DateTime
            | FieldKind::Time
            | FieldKind::Duration
            | FieldKind::Uuid
            | FieldKind::Ip
            | FieldKind::Json => Ok("TEXT".to_string()),
            FieldKind::Binary => Ok("BLOB".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_field_maps_to_the_same_sql_type_as_text() {
        let file_field_sql = sql_type_for(
            &FieldKind::FileField,
            None,
            false,
            "attachment",
            Dialect::Postgres,
        )
        .unwrap();
        let text_sql =
            sql_type_for(&FieldKind::Text, None, false, "body", Dialect::Postgres).unwrap();
        assert_eq!(file_field_sql, text_sql);
        assert_eq!(file_field_sql, "TEXT");
    }

    #[test]
    fn sqlite_auto_primary_key_yields_integer_primary_key_autoincrement() {
        let sql_int = sql_type_for(&FieldKind::Integer, None, true, "id", Dialect::Sqlite).unwrap();
        assert_eq!(sql_int, "INTEGER PRIMARY KEY AUTOINCREMENT");

        let sql_bigint =
            sql_type_for(&FieldKind::BigInt, None, true, "id", Dialect::Sqlite).unwrap();
        assert_eq!(sql_bigint, "INTEGER PRIMARY KEY AUTOINCREMENT");
    }
}

#[cfg(test)]
mod sqlite_ddl_tests {
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
        };

        let sql = op.to_sql();
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
}
