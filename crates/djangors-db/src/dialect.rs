//! Database dialect representation for SQL syntax generation.

/// Supported SQL dialects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// PostgreSQL database dialect.
    Postgres,
    /// SQLite database dialect.
    Sqlite,
}

impl Dialect {
    /// Returns the parameter placeholder for this dialect.
    ///
    /// `$1`, `$2`, … for Postgres; `?` for SQLite.
    pub fn placeholder(&self, index: usize) -> String {
        match self {
            Dialect::Postgres => format!("${}", index),
            Dialect::Sqlite => "?".to_string(),
        }
    }

    /// Returns the case-insensitive LIKE operator for this dialect.
    ///
    /// `ILIKE` for Postgres; `LIKE` for SQLite (whose LIKE is already
    /// ASCII-case-insensitive, so this is the correct equivalent, not a downgrade).
    pub fn ilike(&self) -> &'static str {
        match self {
            Dialect::Postgres => "ILIKE",
            Dialect::Sqlite => "LIKE",
        }
    }

    /// Double-quotes an identifier name for both dialects.
    ///
    /// `"name"` for both dialects (SQLite accepts double-quoted identifiers).
    pub fn quote_ident(&self, name: &str) -> String {
        format!("\"{name}\"")
    }

    /// Casts an expression to a float/real type.
    ///
    /// `::float8` for Postgres; `CAST(… AS REAL)` shape for SQLite.
    pub fn cast_float(&self, expr: &str) -> String {
        match self {
            Dialect::Postgres => format!("{}::float8", expr),
            Dialect::Sqlite => format!("CAST({} AS REAL)", expr),
        }
    }
}
