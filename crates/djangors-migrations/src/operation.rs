/// Reference target for a foreign key constraint.
pub struct ForeignKeyRef {
    /// Target table name.
    pub table: String,
    /// Target column name.
    pub column: String,
    /// Foreign key `ON DELETE` clause string (`"CASCADE"`, `"RESTRICT"`, etc.).
    pub on_delete: String,
}

/// Definition of a database table column for migration operations.
pub struct ColumnDef {
    /// Column name in the database.
    pub name: String,
    /// SQL type definition string (e.g., `VARCHAR(255)`).
    pub nullable: bool,
    /// Whether the column allows NULL values.
    pub sql_type: String,
    /// Whether the column is part of the primary key.
    pub primary_key: bool,
    /// Whether the column has a UNIQUE constraint.
    pub unique: bool,
    /// Optional default SQL expression.
    pub default_sql: Option<String>,
    /// Optional foreign key constraint reference.
    pub references: Option<ForeignKeyRef>,
}

/// A DDL operation step in a migration plan.
pub enum Operation {
    /// Creates a table with the specified name and columns.
    CreateTable {
        /// Name of the table to create.
        table_name: String,
        /// List of column definitions for the table.
        columns: Vec<ColumnDef>,
    },
}

impl Operation {
    /// Generates the SQL statement for this migration operation.
    pub fn to_sql(&self) -> String {
        match self {
            Operation::CreateTable {
                table_name,
                columns,
            } => {
                let col_sqls: Vec<String> = columns
                    .iter()
                    .map(|col| {
                        let mut sql = format!("{} {}", col.name, col.sql_type);
                        if !col.nullable {
                            sql.push_str(" NOT NULL");
                        }
                        if col.primary_key {
                            sql.push_str(" PRIMARY KEY");
                        }
                        if col.unique && !col.primary_key {
                            sql.push_str(" UNIQUE");
                        }
                        if let Some(ref default) = col.default_sql {
                            sql.push_str(&format!(" DEFAULT {}", default));
                        }
                        if let Some(ref refs) = col.references {
                            sql.push_str(&format!(
                                " REFERENCES {}({}) ON DELETE {}",
                                refs.table, refs.column, refs.on_delete
                            ));
                        }
                        sql
                    })
                    .collect();
                format!(
                    "CREATE TABLE IF NOT EXISTS {} (\n    {}\n)",
                    table_name,
                    col_sqls.join(",\n    ")
                )
            }
        }
    }
}
