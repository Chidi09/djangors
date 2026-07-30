use chrono::{DateTime, Utc};
use std::ops::{BitAnd, BitOr, Not};

/// A dynamic value type used in ORM expressions and query parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// 64-bit integer value.
    I64(i64),
    /// 64-bit floating point value.
    F64(f64),
    /// Text string value.
    Text(String),
    /// Boolean value.
    Bool(bool),
    /// UTC timestamp value.
    DateTime(DateTime<Utc>),
    /// Database NULL value.
    Null,
    /// A list of values, used as the right-hand side of an `__in` lookup.
    ///
    /// Never reaches the parameter binder: [`CompareOp::In`] expands the list
    /// into one placeholder per element while compiling the SQL, so the binder
    /// only ever sees the scalar elements.
    List(Vec<Value>),
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::I64(v)
    }
}

/// Smaller integer types widen to `i64`, which is what the database side binds
/// anyway. Without these, a model field declared `i32` (the common case for
/// Django's `IntegerField`) could not be used with `q!` without a manual cast.
macro_rules! impl_from_int {
    ($($t:ty),+ $(,)?) => {
        $(
            impl From<$t> for Value {
                fn from(v: $t) -> Self {
                    Value::I64(v as i64)
                }
            }
        )+
    };
}

impl_from_int!(i8, i16, i32, u8, u16, u32);

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::F64(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::F64(v as f64)
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

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::I64(v) => write!(f, "{}", v),
            Value::F64(v) => write!(f, "{}", v),
            Value::Text(v) => write!(f, "{}", v),
            Value::Bool(v) => write!(f, "{}", v),
            Value::DateTime(v) => write!(f, "{}", v.format("%Y-%m-%d %H:%M:%S")),
            Value::Null => write!(f, "-"),
            Value::List(items) => {
                let rendered: Vec<String> = items.iter().map(|i| i.to_string()).collect();
                write!(f, "{}", rendered.join(", "))
            }
        }
    }
}

/// Comparison operators for query filtering expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    /// Exact equality comparison (`=`).
    Eq,
    /// Less-than comparison (`<`).
    Lt,
    /// Less-than-or-equal comparison (`<=`).
    Lte,
    /// Greater-than comparison (`>`).
    Gt,
    /// Greater-than-or-equal comparison (`>=`).
    Gte,
    /// Case-sensitive substring search (`LIKE '%val%'`).
    Contains,
    /// Case-insensitive substring search (`ILIKE '%val%'`).
    IContains,
    /// Prefix string match (`LIKE 'val%'`).
    StartsWith,
    /// Suffix string match (`LIKE '%val'`).
    EndsWith,
    /// Inequality comparison (`<>`). Lookup suffix `ne`.
    Ne,
    /// Case-insensitive exact match (`ILIKE 'val'`, no wildcards). Lookup suffix `iexact`.
    IExact,
    /// Membership test (`IN (...)`). Lookup suffix `in`; expects [`Value::List`].
    ///
    /// An empty list compiles to `FALSE` rather than the syntactically invalid
    /// `IN ()`, matching Django's behaviour for `__in=[]`.
    In,
    /// NULL test (`IS NULL` / `IS NOT NULL`). Lookup suffix `isnull`; expects
    /// [`Value::Bool`], where `false` inverts the test.
    IsNull,
    /// Case-sensitive POSIX regular-expression match (`~`). Lookup suffix `regex`.
    Regex,
    /// Case-insensitive POSIX regular-expression match (`~*`). Lookup suffix `iregex`.
    IRegex,
}

