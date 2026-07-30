//! Bind parameters and typed NULL kinds.

/// The SQL type a NULL parameter must be bound as.
///
/// Postgres rejects a mismatched parameter type even for NULL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullKind {
    /// 64-bit integer NULL.
    I64,
    /// 64-bit float NULL.
    F64,
    /// Text string NULL.
    Text,
    /// Boolean NULL.
    Bool,
    /// Timestamp NULL.
    DateTime,
    /// Byte vector NULL.
    Bytes,
}

/// A value that can be bound to a database query parameter across backends.
#[derive(Debug, Clone, PartialEq)]
pub enum BindValue {
    /// 64-bit integer.
    I64(i64),
    /// 64-bit float.
    F64(f64),
    /// Text string.
    Text(String),
    /// Boolean.
    Bool(bool),
    /// UTC Timestamp.
    DateTime(chrono::DateTime<chrono::Utc>),
    /// Byte vector.
    Bytes(Vec<u8>),
    /// Typed NULL value.
    Null(NullKind),
}
