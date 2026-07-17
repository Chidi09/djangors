#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggExpr {
    Count { field: &'static str }, // use "*" as field for COUNT(*)
    Sum { field: &'static str },
    Avg { field: &'static str },
    Min { field: &'static str },
    Max { field: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggResult {
    I64(i64),
    F64(f64),
    Null,
}
