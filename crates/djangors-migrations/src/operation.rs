pub struct ForeignKeyRef {
    pub table: String,
    pub column: String,
    pub on_delete: String, // "CASCADE" | "RESTRICT" | "SET NULL" | "NO ACTION"
}

pub struct ColumnDef {
    pub name: String,
    pub sql_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub unique: bool,
    pub default_sql: Option<String>,
    pub references: Option<ForeignKeyRef>,
}

pub enum Operation {
    CreateTable {
        table_name: String,
        columns: Vec<ColumnDef>,
    },
}

impl Operation {
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