/// Resolved boolean expression tree for query WHERE clauses.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A single field comparison expression.
    Compare {
        /// Target field name on the model.
        field: &'static str,
        /// Comparison operator.
        op: CompareOp,
        /// Comparison value.
        value: Value,
    },
    /// Conjunction of multiple expressions (AND).
    And(Vec<Expr>),
    /// Disjunction of multiple expressions (OR).
    Or(Vec<Expr>),
    /// Negation of an expression (NOT).
    Not(Box<Expr>),
    /// Comparison of one column against another on the same row.
    ///
    /// This is Django's `F()` on the *filter* side (`.filter(q_f!(paid_at gte
    /// due_at))`), as opposed to [`SetExpr::FieldOp`], which is `F()` on the
    /// *update* side. Both operands are resolved field names, so neither
    /// contributes a bind parameter.
    CompareField {
        /// Left-hand field name on the model.
        left: &'static str,
        /// Comparison operator.
        op: CompareOp,
        /// Right-hand field name on the model.
        right: &'static str,
    },
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

/// Raw field comparison before lookup suffix resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedCompare {
    /// Field name (possibly containing lookup suffix like `field__contains`).
    pub field: &'static str,
    /// Comparison value.
    pub value: Value,
}

/// Raw column-to-column comparison before lookup suffix resolution.
///
/// Produced by the [`q_f!`](crate::q_f) macro; the counterpart to
/// [`UnresolvedCompare`] for the right-hand side being a field rather than a
/// value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedFieldCompare {
    /// Left-hand field name (possibly carrying a lookup suffix).
    pub left: &'static str,
    /// Right-hand field name (never carries a suffix).
    pub right: &'static str,
}

/// Unresolved filter expression generated by the `q!` macro.
///
/// This is a tree: [`And`](Self::And) is the leaf group produced by a single
/// `q!(...)` invocation, while [`All`](Self::All), [`Any`](Self::Any) and
/// [`Negate`](Self::Negate) compose those leaves. The [`BitAnd`], [`BitOr`] and
/// [`Not`] operators build the composite variants, which is what makes Django's
/// `Q(a=1) | Q(b=2)` spell as `q!(a = 1) | q!(b = 2)`.
///
/// ```
/// # use djangors_orm::q;
/// let expr = (q!(status = "open") | q!(status = "pending")) & !q!(archived = true);
/// # let _ = expr;
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum UnresolvedExpr {
    /// Conjunction of unresolved field comparisons (the `q!` leaf).
    And(Vec<UnresolvedCompare>),
    /// Conjunction of unresolved column-to-column comparisons (the `q_f!` leaf).
    AndFields(Vec<UnresolvedFieldCompare>),
    /// Conjunction of sub-expressions.
    All(Vec<UnresolvedExpr>),
    /// Disjunction of sub-expressions.
    Any(Vec<UnresolvedExpr>),
    /// Negation of a sub-expression.
    Negate(Box<UnresolvedExpr>),
}

impl BitAnd for UnresolvedExpr {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        match (self, rhs) {
            (UnresolvedExpr::All(mut l), UnresolvedExpr::All(r)) => {
                l.extend(r);
                UnresolvedExpr::All(l)
            }
            (UnresolvedExpr::All(mut l), r) => {
                l.push(r);
                UnresolvedExpr::All(l)
            }
            (l, UnresolvedExpr::All(mut r)) => {
                r.insert(0, l);
                UnresolvedExpr::All(r)
            }
            (l, r) => UnresolvedExpr::All(vec![l, r]),
        }
    }
}

impl BitOr for UnresolvedExpr {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        match (self, rhs) {
            (UnresolvedExpr::Any(mut l), UnresolvedExpr::Any(r)) => {
                l.extend(r);
                UnresolvedExpr::Any(l)
            }
            (UnresolvedExpr::Any(mut l), r) => {
                l.push(r);
                UnresolvedExpr::Any(l)
            }
            (l, UnresolvedExpr::Any(mut r)) => {
                r.insert(0, l);
                UnresolvedExpr::Any(r)
            }
            (l, r) => UnresolvedExpr::Any(vec![l, r]),
        }
    }
}

impl Not for UnresolvedExpr {
    type Output = Self;

    fn not(self) -> Self {
        UnresolvedExpr::Negate(Box::new(self))
    }
}

