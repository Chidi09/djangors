/// Aggregate function expression for QuerySet aggregations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggExpr {
    /// COUNT aggregate function.
    Count {
        /// Field name to count, or `"*"` for all rows.
        field: &'static str,
    },
    /// SUM aggregate function.
    Sum {
        /// Field name to sum.
        field: &'static str,
    },
    /// AVG aggregate function.
    Avg {
        /// Field name to average.
        field: &'static str,
    },
    /// MIN aggregate function.
    Min {
        /// Field name to find minimum value.
        field: &'static str,
    },
    /// MAX aggregate function.
    Max {
        /// Field name to find maximum value.
        field: &'static str,
    },
}

/// Result value returned by QuerySet aggregate execution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggResult {
    /// Integer result value.
    I64(i64),
    /// Floating point result value.
    F64(f64),
    /// NULL aggregate result value.
    Null,
}
