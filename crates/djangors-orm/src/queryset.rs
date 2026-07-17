use crate::error::{FromRow, OrmError};
use crate::expr::{CompareOp, Expr, UnresolvedExpr, Value};
use crate::meta::Model;
use std::marker::PhantomData;

#[derive(Debug)]
pub struct QuerySet<T: Model + FromRow> {
    filters: Vec<Expr>,
    order_by: Vec<(String, bool)>, // (column, descending)
    limit: Option<i64>,
    offset: Option<i64>,
    _marker: PhantomData<T>,
}

impl<T: Model + FromRow> Clone for QuerySet<T> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            limit: self.limit,
            offset: self.offset,
            _marker: PhantomData,
        }
    }
}

impl<T: Model + FromRow> Default for QuerySet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Model + FromRow> QuerySet<T> {
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
            order_by: Vec::new(),
            limit: None,
            offset: None,
            _marker: PhantomData,
        }
    }

    pub fn filter(mut self, expr: UnresolvedExpr) -> Result<Self, OrmError> {
        let UnresolvedExpr::And(compares) = expr;
        let mut resolved = Vec::new();
        for comp in compares {
            let (field_name, suffix) = split_field_lookup(comp.field);
            let meta = T::meta();
            let exists = meta.fields.iter().any(|f| f.name == field_name)
                || meta.relations.iter().any(|r| r.field_name == field_name);
            if !exists {
                return Err(OrmError::FieldNotFound {
                    field: field_name.to_string(),
                    model: meta.struct_name,
                });
            }
            let op = suffix_to_op(suffix);
            resolved.push(Expr::Compare {
                field: field_name,
                op,
                value: comp.value,
            });
        }
        self.filters.push(Expr::And(resolved));
        Ok(self)
    }

    /// Order results by the given field. A leading `-` means descending.
    ///
    /// # Errors
    /// Returns [`OrmError::FieldNotFound`] if `field` does not match any field
    /// or relation on `T`. This validation exists specifically so a caller can
    /// never get an unvalidated string interpolated into the generated SQL's
    /// `ORDER BY` clause — column/table identifiers can't be bound as query
    /// parameters the way values can, so rejecting anything that isn't a known
    /// column name (from `T::meta()`, not caller input) is the only safe
    /// mitigation against SQL injection via this method.
    pub fn order_by(mut self, field: &str) -> Result<Self, OrmError> {
        let (clean_field, desc) = if let Some(stripped) = field.strip_prefix('-') {
            (stripped, true)
        } else {
            (field, false)
        };
        let meta = T::meta();
        let col = if let Some(f) = meta.fields.iter().find(|f| f.name == clean_field) {
            f.column_name.to_string()
        } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == clean_field) {
            r.field_name.to_string()
        } else {
            return Err(OrmError::FieldNotFound {
                field: clean_field.to_string(),
                model: meta.struct_name,
            });
        };
        self.order_by.push((col, desc));
        Ok(self)
    }

    pub fn limit(mut self, n: i64) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn offset(mut self, n: i64) -> Self {
        self.offset = Some(n);
        self
    }

    pub fn compile_select(&self) -> (String, Vec<Value>) {
        self.compile_select_custom("*")
    }

    pub fn compile_select_custom(&self, select_list: &str) -> (String, Vec<Value>) {
        let meta = T::meta();
        let mut sql = format!("SELECT {} FROM {}", select_list, meta.table_name);
        let mut params = Vec::new();
        let mut param_idx = 1;

        let field_to_col = |field_name: &str| -> String {
            if let Some(f) = meta.fields.iter().find(|f| f.name == field_name) {
                f.column_name.to_string()
            } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field_name) {
                r.field_name.to_string()
            } else {
                field_name.to_string()
            }
        };

        if !self.filters.is_empty() {
            let combined = Expr::And(self.filters.clone());
            let where_clause =
                compile_expr_sql(&combined, &field_to_col, &mut params, &mut param_idx);
            sql.push_str(" WHERE ");
            sql.push_str(&where_clause);
        }

        let order_source = if !self.order_by.is_empty() {
            self.order_by.clone()
        } else {
            meta.ordering
                .iter()
                .map(|&field| {
                    let (clean_field, desc) = if let Some(stripped) = field.strip_prefix('-') {
                        (stripped, true)
                    } else {
                        (field, false)
                    };
                    let col = if let Some(f) = meta.fields.iter().find(|f| f.name == clean_field) {
                        f.column_name.to_string()
                    } else if let Some(r) =
                        meta.relations.iter().find(|r| r.field_name == clean_field)
                    {
                        r.field_name.to_string()
                    } else {
                        clean_field.to_string()
                    };
                    (col, desc)
                })
                .collect()
        };

        if !order_source.is_empty() {
            let order_parts: Vec<String> = order_source
                .iter()
                .map(|(col, desc)| {
                    if *desc {
                        format!("{} DESC", col)
                    } else {
                        format!("{} ASC", col)
                    }
                })
                .collect();
            sql.push_str(" ORDER BY ");
            sql.push_str(&order_parts.join(", "));
        }

        if let Some(limit) = self.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }
        if let Some(offset) = self.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }

        (sql, params)
    }

    pub async fn all(&self, db: &djangors_db::Database) -> Result<Vec<T>, OrmError> {
        let (sql, params) = self.compile_select();
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for val in &params {
            query = match val {
                Value::I64(v) => query.bind(*v),
                Value::F64(v) => query.bind(*v),
                Value::Text(v) => query.bind(v.clone()),
                Value::Bool(v) => query.bind(*v),
                Value::DateTime(v) => query.bind(*v),
                Value::Null => query.bind(None::<i64>),
            };
        }
        let rows = query.fetch_all(db.pool()).await?;
        let mut results = Vec::new();
        for row in rows {
            results.push(T::from_row(&row)?);
        }
        Ok(results)
    }

    pub async fn get(&self, db: &djangors_db::Database) -> Result<T, OrmError> {
        let mut cloned = self.clone();
        cloned.limit = Some(2);
        let results = cloned.all(db).await?;
        if results.is_empty() {
            Err(OrmError::NotFound {
                model: T::meta().struct_name,
            })
        } else if results.len() > 1 {
            Err(OrmError::MultipleObjectsReturned {
                model: T::meta().struct_name,
            })
        } else {
            Ok(results.into_iter().next().unwrap())
        }
    }

    pub async fn first(&self, db: &djangors_db::Database) -> Result<Option<T>, OrmError> {
        let mut cloned = self.clone();
        cloned.limit = Some(1);
        let results = cloned.all(db).await?;
        Ok(results.into_iter().next())
    }

    pub async fn exists(&self, db: &djangors_db::Database) -> Result<bool, OrmError> {
        let mut cloned = self.clone();
        cloned.limit = Some(1);
        let (sql, params) = cloned.compile_select_custom("1");
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for val in &params {
            query = match val {
                Value::I64(v) => query.bind(*v),
                Value::F64(v) => query.bind(*v),
                Value::Text(v) => query.bind(v.clone()),
                Value::Bool(v) => query.bind(*v),
                Value::DateTime(v) => query.bind(*v),
                Value::Null => query.bind(None::<i64>),
            };
        }
        let row_opt = query.fetch_optional(db.pool()).await?;
        Ok(row_opt.is_some())
    }

    pub async fn count(&self, db: &djangors_db::Database) -> Result<i64, OrmError> {
        let (sql, params) = self.compile_select_custom("COUNT(*)");
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for val in &params {
            query = match val {
                Value::I64(v) => query.bind(*v),
                Value::F64(v) => query.bind(*v),
                Value::Text(v) => query.bind(v.clone()),
                Value::Bool(v) => query.bind(*v),
                Value::DateTime(v) => query.bind(*v),
                Value::Null => query.bind(None::<i64>),
            };
        }
        let row = query.fetch_one(db.pool()).await?;
        use sqlx::Row;
        let count: i64 = row.try_get(0)?;
        Ok(count)
    }

    pub async fn aggregate(
        &self,
        db: &djangors_db::Database,
        aggs: Vec<crate::aggregate::AggExpr>,
    ) -> Result<Vec<crate::aggregate::AggResult>, OrmError> {
        let meta = T::meta();

        // 1. Validate every AggExpr's field (except Count's "*" sentinel) against T::meta().fields
        for agg in &aggs {
            let field_name = match agg {
                crate::aggregate::AggExpr::Count { field } => *field,
                crate::aggregate::AggExpr::Sum { field } => *field,
                crate::aggregate::AggExpr::Avg { field } => *field,
                crate::aggregate::AggExpr::Min { field } => *field,
                crate::aggregate::AggExpr::Max { field } => *field,
            };

            if field_name != "*" {
                let exists = meta.fields.iter().any(|f| f.name == field_name)
                    || meta.relations.iter().any(|r| r.field_name == field_name);
                if !exists {
                    return Err(OrmError::FieldNotFound {
                        field: field_name.to_string(),
                        model: meta.struct_name,
                    });
                }
            }
        }

        // 2. Build SELECT list and custom SQL.
        let field_to_col = |field_name: &str| -> String {
            if let Some(f) = meta.fields.iter().find(|f| f.name == field_name) {
                f.column_name.to_string()
            } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field_name) {
                r.field_name.to_string()
            } else {
                field_name.to_string()
            }
        };

        let mut agg_sql_parts = Vec::new();
        for agg in &aggs {
            let part = match agg {
                crate::aggregate::AggExpr::Count { field } => {
                    if *field == "*" {
                        "COUNT(*)".to_string()
                    } else {
                        format!("COUNT({})", field_to_col(field))
                    }
                }
                // Cast to float8 so the result is always decodable as
                // Option<f64> client-side, regardless of the underlying
                // column's numeric type. Without this, Postgres returns
                // different native types depending on both the aggregate
                // function and the column type — SUM(integer) -> bigint,
                // SUM(bigint)/AVG(integer|bigint) -> numeric, MIN/MAX(real)
                // -> double precision, etc. Casting in SQL sidesteps having
                // to handle every combination on the Rust side.
                crate::aggregate::AggExpr::Sum { field } => {
                    format!("SUM({})::float8", field_to_col(field))
                }
                crate::aggregate::AggExpr::Avg { field } => {
                    format!("AVG({})::float8", field_to_col(field))
                }
                crate::aggregate::AggExpr::Min { field } => {
                    format!("MIN({})::float8", field_to_col(field))
                }
                crate::aggregate::AggExpr::Max { field } => {
                    format!("MAX({})::float8", field_to_col(field))
                }
            };
            agg_sql_parts.push(part);
        }

        let select_list = agg_sql_parts.join(", ");

        // A single aggregate row has nothing to order or paginate — reuse
        // compile_select_custom (so the WHERE clause stays in sync with the
        // regular filter-compilation path) via a clone with order_by/limit/
        // offset cleared, rather than duplicating the WHERE-building logic.
        let mut clean_qs = self.clone();
        clean_qs.order_by.clear();
        clean_qs.limit = None;
        clean_qs.offset = None;

        let (sql, params) = clean_qs.compile_select_custom(&select_list);

        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for val in &params {
            query = match val {
                Value::I64(v) => query.bind(*v),
                Value::F64(v) => query.bind(*v),
                Value::Text(v) => query.bind(v.clone()),
                Value::Bool(v) => query.bind(*v),
                Value::DateTime(v) => query.bind(*v),
                Value::Null => query.bind(None::<i64>),
            };
        }

        let row = query.fetch_one(db.pool()).await?;
        use sqlx::Row;

        let mut results = Vec::new();
        for (i, agg) in aggs.iter().enumerate() {
            let res = match agg {
                crate::aggregate::AggExpr::Count { .. } => {
                    let count_val: i64 = row.try_get(i)?;
                    crate::aggregate::AggResult::I64(count_val)
                }
                crate::aggregate::AggExpr::Sum { .. }
                | crate::aggregate::AggExpr::Avg { .. }
                | crate::aggregate::AggExpr::Min { .. }
                | crate::aggregate::AggExpr::Max { .. } => {
                    let val: Option<f64> = row.try_get(i)?;
                    match val {
                        Some(v) => crate::aggregate::AggResult::F64(v),
                        None => crate::aggregate::AggResult::Null,
                    }
                }
            };
            results.push(res);
        }

        Ok(results)
    }
}