/// Constructs an [`UnresolvedExpr`] comparing one column to another on the same row.
///
/// The filter-side counterpart to [`F`]: where `q!(count = 5)` compares a column
/// to a bound value, `q_f!(paid_at__gte due_at)` compares two columns. As in
/// Django, the lookup suffix rides on the left-hand field.
///
/// ```
/// # use djangors_orm::q_f;
/// let overdue = q_f!(paid_at__gte due_at);
/// # let _ = overdue;
/// ```
#[macro_export]
macro_rules! q_f {
    ($($left:ident $right:ident),+ $(,)?) => {
        $crate::expr::UnresolvedExpr::AndFields(vec![
            $(
                $crate::expr::UnresolvedFieldCompare {
                    left: stringify!($left),
                    right: stringify!($right),
                }
            ),+
        ])
    };
}

/// Constructs an [`UnresolvedExpr`] for filtering querysets.
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

/// Arithmetic operators for UPDATE set expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    /// Addition operator (`+`).
    Add,
    /// Subtraction operator (`-`).
    Sub,
    /// Multiplication operator (`*`).
    Mul,
    /// Division operator (`/`).
    Div,
}

/// Expression specifying a value update in an UPDATE query.
#[derive(Debug, Clone, PartialEq)]
pub enum SetExpr {
    /// A literal new value.
    Literal(Value),
    /// An in-database field arithmetic operation (e.g. `col = col + 1`).
    FieldOp {
        /// Target field name on the model.
        field: &'static str,
        /// Arithmetic operator.
        op: ArithOp,
        /// Right-hand operand value.
        operand: Value,
    },
}

/// Django's F() — a reference to a field's CURRENT value in the database.
pub struct F(pub &'static str);

impl std::ops::Add<i64> for F {
    type Output = SetExpr;
    fn add(self, rhs: i64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Add,
            operand: Value::I64(rhs),
        }
    }
}

impl std::ops::Sub<i64> for F {
    type Output = SetExpr;
    fn sub(self, rhs: i64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Sub,
            operand: Value::I64(rhs),
        }
    }
}

impl std::ops::Mul<i64> for F {
    type Output = SetExpr;
    fn mul(self, rhs: i64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Mul,
            operand: Value::I64(rhs),
        }
    }
}

impl std::ops::Div<i64> for F {
    type Output = SetExpr;
    fn div(self, rhs: i64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Div,
            operand: Value::I64(rhs),
        }
    }
}

impl std::ops::Add<f64> for F {
    type Output = SetExpr;
    fn add(self, rhs: f64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Add,
            operand: Value::F64(rhs),
        }
    }
}

impl std::ops::Sub<f64> for F {
    type Output = SetExpr;
    fn sub(self, rhs: f64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Sub,
            operand: Value::F64(rhs),
        }
    }
}

impl std::ops::Mul<f64> for F {
    type Output = SetExpr;
    fn mul(self, rhs: f64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Mul,
            operand: Value::F64(rhs),
        }
    }
}

impl std::ops::Div<f64> for F {
    type Output = SetExpr;
    fn div(self, rhs: f64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Div,
            operand: Value::F64(rhs),
        }
    }
}

/// Trait implemented by types that can be converted into a [`SetExpr`].
pub trait IntoSetExpr {
    /// Converts `self` into a [`SetExpr`].
    fn into_set_expr(self) -> SetExpr;
}

impl IntoSetExpr for SetExpr {
    fn into_set_expr(self) -> SetExpr {
        self
    }
}

impl<T: Into<Value>> IntoSetExpr for T {
    fn into_set_expr(self) -> SetExpr {
        SetExpr::Literal(self.into())
    }
}

/// Constructs a vector of field assignment tuples for [`QuerySet::update`](crate::QuerySet::update).
#[macro_export]
macro_rules! set {
    ($($field:ident = $value:expr),+ $(,)?) => {
        vec![
            $(
                (stringify!($field), $crate::expr::IntoSetExpr::into_set_expr($value))
            ),+
        ]
    };
}
