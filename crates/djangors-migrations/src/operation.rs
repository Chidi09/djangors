/// Reference target for a foreign key constraint.
#[derive(Clone)]
pub struct ForeignKeyRef {
    /// Target table name.
    pub table: String,
    /// Target column name.
    pub column: String,
    /// Foreign key `ON DELETE` clause string (`"CASCADE"`, `"RESTRICT"`, etc.).
    pub on_delete: String,
}

/// Definition of a database table column for migration operations.
#[derive(Clone)]
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
#[allow(missing_docs)]
pub enum Operation {
    /// Creates a table with the specified name and columns.
    CreateTable {
        /// Name of the table to create.
        table_name: String,
        /// List of column definitions for the table.
        columns: Vec<ColumnDef>,
    },
    /// Adds a column to a table.
    AddColumn {
        table_name: String,
        column: ColumnDef,
    },
    /// Drops a column (not mechanically reversible).
    DropColumn {
        table_name: String,
        column_name: String,
    },
    /// Changes a column type (not mechanically reversible).
    AlterColumnType {
        table_name: String,
        column_name: String,
        new_sql_type: String,
    },
    /// Renames a column.
    RenameColumn {
        table_name: String,
        old_name: String,
        new_name: String,
    },
    /// Drops a table.
    DropTable { table_name: String },
}

impl Operation {
    /// Generates the SQL statement for this migration operation.
    pub fn to_sql(&self) -> String {
        match self {
            Operation::CreateTable {
                table_name,
                columns,
            } => {
                let col_sqls: Vec<String> = columns.iter().map(|col| column_sql(col)).collect();
                format!(
                    "CREATE TABLE IF NOT EXISTS \"{}\" (\n    {}\n)",
                    table_name,
                    col_sqls.join(",\n    ")
                )
            }
            Operation::AddColumn { table_name, column } => format!(
                "ALTER TABLE \"{}\" ADD COLUMN {};",
                table_name,
                column_sql(column)
            ),
            Operation::DropColumn {
                table_name,
                column_name,
            } => format!(
                "ALTER TABLE \"{}\" DROP COLUMN \"{}\";",
                table_name, column_name
            ),
            Operation::AlterColumnType {
                table_name,
                column_name,
                new_sql_type,
            } => format!(
                "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" TYPE {} USING \"{}\"::{};",
                table_name, column_name, new_sql_type, column_name, new_sql_type
            ),
            Operation::RenameColumn {
                table_name,
                old_name,
                new_name,
            } => format!(
                "ALTER TABLE \"{}\" RENAME COLUMN \"{}\" TO \"{}\";",
                table_name, old_name, new_name
            ),
            Operation::DropTable { table_name } => format!("DROP TABLE \"{}\";", table_name),
        }
    }

    /// Returns the mechanically safe reverse operation, if one exists.
    pub fn reverse(&self) -> Option<Self> {
        match self {
            Self::CreateTable { table_name, .. } => Some(Self::DropTable {
                table_name: table_name.clone(),
            }),
            Self::AddColumn { table_name, column } => Some(Self::DropColumn {
                table_name: table_name.clone(),
                column_name: column.name.clone(),
            }),
            Self::RenameColumn {
                table_name,
                old_name,
                new_name,
            } => Some(Self::RenameColumn {
                table_name: table_name.clone(),
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            }),
            Self::DropColumn { .. } | Self::AlterColumnType { .. } | Self::DropTable { .. } => None,
        }
    }

    /// Generates reverse SQL when this operation is safely invertible.
    pub fn to_down_sql(&self) -> Option<String> {
        self.reverse().map(|op| op.to_sql())
    }
}

fn column_sql(col: &ColumnDef) -> String {
    let mut sql = format!("\"{}\" {}", col.name, col.sql_type);
    if !col.nullable {
        sql.push_str(" NOT NULL");
    }
    if col.primary_key {
        sql.push_str(" PRIMARY KEY");
    }
    if col.unique && !col.primary_key {
        sql.push_str(" UNIQUE");
    }
    if let Some(default) = &col.default_sql {
        sql.push_str(&format!(" DEFAULT {}", default));
    }
    if let Some(refs) = &col.references {
        sql.push_str(&format!(
            " REFERENCES \"{}\"(\"{}\") ON DELETE {}",
            refs.table, refs.column, refs.on_delete
        ));
    }
    sql
}