fn split_field_lookup(s: &'static str) -> (&'static str, &'static str) {
    if let Some(idx) = s.rfind("__") {
        let field = &s[..idx];
        let suffix = &s[idx + 2..];
        match suffix {
            "eq" | "lt" | "lte" | "gt" | "gte" | "contains" | "icontains" | "startswith"
            | "endswith" => (field, suffix),
            _ => (s, "eq"),
        }
    } else {
        (s, "eq")
    }
}

fn suffix_to_op(suffix: &str) -> CompareOp {
    match suffix {
        "eq" => CompareOp::Eq,
        "lt" => CompareOp::Lt,
        "lte" => CompareOp::Lte,
        "gt" => CompareOp::Gt,
        "gte" => CompareOp::Gte,
        "contains" => CompareOp::Contains,
        "icontains" => CompareOp::IContains,
        "startswith" => CompareOp::StartsWith,
        "endswith" => CompareOp::EndsWith,
        _ => CompareOp::Eq,
    }
}

fn compile_expr_sql(
    expr: &Expr,
    field_to_col: &dyn Fn(&str) -> String,
    params: &mut Vec<Value>,
    param_idx: &mut usize,
) -> String {
    match expr {
        Expr::Compare { field, op, value } => {
            let col = field_to_col(field);
            let (op_sql, bind_val) = match op {
                CompareOp::Eq => ("=", value.clone()),
                CompareOp::Lt => ("<", value.clone()),
                CompareOp::Lte => ("<=", value.clone()),
                CompareOp::Gt => (">", value.clone()),
                CompareOp::Gte => (">=", value.clone()),
                CompareOp::Contains => {
                    let s = match value {
                        Value::Text(t) => format!("%{}%", t),
                        other => format!("%{:?}%", other),
                    };
                    ("LIKE", Value::Text(s))
                }
                CompareOp::IContains => {
                    let s = match value {
                        Value::Text(t) => format!("%{}%", t),
                        other => format!("%{:?}%", other),
                    };
                    ("ILIKE", Value::Text(s))
                }
                CompareOp::StartsWith => {
                    let s = match value {
                        Value::Text(t) => format!("{}%", t),
                        other => format!("{:?}%", other),
                    };
                    ("LIKE", Value::Text(s))
                }
                CompareOp::EndsWith => {
                    let s = match value {
                        Value::Text(t) => format!("%{}", t),
                        other => format!("%{:?}", other),
                    };
                    ("LIKE", Value::Text(s))
                }
            };
            params.push(bind_val);
            let placeholder = format!("${}", param_idx);
            *param_idx += 1;
            format!("{} {} {}", col, op_sql, placeholder)
        }
        Expr::And(exprs) => {
            if exprs.is_empty() {
                "TRUE".to_string()
            } else {
                let parts: Vec<String> = exprs
                    .iter()
                    .map(|e| format!("({})", compile_expr_sql(e, field_to_col, params, param_idx)))
                    .collect();
                parts.join(" AND ")
            }
        }
        Expr::Or(exprs) => {
            if exprs.is_empty() {
                "FALSE".to_string()
            } else {
                let parts: Vec<String> = exprs
                    .iter()
                    .map(|e| format!("({})", compile_expr_sql(e, field_to_col, params, param_idx)))
                    .collect();
                parts.join(" OR ")
            }
        }
        Expr::Not(inner) => {
            format!(
                "NOT ({})",
                compile_expr_sql(inner, field_to_col, params, param_idx)
            )
        }
    }
}
