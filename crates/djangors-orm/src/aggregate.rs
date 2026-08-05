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

impl AggResult {
    /// Convert a row [`crate::expr::Value`] back into an aggregate result.
    pub fn from_value(v: &crate::expr::Value) -> Self {
        match v {
            crate::expr::Value::I64(n) => AggResult::I64(*n),
            crate::expr::Value::F64(f) => AggResult::F64(*f),
            crate::expr::Value::Null => AggResult::Null,
            other => {
                if let Ok(f) = other.to_string().parse::<f64>() {
                    AggResult::F64(f)
                } else {
                    AggResult::Null
                }
            }
        }
    }
}

/// Scalar database function expressions usable in `annotate` and `values`.
#[derive(Debug, Clone)]
pub enum FuncExpr {
    /// `COALESCE(field, default)` — returns the first non-null.
    Coalesce {
        /// Field name whose value is returned when non-null.
        field: &'static str,
        /// Value substituted when `field` is NULL.
        default: crate::expr::Value,
    },
    /// `LOWER(field)` — lowercase.
    Lower {
        /// Field name to lowercase.
        field: &'static str,
    },
    /// `UPPER(field)` — uppercase.
    Upper {
        /// Field name to uppercase.
        field: &'static str,
    },
    /// `field1 || field2 || ...` — string concatenation.
    Concat {
        /// Field names joined with the concatenation operator.
        fields: Vec<&'static str>,
    },
    /// `LENGTH(field)` — string length.
    Length {
        /// Field name whose character length is computed.
        field: &'static str,
    },
}

impl FuncExpr {
    /// Build a `COALESCE(field, default)` expression.
    pub fn coalesce(field: &'static str, default: impl Into<crate::expr::Value>) -> Self {
        Self::Coalesce {
            field,
            default: default.into(),
        }
    }

    /// Build a `LOWER(field)` expression.
    pub fn lower(field: &'static str) -> Self {
        Self::Lower { field }
    }

    /// Build an `UPPER(field)` expression.
    pub fn upper(field: &'static str) -> Self {
        Self::Upper { field }
    }

    /// Build a `fields[0] || fields[1] || ...` concatenation expression.
    pub fn concat(fields: &[&'static str]) -> Self {
        Self::Concat {
            fields: fields.to_vec(),
        }
    }

    /// Build a `LENGTH(field)` expression.
    pub fn length(field: &'static str) -> Self {
        Self::Length { field }
    }

    /// Produce the SQL expression for this function call.
    pub fn to_sql(&self, dialect: &djangors_db::Dialect) -> String {
        match self {
            Self::Coalesce { field, default } => {
                let col = dialect.quote_ident(field);
                let def = match default {
                    crate::expr::Value::I64(n) => n.to_string(),
                    crate::expr::Value::F64(f) => f.to_string(),
                    crate::expr::Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
                    crate::expr::Value::Bool(b) => {
                        (if *b { "TRUE" } else { "FALSE" }).to_string()
                    }
                    crate::expr::Value::Null => "NULL".to_string(),
                    _ => "NULL".to_string(),
                };
                format!("COALESCE({col}, {def})")
            }
            Self::Lower { field } => {
                format!("LOWER({})", dialect.quote_ident(field))
            }
            Self::Upper { field } => {
                format!("UPPER({})", dialect.quote_ident(field))
            }
            Self::Length { field } => {
                format!("LENGTH({})", dialect.quote_ident(field))
            }
            Self::Concat { fields } => {
                let quoted: Vec<String> =
                    fields.iter().map(|f| dialect.quote_ident(f)).collect();
                quoted.join(" || ")
            }
        }
    }

    /// A unique alias-safe name for this expression.
    pub fn alias(&self) -> String {
        match self {
            Self::Coalesce { field, .. } => format!("coalesce_{field}"),
            Self::Lower { field } => format!("lower_{field}"),
            Self::Upper { field } => format!("upper_{field}"),
            Self::Length { field } => format!("length_{field}"),
            Self::Concat { fields } => format!("concat_{}", fields.join("_")),
        }
    }
}
