use chrono::{DateTime, Utc};
use std::ops::{BitAnd, BitOr, Not};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    I64(i64),
    F64(f64),
    Text(String),
    Bool(bool),
    DateTime(DateTime<Utc>),
    Null,
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::I64(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::F64(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<DateTime<Utc>> for Value {
    fn from(v: DateTime<Utc>) -> Self {
        Value::DateTime(v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Lt,
    Lte,
    Gt,
    Gte,
    Contains,
    IContains,
    StartsWith,
    EndsWith,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Compare {
        field: &'static str,
        op: CompareOp,
        value: Value,
    },
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
}

impl BitAnd for Expr {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        match (self, rhs) {
            (Expr::And(mut l), Expr::And(r)) => {
                l.extend(r);
                Expr::And(l)
            }
            (Expr::And(mut l), r) => {
                l.push(r);
                Expr::And(l)
            }
            (l, Expr::And(mut r)) => {
                r.insert(0, l);
                Expr::And(r)
            }
            (l, r) => Expr::And(vec![l, r]),
        }
    }
}

impl BitOr for Expr {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        match (self, rhs) {
            (Expr::Or(mut l), Expr::Or(r)) => {
                l.extend(r);
                Expr::Or(l)
            }
            (Expr::Or(mut l), r) => {
                l.push(r);
                Expr::Or(l)
            }
            (l, Expr::Or(mut r)) => {
                r.insert(0, l);
                Expr::Or(r)
            }
            (l, r) => Expr::Or(vec![l, r]),
        }
    }
}

impl Not for Expr {
    type Output = Self;

    fn not(self) -> Self {
        Expr::Not(Box::new(self))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedCompare {
    pub field: &'static str,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnresolvedExpr {
    And(Vec<UnresolvedCompare>),
}

#[macro_export]
macro_rules! q {
    ($($field:ident = $value:expr),+ $(,)?) => {
        $crate::expr::UnresolvedExpr::And(vec![
            $(
                $crate::expr::UnresolvedCompare {
                    field: stringify!($field),
                    value: $crate::expr::Value::from($value),
                }
            ),+
        ])
    };
}
