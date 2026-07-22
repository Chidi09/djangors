use async_trait::async_trait;
use djangors_auth::{Auth, User};
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_orm::meta::{Model, ModelMeta};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

pub(crate) const CHANGELIST_PER_PAGE: i64 = 100;

static ADMIN_TEMPLATES: std::sync::LazyLock<djangors_template::TemplateEngine> =
    std::sync::LazyLock::new(|| {
        djangors_template::TemplateEngine::from_embedded(&[
            (
                "admin/index.html",
                include_str!("../templates/admin/index.html"),
            ),
            (
                "admin/delete_confirm.html",
                include_str!("../templates/admin/delete_confirm.html"),
            ),
            (
                "admin/bulk_action_confirm.html",
                include_str!("../templates/admin/bulk_action_confirm.html"),
            ),
            (
                "admin/bulk_delete_confirm.html",
                include_str!("../templates/admin/bulk_delete_confirm.html"),
            ),
            (
                "admin/save_changelist_error.html",
                include_str!("../templates/admin/save_changelist_error.html"),
            ),
            (
                "admin/render_form.html",
                include_str!("../templates/admin/render_form.html"),
            ),
            (
                "admin/changelist.html",
                include_str!("../templates/admin/changelist.html"),
            ),
            (
                "admin/object_history.html",
                include_str!("../templates/admin/object_history.html"),
            ),
            (
                "admin/base.html",
                include_str!("../templates/admin/base.html"),
            ),
        ])
        .expect("admin templates are compiled into the binary and must always be valid")
    });

/// Percent-encodes a value for embedding in a `href="?...&key=<value>"` query
/// string. HTML-escaping alone is not enough here: `&` in a raw search term
/// would be rendered as `&amp;`, which a browser decodes right back to a
/// literal `&` before parsing the URL, letting the value inject extra query
/// parameters or truncate the link (via `#`) instead of round-tripping as a
/// single opaque value.
fn url_encode_query_value(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Builds a `?key=value&key=value...` query string from already-string-ish
/// parts, percent-encoding every value. Skips pairs whose value is `None`.
fn build_query_string(pairs: &[(&str, Option<&str>)]) -> String {
    let parts: Vec<String> = pairs
        .iter()
        .filter_map(|(k, v)| v.map(|val| format!("{}={}", k, url_encode_query_value(val))))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

pub struct ChangelistPage {
    pub columns: Vec<&'static str>, // field names, declaration order
    pub rows: Vec<Vec<String>>,     // Display-rendered, NOT escaped (view escapes)
    pub pks: Vec<String>,
    pub total: i64, // COUNT(*) over the whole table
    pub page: i64,  // 1-based current page
    pub per_page: i64,
}

#[derive(Default, Clone)]
pub struct ModelAdminConfig {
    /// Subset/reorder of real field names to show as changelist columns.
    /// `None` = all fields, declaration order (today's only behavior).
    pub list_display: Option<&'static [&'static str]>,
    /// Real, text-like field names to ILIKE-match against `?q=`.
    /// `None` or empty = no search box rendered, `?q=` ignored if present.
    pub search_fields: Option<&'static [&'static str]>,
    /// Boolean field names only.
    pub list_filter: Option<&'static [&'static str]>,
    pub date_hierarchy: Option<&'static str>,
    pub list_editable: Option<&'static [&'static str]>,
    // Computed columns for display
    #[allow(clippy::type_complexity)]
    pub computed_columns: Option<
        &'static [(
            &'static str,
            fn(&[(&'static str, djangors_orm::expr::Value)]) -> String,
        )],
    >,
    pub actions: Option<&'static [AdminAction]>,
    pub fieldsets: Option<&'static [(&'static str, &'static [&'static str])]>,
    pub readonly_fields: Option<&'static [&'static str]>,
    pub raw_id_fields: Option<&'static [&'static str]>,
    pub base_filter: Option<djangors_orm::UnresolvedExpr>,
}

#[async_trait]
pub trait ModelAdmin: Send + Sync {
    fn model_meta(&self) -> &'static ModelMeta;
    fn field_names(&self) -> Vec<&'static str>;
    #[allow(clippy::too_many_arguments)]
    async fn changelist(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>, // raw ?o= value, e.g. "name" or "-name"
        page: i64,           // already-validated >= 1
        per_page: i64,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    ) -> Result<ChangelistPage, DjangorsError>;

    #[allow(clippy::too_many_arguments)]
    async fn export_csv_rows(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    ) -> Result<(Vec<&'static str>, Vec<Vec<String>>), DjangorsError>;

    fn search_fields(&self) -> &[&'static str];
    fn list_filter_fields(&self) -> &[&'static str];
    fn date_hierarchy_field(&self) -> Option<&'static str>;
    fn list_editable_fields(&self) -> &[&'static str];
    fn actions(&self) -> Vec<AdminAction> {
        Vec::new()
    }
    fn fieldsets(&self) -> Option<&'static [(&'static str, &'static [&'static str])]> {
        None
    }
    fn readonly_fields(&self) -> &[&'static str] {
        &[]
    }
    fn raw_id_fields(&self) -> &[&'static str] {
        &[]
    }

    async fn get_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
    ) -> Result<Option<Vec<(&'static str, djangors_orm::expr::Value)>>, DjangorsError>;

    async fn update_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError>;

    async fn update_fields_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError>;

    async fn create_from_form(
        &self,
        db: &djangors_db::Database,
        form: &std::collections::HashMap<String, String>,
    ) -> Result<Result<i64, std::collections::HashMap<String, String>>, DjangorsError>;

    async fn delete_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
    ) -> Result<bool, DjangorsError>;
}

/// Blanket impl so any real Model can be registered with zero boilerplate.
pub struct DefaultModelAdmin<M: Model> {
    config: ModelAdminConfig,
    _marker: PhantomData<M>,
}

impl<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static> DefaultModelAdmin<M> {
    fn effective_columns(&self) -> Vec<&'static str> {
        self.config
            .list_display
            .map(|cols| cols.to_vec())
            .unwrap_or_else(M::field_names)
    }

    fn build_filtered_queryset(
        &self,
        order: Option<&str>,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    ) -> Result<djangors_orm::queryset::QuerySet<M>, DjangorsError> {
        let mut qs = M::objects();
        if let Some(o) = order {
            qs = qs.order_by(o).map_err(|e| match e {
                djangors_orm::error::OrmError::FieldNotFound { .. } => {
                    DjangorsError::BadRequest(e.to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;
        }
        if let (Some(term), Some(fields)) = (search, self.config.search_fields) {
            if !term.is_empty() {
                qs = qs
                    .filter_or_icontains(fields, term)
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            }
        }
        if !filters.is_empty() {
            let compares = filters
                .iter()
                .map(|(field, val)| djangors_orm::expr::UnresolvedCompare {
                    field,
                    value: djangors_orm::expr::Value::Bool(*val),
                })
                .collect();
            qs = qs
                .filter(djangors_orm::expr::UnresolvedExpr::And(compares))
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        }
        if let (Some(field), Some((gte, lt))) = (self.config.date_hierarchy, date_range) {
            qs = qs
                .filter_datetime_range(field, gte, lt)
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        }
        if let Some(bf) = &self.config.base_filter {
            qs = qs
                .filter(bf.clone())
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        }
        Ok(qs)
    }

    fn row_values(&self, item: &M, columns: &[&'static str]) -> Vec<String> {
        let field_values = item.field_values();
        columns
            .iter()
            .map(|col| {
                // Check computed columns first
                if let Some(comp_cols) = self.config.computed_columns {
                    if let Some((_, f)) = comp_cols.iter().find(|(n, _)| *n == *col) {
                        return f(&field_values);
                    }
                }
                // Fallback to normal field values
                field_values
                    .iter()
                    .find(|(name, _)| name == col)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default()
            })
            .collect()
    }
}

#[async_trait]
impl<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static> ModelAdmin
    for DefaultModelAdmin<M>
{
    fn model_meta(&self) -> &'static ModelMeta {
        M::meta()
    }

    fn field_names(&self) -> Vec<&'static str> {
        M::field_names()
    }

    fn search_fields(&self) -> &[&'static str] {
        self.config.search_fields.unwrap_or(&[])
    }

    fn list_filter_fields(&self) -> &[&'static str] {
        self.config.list_filter.unwrap_or(&[])
    }

    fn date_hierarchy_field(&self) -> Option<&'static str> {
        self.config.date_hierarchy
    }

    fn list_editable_fields(&self) -> &[&'static str] {
        self.config.list_editable.unwrap_or(&[])
    }

    fn actions(&self) -> Vec<AdminAction> {
        let mut list = self.config.actions.map(|a| a.to_vec()).unwrap_or_default();
        list.push(AdminAction {
            name: "delete_selected",
            label: "Delete selected",
            requires_confirm: true,
            handler: |db: &djangors_db::Database, pks: &[i64]| {
                Box::pin(async move {
                    for &pk in pks {
                        let _ = djangors_orm::queryset::QuerySet::<M>::delete_by_pk(db, pk).await;
                    }
                    Ok(())
                })
            },
        });
        list
    }

    fn fieldsets(&self) -> Option<&'static [(&'static str, &'static [&'static str])]> {
        self.config.fieldsets
    }

    fn readonly_fields(&self) -> &[&'static str] {
        self.config.readonly_fields.unwrap_or(&[])
    }

    fn raw_id_fields(&self) -> &[&'static str] {
        self.config.raw_id_fields.unwrap_or(&[])
    }

    #[allow(clippy::too_many_arguments)]
    async fn changelist(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>,
        page: i64,
        per_page: i64,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    ) -> Result<ChangelistPage, DjangorsError> {
        let mut qs = self.build_filtered_queryset(order, search, filters, date_range)?;

        let total = qs
            .clone()
            .count(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let offset = (page - 1) * per_page;
        qs = qs.limit(per_page).offset(offset);
        let items = qs
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let columns = self.effective_columns();

        let meta = M::meta();
        let pk_field_name = meta
            .fields
            .iter()
            .find(|f| f.primary_key)
            .map(|f| f.name)
            .unwrap_or("id");

        let mut rows = Vec::new();
        let mut pks = Vec::new();

        for item in &items {
            let row_vals = self.row_values(item, &columns);
            rows.push(row_vals);

            let field_values = item.field_values();
            let pk_val = field_values
                .iter()
                .find(|(name, _)| name == &pk_field_name)
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            pks.push(pk_val);
        }

        Ok(ChangelistPage {
            columns,
            rows,
            pks,
            total,
            page,
            per_page,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn export_csv_rows(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    ) -> Result<(Vec<&'static str>, Vec<Vec<String>>), DjangorsError> {
        let qs = self.build_filtered_queryset(order, search, filters, date_range)?;
        let items = qs
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        let columns = self.effective_columns();
        let rows = items
            .iter()
            .map(|item| self.row_values(item, &columns))
            .collect();
        Ok((columns, rows))
    }

    async fn get_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
    ) -> Result<Option<Vec<(&'static str, djangors_orm::expr::Value)>>, DjangorsError> {
        let meta = M::meta();
        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;

        let unresolved_cmp =
            djangors_orm::expr::UnresolvedExpr::And(vec![djangors_orm::expr::UnresolvedCompare {
                field: pk_field.name,
                value: djangors_orm::expr::Value::I64(pk),
            }]);

        let row_opt = M::objects()
            .filter(unresolved_cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await;

        match row_opt {
            Ok(row) => Ok(Some(row.field_values())),
            Err(djangors_orm::error::OrmError::NotFound { .. }) => Ok(None),
            Err(e) => Err(DjangorsError::Internal(e.to_string())),
        }
    }

    async fn update_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError> {
        let meta = M::meta();
        let mut errors = std::collections::HashMap::new();
        let mut sets = Vec::new();

        for field in meta.fields {
            if field.auto {
                continue;
            }
            let raw_opt = form.get(field.name).map(|s| s.as_str());
            match parse_field_value(field, raw_opt) {
                Ok(val) => {
                    sets.push((field.name, djangors_orm::expr::SetExpr::Literal(val)));
                }
                Err(err_msg) => {
                    errors.insert(field.name.to_string(), err_msg);
                }
            }
        }

        for rel in meta.relations {
            let raw_opt = form.get(rel.field_name).map(|s| s.as_str());
            match parse_relation_value(rel, raw_opt) {
                Ok(val) => {
                    sets.push((rel.field_name, djangors_orm::expr::SetExpr::Literal(val)));
                }
                Err(err_msg) => {
                    errors.insert(rel.field_name.to_string(), err_msg);
                }
            }
        }

        if !errors.is_empty() {
            return Ok(Err(errors));
        }

        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;

        let unresolved_cmp =
            djangors_orm::expr::UnresolvedExpr::And(vec![djangors_orm::expr::UnresolvedCompare {
                field: pk_field.name,
                value: djangors_orm::expr::Value::I64(pk),
            }]);

        let qs = M::objects()
            .filter(unresolved_cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        qs.update(db, sets)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Ok(Ok(()))
    }

    async fn update_fields_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError> {
        let meta = M::meta();
        let mut errors = std::collections::HashMap::new();
        let mut sets = Vec::new();

        for field in meta.fields {
            if field.auto {
                continue;
            }
            let Some(raw) = form.get(field.name) else {
                continue; // not part of this edit - leave the column untouched
            };
            match parse_field_value(field, Some(raw.as_str())) {
                Ok(val) => sets.push((field.name, djangors_orm::expr::SetExpr::Literal(val))),
                Err(err_msg) => {
                    errors.insert(field.name.to_string(), err_msg);
                }
            }
        }

        if !errors.is_empty() {
            return Ok(Err(errors));
        }
        if sets.is_empty() {
            return Ok(Ok(())); // nothing in `form` matched a real field - a no-op, not an error
        }

        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;
        let unresolved_cmp =
            djangors_orm::expr::UnresolvedExpr::And(vec![djangors_orm::expr::UnresolvedCompare {
                field: pk_field.name,
                value: djangors_orm::expr::Value::I64(pk),
            }]);
        let qs = M::objects()
            .filter(unresolved_cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        qs.update(db, sets)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Ok(Ok(()))
    }

    async fn create_from_form(
        &self,
        db: &djangors_db::Database,
        form: &std::collections::HashMap<String, String>,
    ) -> Result<Result<i64, std::collections::HashMap<String, String>>, DjangorsError> {
        let meta = M::meta();
        let mut errors = std::collections::HashMap::new();
        let mut values = Vec::new();

        for field in meta.fields {
            if field.auto || field.primary_key {
                continue;
            }
            let raw_opt = form.get(field.name).map(|s| s.as_str());
            match parse_field_value(field, raw_opt) {
                Ok(val) => {
                    values.push((field.name, val));
                }
                Err(err_msg) => {
                    errors.insert(field.name.to_string(), err_msg);
                }
            }
        }

        for rel in meta.relations {
            let raw_opt = form.get(rel.field_name).map(|s| s.as_str());
            match parse_relation_value(rel, raw_opt) {
                Ok(val) => {
                    values.push((rel.field_name, val));
                }
                Err(err_msg) => {
                    errors.insert(rel.field_name.to_string(), err_msg);
                }
            }
        }

        if !errors.is_empty() {
            return Ok(Err(errors));
        }

        let pk = djangors_orm::queryset::QuerySet::<M>::insert_raw(db, values)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Ok(Ok(pk))
    }

    async fn delete_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
    ) -> Result<bool, DjangorsError> {
        let rows = djangors_orm::queryset::QuerySet::<M>::delete_by_pk(db, pk)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        Ok(rows > 0)
    }
}

fn parse_field_value(
    field: &djangors_orm::meta::FieldMeta,
    raw: Option<&str>,
) -> Result<djangors_orm::expr::Value, String> {
    use djangors_orm::expr::Value;
    use djangors_orm::meta::FieldKind;

    if field.kind == FieldKind::Boolean {
        match raw {
            Some(val) if !val.is_empty() => return Ok(Value::Bool(true)),
            _ => {
                if field.nullable {
                    return Ok(Value::Null);
                } else {
                    return Ok(Value::Bool(false));
                }
            }
        }
    }

    let raw = raw.unwrap_or("");
    if raw.is_empty() {
        if field.nullable {
            return Ok(Value::Null);
        } else {
            return Err(format!("Field '{}' is required.", field.name));
        }
    }

    match field.kind {
        FieldKind::Char
        | FieldKind::Text
        | FieldKind::Email
        | FieldKind::Url
        | FieldKind::Slug
        | FieldKind::Ip
        | FieldKind::Binary
        | FieldKind::Json => Ok(Value::Text(raw.to_string())),
        FieldKind::Integer | FieldKind::BigInt => raw
            .parse::<i64>()
            .map(Value::I64)
            .map_err(|_| format!("Field '{}' must be a valid integer.", field.name)),
        FieldKind::Float => raw
            .parse::<f64>()
            .map(Value::F64)
            .map_err(|_| format!("Field '{}' must be a valid float.", field.name)),
        FieldKind::Boolean => unreachable!(),
        FieldKind::DateTime => {
            let naive =
                chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S").map_err(|_| {
                    format!(
                        "Field '{}' must be in YYYY-MM-DD HH:MM:SS format.",
                        field.name
                    )
                })?;
            let dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc);
            Ok(Value::DateTime(dt))
        }
        FieldKind::Decimal { .. }
        | FieldKind::Date
        | FieldKind::Time
        | FieldKind::Duration
        | FieldKind::Uuid => {
            unreachable!(
                "Unsupported FieldKind {:?} for field '{}' in djangors-admin.",
                field.kind, field.name
            );
        }
    }
}

fn parse_relation_value(
    relation: &djangors_orm::meta::RelationMeta,
    raw: Option<&str>,
) -> Result<djangors_orm::expr::Value, String> {
    use djangors_orm::expr::Value;
    let raw = raw.unwrap_or("");
    if raw.is_empty() {
        return Ok(Value::Null);
    }
    raw.parse::<i64>().map(Value::I64).map_err(|_| {
        format!(
            "Field '{}' must be a valid integer ID.",
            relation.field_name
        )
    })
}

#[derive(serde::Serialize)]
struct FormFieldRow {
    kind: &'static str, // "readonly" | "checkbox" | "number" | "text"
    name: String,
    value: String, // readonly span text, or the input's value="" attribute; unused for checkbox
    checked: bool, // only meaningful when kind == "checkbox"
    error: Option<String>,
    section: Option<&'static str>,
    lookup_href: Option<minijinja::value::Value>,
}

use djangors_macros::Model as DeriveModel;

#[derive(Clone)]
pub struct AdminAction {
    pub name: &'static str,
    pub label: &'static str,
    pub requires_confirm: bool,
    #[allow(clippy::type_complexity)]
    pub handler: for<'a> fn(
        &'a djangors_db::Database,
        &'a [i64],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), DjangorsError>> + Send + 'a>,
    >,
}

#[derive(serde::Serialize)]
pub struct AdminActionRow {
    pub name: String,
    pub label: String,
}

pub const ACTION_ADDITION: i32 = 1;
pub const ACTION_CHANGE: i32 = 2;
pub const ACTION_DELETION: i32 = 3;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldDiffItem {
    pub field: String,
    pub old: String,
    pub new: String,
}

#[derive(DeriveModel, Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[djangors(app = "admin", table_name = "djangors_admin_log")]
pub struct LogEntry {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub user_id: i64,
    pub action_time: chrono::DateTime<chrono::Utc>,
    pub app_label: String,
    pub model_name: String,
    pub object_id: i64,
    pub object_repr: String,
    pub action_flag: i32,
    pub change_message: String,
    pub field_diff: Option<String>,
}

async fn log_action(
    db: &djangors_db::Database,
    user_id: i64,
    meta: &'static ModelMeta,
    object_id: i64,
    action_flag: i32,
    change_message: &str,
    field_diff: Option<String>,
) -> Result<(), DjangorsError> {
    let res = LogEntry {
        id: 0,
        user_id,
        action_time: chrono::Utc::now(),
        app_label: meta.app_label.to_string(),
        model_name: meta.struct_name.to_string(),
        object_id,
        object_repr: format!("{} object ({})", meta.struct_name, object_id),
        action_flag,
        change_message: change_message.to_string(),
        field_diff,
    }
    .save(db)
    .await;

    match res {
        Ok(_) => Ok(()),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("relation \"djangors_admin_log\" does not exist") {
                return Ok(());
            }
            Err(DjangorsError::Internal(err_str))
        }
    }
}

#[derive(Clone)]
pub struct SiteBranding {
    pub site_header: String,
    pub site_title: String,
    pub logo_url: Option<String>,
    pub accent_color: Option<String>,
}

impl Default for SiteBranding {
    fn default() -> Self {
        Self {
            site_header: "Djangors Administration".to_string(),
            site_title: "Djangors site admin".to_string(),
            logo_url: None,
            accent_color: None,
        }
    }
}

#[derive(serde::Serialize)]
struct RenderFormContext {
    rows: Vec<FormFieldRow>,
    site_header: String,
    site_title: String,
    csrf_token: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
    has_change_permission: bool,
    has_delete_permission: bool,
    is_add: bool,
}

#[allow(clippy::too_many_arguments)]
fn render_form(
    meta: &'static ModelMeta,
    field_names: &[&'static str],
    submitted_values: &std::collections::HashMap<String, String>,
    errors: &std::collections::HashMap<String, String>,
    is_add: bool,
    branding: &SiteBranding,
    csrf_token: String,
    has_change_permission: bool,
    has_delete_permission: bool,
    fieldsets: Option<&'static [(&'static str, &'static [&'static str])]>,
    readonly_fields: &[&'static str],
    raw_id_fields: &[&'static str],
) -> Result<Response, DjangorsError> {
    let build_row = |name: &'static str, section: Option<&'static str>| -> Option<FormFieldRow> {
        if let Some(field) = meta.fields.iter().find(|f| f.name == name) {
            if field.auto || field.primary_key {
                if is_add {
                    return None;
                } else {
                    let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
                    return Some(FormFieldRow {
                        kind: "readonly",
                        name: name.to_string(),
                        value: val.to_string(),
                        checked: false,
                        error: None,
                        section,
                        lookup_href: None,
                    });
                }
            }

            let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
            let err = errors.get(name).cloned();

            let is_readonly = readonly_fields.contains(&name);
            let kind = if is_readonly {
                "readonly"
            } else {
                match field.kind {
                    djangors_orm::meta::FieldKind::Boolean => "checkbox",
                    djangors_orm::meta::FieldKind::Integer
                    | djangors_orm::meta::FieldKind::BigInt
                    | djangors_orm::meta::FieldKind::Float => "number",
                    _ => "text",
                }
            };

            let checked = kind == "checkbox" && (val == "on" || val == "true");

            Some(FormFieldRow {
                kind,
                name: name.to_string(),
                value: val.to_string(),
                checked,
                error: err,
                section,
                lookup_href: None,
            })
        } else if let Some(rel) = meta.relations.iter().find(|r| r.field_name == name) {
            let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
            let err = errors.get(name).cloned();

            let is_readonly = readonly_fields.contains(&name);
            let kind = if is_readonly { "readonly" } else { "number" };

            let lookup_href = if raw_id_fields.contains(&name) {
                let target_meta = (rel.target)();
                Some(minijinja::value::Value::from_safe_string(format!(
                    "/{}/{}/",
                    target_meta.app_label,
                    target_meta.struct_name.to_lowercase()
                )))
            } else {
                None
            };

            Some(FormFieldRow {
                kind,
                name: name.to_string(),
                value: val.to_string(),
                checked: false,
                error: err,
                section,
                lookup_href,
            })
        } else {
            None
        }
    };

    let mut rows = Vec::new();

    if let Some(fsets) = fieldsets {
        for &(section_title, names) in fsets {
            for &name in names {
                if let Some(row) = build_row(name, Some(section_title)) {
                    rows.push(row);
                }
            }
        }
        // Also add any field names not in any fieldset
        let all_fieldset_names: std::collections::HashSet<&'static str> = fsets
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .collect();
        for &name in field_names {
            if !all_fieldset_names.contains(name) {
                if let Some(row) = build_row(name, None) {
                    rows.push(row);
                }
            }
        }
    } else {
        for &name in field_names {
            if let Some(row) = build_row(name, None) {
                rows.push(row);
            }
        }
    }

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/render_form.html",
        RenderFormContext {
            rows,
            site_header: branding.site_header.clone(),
            site_title: branding.site_title.clone(),
            csrf_token,
            logo_url: branding.logo_url.clone(),
            accent_color: branding.accent_color.clone(),
            has_change_permission,
            has_delete_permission,
            is_add,
        },
    )
}

pub struct AdminSite {
    registry: Mutex<Vec<Arc<dyn ModelAdmin>>>,
    branding: SiteBranding,
    extra_routes: Vec<std::panic::AssertUnwindSafe<Router>>,
}

impl Default for AdminSite {
    fn default() -> Self {
        Self::new()
    }
}

impl AdminSite {
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(Vec::new()),
            branding: SiteBranding::default(),
            extra_routes: Vec::new(),
        }
    }

    pub fn with_site_header(mut self, header: impl Into<String>) -> Self {
        self.branding.site_header = header.into();
        self
    }

    pub fn with_site_title(mut self, title: impl Into<String>) -> Self {
        self.branding.site_title = title.into();
        self
    }

    pub fn with_logo_url(mut self, url: impl Into<String>) -> Self {
        self.branding.logo_url = Some(url.into());
        self
    }

    pub fn with_accent_color(mut self, color: impl Into<String>) -> Self {
        self.branding.accent_color = Some(color.into());
        self
    }

    /// Register an extra route directly on the admin site.
    pub fn extra_route(mut self, router: Router) -> Self {
        self.extra_routes.push(std::panic::AssertUnwindSafe(router));
        self
    }

    /// Register a model with the default (no customization) ModelAdmin.
    pub fn register<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static>(&self) {
        self.register_with::<M>(ModelAdminConfig::default());
    }

    pub fn register_with<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static>(
        &self,
        config: ModelAdminConfig,
    ) {
        let meta = M::meta();
        if let Some(list_display) = config.list_display {
            for name in list_display {
                assert!(
                    meta.fields.iter().any(|f| f.name == *name)
                        || meta.relations.iter().any(|r| r.field_name == *name)
                        || config
                            .computed_columns
                            .is_some_and(|cols| cols.iter().any(|(n, _)| *n == *name)),
                    "list_display field '{}' does not exist on model '{}'",
                    name,
                    meta.struct_name
                );
            }
        }
        if let Some(search_fields) = config.search_fields {
            for name in search_fields {
                let field = meta
                    .fields
                    .iter()
                    .find(|f| f.name == *name)
                    .unwrap_or_else(|| {
                        panic!(
                            "search_fields field '{}' does not exist on model '{}'",
                            name, meta.struct_name
                        )
                    });
                assert!(
                    matches!(
                        field.kind,
                        djangors_orm::meta::FieldKind::Char
                            | djangors_orm::meta::FieldKind::Text
                            | djangors_orm::meta::FieldKind::Email
                            | djangors_orm::meta::FieldKind::Url
                            | djangors_orm::meta::FieldKind::Slug
                            | djangors_orm::meta::FieldKind::Ip
                    ),
                    "search_fields field '{}' on model '{}' is not a text-like field",
                    name,
                    meta.struct_name
                );
            }
        }
        if let Some(list_filter) = config.list_filter {
            for name in list_filter {
                let field = meta
                    .fields
                    .iter()
                    .find(|f| f.name == *name)
                    .unwrap_or_else(|| {
                        panic!(
                            "list_filter field '{}' does not exist on model '{}'",
                            name, meta.struct_name
                        )
                    });
                assert!(
                    matches!(field.kind, djangors_orm::meta::FieldKind::Boolean),
                    "list_filter field '{}' on model '{}' is not a Boolean field (choices-based \
                     list_filter is not supported — this ORM has no choices metadata yet)",
                    name,
                    meta.struct_name
                );
            }
        }
        if let Some(field_name) = config.date_hierarchy {
            let field = meta
                .fields
                .iter()
                .find(|f| f.name == field_name)
                .unwrap_or_else(|| {
                    panic!(
                        "date_hierarchy field '{}' does not exist on model '{}'",
                        field_name, meta.struct_name
                    )
                });
            assert!(
                field.kind == djangors_orm::meta::FieldKind::DateTime,
                "date_hierarchy field '{}' on model '{}' is not a DateTime field",
                field_name,
                meta.struct_name
            );
        }

        if let Some(list_editable) = config.list_editable {
            let effective_columns: Vec<&'static str> = config
                .list_display
                .map(|c| c.to_vec())
                .unwrap_or_else(M::field_names);
            for name in list_editable {
                let field = meta
                    .fields
                    .iter()
                    .find(|f| f.name == *name)
                    .unwrap_or_else(|| {
                        panic!(
                            "list_editable field '{}' does not exist on model '{}'",
                            name, meta.struct_name
                        )
                    });
                assert!(
                    matches!(
                        field.kind,
                        djangors_orm::meta::FieldKind::Char
                            | djangors_orm::meta::FieldKind::Text
                            | djangors_orm::meta::FieldKind::Email
                            | djangors_orm::meta::FieldKind::Url
                            | djangors_orm::meta::FieldKind::Slug
                            | djangors_orm::meta::FieldKind::Integer
                            | djangors_orm::meta::FieldKind::BigInt
                            | djangors_orm::meta::FieldKind::Float
                    ),
                    "list_editable field '{}' on model '{}' is not a supported type \
                     (Boolean and other kinds are not supported in v1 — see design doc)",
                    name,
                    meta.struct_name
                );
                let col_index = effective_columns.iter().position(|c| c == name);
                assert!(
                    col_index.is_some(),
                    "list_editable field '{}' on model '{}' must also be in list_display",
                    name,
                    meta.struct_name
                );
                assert!(
                    col_index != Some(0),
                    "list_editable field '{}' on model '{}' cannot be the first list_display \
                     column (that column is always the row's edit link)",
                    name,
                    meta.struct_name
                );
            }
        }

        let mut reg = self.registry.lock().unwrap();
        reg.push(Arc::new(DefaultModelAdmin::<M> {
            config,
            _marker: PhantomData,
        }));
    }

    /// Build a Router with the index, changelist, and add/change routes.
    ///
    /// Every admin registered here is captured directly in the handler
    /// closures, not via `Router::with_state` — `Router::mount` (the real,
    /// documented way a caller composes `site.urls()` into a larger app
    /// router) only copies routes, never a sub-router's own state, so any
    /// state attached here would silently never reach the handlers once
    /// mounted. Capturing it in the closures sidesteps that entirely: it's
    /// baked into each handler itself, independent of whatever router it
    /// ends up mounted under. `Auth::<User>::from_request` inside the
    /// handlers still relies on `Request::state::<Database>()`, which *is*
    /// expected to come from the caller's own top-level `.with_state(db)`
    /// call (the same state that already correctly reaches every route,
    /// mounted or not, since `Router::dispatch` always attaches whichever
    /// router's `dispatch` is actually running its own `self.state` — see
    /// `Router::dispatch`).
    pub fn urls(&self) -> Router {
        let reg = self.registry.lock().unwrap();
        let admins: Vec<Arc<dyn ModelAdmin>> = reg.iter().cloned().collect();
        let snapshot: Vec<&'static ModelMeta> =
            admins.iter().map(|item| item.model_meta()).collect();
        let branding = self.branding.clone();

        let index_admins = snapshot.clone();
        let changelist_admins = admins.clone();
        let add_get_admins = admins.clone();
        let add_post_admins = admins.clone();
        let change_get_admins = admins.clone();
        let change_post_admins = admins.clone();
        let delete_get_admins = admins.clone();
        let delete_post_admins = admins.clone();
        let bulk_delete_admins = admins.clone();
        let bulk_action_admins = admins.clone();
        let save_changelist_admins = admins.clone();
        let export_csv_admins = admins.clone();

        let index_branding = branding.clone();
        let changelist_branding = branding.clone();
        let add_get_branding = branding.clone();
        let add_post_branding = branding.clone();
        let change_get_branding = branding.clone();
        let change_post_branding = branding.clone();
        let delete_get_branding = branding.clone();
        let bulk_delete_branding = branding.clone();
        let save_changelist_branding = branding.clone();
        let bulk_action_branding = branding.clone();
        let history_branding = branding.clone();

        let mut router = Router::new()
            .get("/", move |req: Request, params: PathParams| {
                admin_index(req, params, index_admins.clone(), index_branding.clone())
            })
            .get(
                "/{app:slug}/{model:slug}/",
                move |req: Request, params: PathParams| {
                    admin_changelist(
                        req,
                        params,
                        changelist_admins.clone(),
                        changelist_branding.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/export-csv/",
                move |req: Request, params: PathParams| {
                    admin_export_csv(req, params, export_csv_admins.clone())
                },
            )
            .get(
                "/{app:slug}/{model:slug}/add/",
                move |req: Request, params: PathParams| {
                    admin_add_get(
                        req,
                        params,
                        add_get_admins.clone(),
                        add_get_branding.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/add/",
                move |req: Request, params: PathParams| {
                    admin_add_post(
                        req,
                        params,
                        add_post_admins.clone(),
                        add_post_branding.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/{pk:i64}/change/",
                move |req: Request, params: PathParams| {
                    admin_change_get(
                        req,
                        params,
                        change_get_admins.clone(),
                        change_get_branding.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/{pk:i64}/change/",
                move |req: Request, params: PathParams| {
                    admin_change_post(
                        req,
                        params,
                        change_post_admins.clone(),
                        change_post_branding.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/{pk:i64}/delete/",
                move |req: Request, params: PathParams| {
                    admin_delete_get(
                        req,
                        params,
                        delete_get_admins.clone(),
                        delete_get_branding.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/{pk:i64}/delete/",
                move |req: Request, params: PathParams| {
                    admin_delete_post(req, params, delete_post_admins.clone())
                },
            )
            .get(
                "/{app:slug}/{model:slug}/{pk:i64}/history/",
                move |req: Request, params: PathParams| {
                    admin_history(req, params, admins.clone(), history_branding.clone())
                },
            )
            .post(
                "/{app:slug}/{model:slug}/bulk-delete/",
                move |req: Request, params: PathParams| {
                    admin_bulk_delete_post(
                        req,
                        params,
                        bulk_delete_admins.clone(),
                        bulk_delete_branding.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/save-changelist/",
                move |req: Request, params: PathParams| {
                    admin_save_changelist_post(
                        req,
                        params,
                        save_changelist_admins.clone(),
                        save_changelist_branding.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/bulk-action/",
                move |req: Request, params: PathParams| {
                    admin_bulk_action_post(
                        req,
                        params,
                        bulk_action_admins.clone(),
                        bulk_action_branding.clone(),
                    )
                },
            );

        for extra in self.extra_routes.iter() {
            router = router.mount("", extra.0.clone());
        }
        router
    }
}

fn csv_escape_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn rows_to_csv(columns: &[&'static str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push_str(
        &columns
            .iter()
            .map(|c| csv_escape_field(c))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push_str("\r\n");
    for row in rows {
        out.push_str(
            &row.iter()
                .map(|v| csv_escape_field(v))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("\r\n");
    }
    out
}

async fn admin_export_csv(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    require_perm(&req, db, admin.model_meta(), "view").await?;

    let state = parse_changelist_query_state(&req, admin.as_ref())?;
    let (columns, rows) = admin
        .export_csv_rows(
            db,
            state.order,
            state.search,
            &state.filters,
            state.date_range,
        )
        .await?;
    let csv_body = rows_to_csv(&columns, &rows);

    let filename = format!("{}.csv", admin.model_meta().struct_name.to_lowercase());
    Ok(Response::bytes(
        StatusCode::OK,
        "text/csv; charset=utf-8",
        csv_body.into_bytes(),
    )
    .header(
        "Content-Disposition",
        &format!("attachment; filename=\"{}\"", filename),
    ))
}

async fn require_staff(req: &Request) -> Result<User, DjangorsError> {
    let auth = Auth::<User>::from_request(req).await?;
    if !auth.0.is_staff {
        return Err(DjangorsError::Forbidden(
            "staff status required".to_string(),
        ));
    }
    Ok(auth.0)
}

fn action_codename(meta: &'static ModelMeta, action: &str) -> String {
    format!(
        "{}.{}_{}",
        meta.app_label,
        action,
        meta.struct_name.to_lowercase()
    )
}

async fn require_perm(
    req: &Request,
    db: &djangors_db::Database,
    meta: &'static ModelMeta,
    action: &str,
) -> Result<User, DjangorsError> {
    let user = require_staff(req).await?;
    if user.is_superuser {
        return Ok(user);
    }
    let codename = action_codename(meta, action);
    let allowed = djangors_auth::has_perm(db, user.id, &codename)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    if !allowed {
        return Err(DjangorsError::Forbidden(format!(
            "permission '{}' required",
            codename
        )));
    }
    Ok(user)
}

#[derive(serde::Serialize)]
struct IndexModelLink {
    // `from_safe_string` bypasses the template's HTML autoescaping (which
    // escapes `/` as `&#x2f;`, unlike a plain `String` field). Safe here
    // specifically because both values are built purely from `ModelMeta`'s
    // `app_label`/`struct_name` - compile-time `&'static str`s from
    // `#[derive(Model)]`, never user input. Do not reuse this pattern for a
    // field that could ever carry request-derived or database-stored data.
    href: minijinja::value::Value,
    label: minijinja::value::Value,
    has_view_permission: bool,
    has_change_permission: bool,
    has_add_permission: bool,
}

#[derive(serde::Serialize)]
struct RecentActionRow {
    action_label: &'static str,
    object_repr: String,
    app_label: String,
    model_name: String,
    action_time: String,
}

#[derive(serde::Serialize)]
struct IndexContext {
    models: Vec<IndexModelLink>,
    recent_actions: Vec<RecentActionRow>,
    site_header: String,
    site_title: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
}

async fn admin_index(
    req: Request,
    _params: PathParams,
    registry: Vec<&'static ModelMeta>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let user = require_staff(&req).await?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let mut models = Vec::new();
    for meta in &registry {
        let has_view = if user.is_superuser {
            true
        } else {
            let codename = action_codename(meta, "view");
            djangors_auth::has_perm(db, user.id, &codename)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?
        };
        let has_change = if user.is_superuser {
            true
        } else {
            let codename = action_codename(meta, "change");
            djangors_auth::has_perm(db, user.id, &codename)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?
        };
        let has_add = if user.is_superuser {
            true
        } else {
            let codename = action_codename(meta, "add");
            djangors_auth::has_perm(db, user.id, &codename)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?
        };
        if !has_view && !has_change && !has_add {
            continue;
        }
        models.push(IndexModelLink {
            href: minijinja::value::Value::from_safe_string(format!(
                "{}/{}/",
                meta.app_label,
                meta.struct_name.to_lowercase()
            )),
            label: minijinja::value::Value::from_safe_string(format!(
                "{}.{}",
                meta.app_label, meta.struct_name
            )),
            has_view_permission: has_view,
            has_change_permission: has_change,
            has_add_permission: has_add,
        });
    }

    let recent_rows_res: Result<Vec<LogEntry>, _> = sqlx::query_as(sqlx::AssertSqlSafe(
        "SELECT id, user_id, action_time, app_label, model_name, object_id, object_repr, \
         action_flag, change_message, field_diff FROM djangors_admin_log WHERE user_id = $1 \
         ORDER BY action_time DESC LIMIT 10",
    ))
    .bind(user.id)
    .fetch_all(db.pool())
    .await;

    let recent_rows = match recent_rows_res {
        Ok(rows) => rows,
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("relation \"djangors_admin_log\" does not exist") {
                Vec::new()
            } else {
                return Err(DjangorsError::Internal(err_str));
            }
        }
    };

    let recent_actions: Vec<RecentActionRow> = recent_rows
        .into_iter()
        .map(|e| RecentActionRow {
            action_label: match e.action_flag {
                ACTION_ADDITION => "Added",
                ACTION_DELETION => "Deleted",
                _ => "Changed",
            },
            object_repr: e.object_repr,
            app_label: e.app_label,
            model_name: e.model_name,
            action_time: e.action_time.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect();

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/index.html",
        IndexContext {
            models,
            recent_actions,
            site_header: branding.site_header,
            site_title: branding.site_title,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
        },
    )
}

fn get_month_name(m: u32) -> &'static str {
    match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}

struct ChangelistQueryState<'a> {
    order: Option<&'a str>,
    search: Option<&'a str>,
    filters: Vec<(&'static str, bool)>,
    year: Option<i32>,
    month: Option<u32>,
    day: Option<u32>,
    date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
}

fn parse_changelist_query_state<'a>(
    req: &'a Request,
    admin: &dyn ModelAdmin,
) -> Result<ChangelistQueryState<'a>, DjangorsError> {
    let order = req.query("o");
    let search = req.query("q");

    let year: Option<i32> = match req.query("year") {
        Some(s) => Some(
            s.parse()
                .map_err(|_| DjangorsError::BadRequest("invalid year".into()))?,
        ),
        None => None,
    };
    let month: Option<u32> = match req.query("month") {
        Some(s) => Some(
            s.parse()
                .map_err(|_| DjangorsError::BadRequest("invalid month".into()))?,
        ),
        None => None,
    };
    let day: Option<u32> = match req.query("day") {
        Some(s) => Some(
            s.parse()
                .map_err(|_| DjangorsError::BadRequest("invalid day".into()))?,
        ),
        None => None,
    };

    use chrono::{Duration, TimeZone, Utc};

    let date_range: Option<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)> =
        match (admin.date_hierarchy_field(), year) {
            (Some(_), Some(y)) => {
                let (gte, lt) = match month {
                    Some(m) => {
                        let start =
                            Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0)
                                .single()
                                .ok_or_else(|| {
                                    DjangorsError::BadRequest("invalid year/month".into())
                                })?;
                        let end = match day {
                            Some(d) => {
                                let s =
                                    Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).single().ok_or_else(
                                        || DjangorsError::BadRequest("invalid day".into()),
                                    )?;
                                s + Duration::days(1)
                            }
                            None => if m == 12 {
                                Utc.with_ymd_and_hms(y + 1, 1, 1, 0, 0, 0).single()
                            } else {
                                Utc.with_ymd_and_hms(y, m + 1, 1, 0, 0, 0).single()
                            }
                            .ok_or_else(|| {
                                DjangorsError::BadRequest("invalid year/month".into())
                            })?,
                        };
                        let start = match day {
                            Some(d) => Utc
                                .with_ymd_and_hms(y, m, d, 0, 0, 0)
                                .single()
                                .ok_or_else(|| DjangorsError::BadRequest("invalid day".into()))?,
                            None => start,
                        };
                        (start, end)
                    }
                    None => {
                        let start = Utc
                            .with_ymd_and_hms(y, 1, 1, 0, 0, 0)
                            .single()
                            .ok_or_else(|| DjangorsError::BadRequest("invalid year".into()))?;
                        let end = Utc
                            .with_ymd_and_hms(y + 1, 1, 1, 0, 0, 0)
                            .single()
                            .ok_or_else(|| DjangorsError::BadRequest("invalid year".into()))?;
                        (start, end)
                    }
                };
                Some((gte, lt))
            }
            _ => None,
        };

    let list_filter_fields = admin.list_filter_fields();
    let mut active_filters: Vec<(&'static str, bool)> = Vec::new();
    for &field_name in list_filter_fields {
        if let Some(raw) = req.query(field_name) {
            match raw {
                "true" => active_filters.push((field_name, true)),
                "false" => active_filters.push((field_name, false)),
                _ => {} // anything else (including empty/"all") = not filtered on this field
            }
        }
    }

    Ok(ChangelistQueryState {
        order,
        search,
        filters: active_filters,
        year,
        month,
        day,
        date_range,
    })
}

// As per design, all href, value, and label fields in the changelist context are treated
// as plain String/Option<String> rather than Safe-wrapped minijinja::value::Value.
// This is because build_query_string returns raw URLs with unescaped ampersands.
// Entity-escaping via autoescape correctly turns '&' into '&amp;', which is spec-compliant
// and doesn't break any test assertions.
#[derive(serde::Serialize)]
struct HeaderCellData {
    href: String,
    label: String,
}

#[derive(serde::Serialize)]
struct ChangelistCellData {
    kind: &'static str, // "pk_link" | "editable" | "plain"
    value: String,
    field_name: String, // only meaningful when kind == "editable"
}

#[derive(serde::Serialize)]
struct ChangelistRowData {
    pk: String,
    cells: Vec<ChangelistCellData>,
}

#[derive(serde::Serialize)]
struct PagerData {
    prev_href: Option<String>,
    page: i64,
    total_pages: i64,
    total: i64,
    next_href: Option<String>,
}

#[derive(serde::Serialize)]
struct HiddenInputData {
    name: String,
    value: String,
}

#[derive(serde::Serialize)]
struct SearchBoxData {
    visible: bool,
    hidden_inputs: Vec<HiddenInputData>,
    q_value: String,
}

#[derive(serde::Serialize)]
struct FilterBlockData {
    field: String,
    all_href: String,
    yes_href: String,
    no_href: String,
}

#[derive(serde::Serialize)]
struct DateHierarchyLinkData {
    href: String,
    label: String,
}

#[derive(serde::Serialize)]
struct DateHierarchyData {
    visible: bool,
    breadcrumbs: Vec<DateHierarchyLinkData>,
    links: Vec<DateHierarchyLinkData>,
}

#[derive(serde::Serialize)]
struct ChangelistTemplateContext {
    date_hierarchy: DateHierarchyData,
    search: SearchBoxData,
    list_filter_blocks: Vec<FilterBlockData>,
    actions: Vec<AdminActionRow>,
    header_cells: Vec<HeaderCellData>,
    rows: Vec<ChangelistRowData>,
    show_save_button: bool,
    pager: PagerData,
    export_query: String,
    site_header: String,
    site_title: String,
    csrf_token: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
    has_add_permission: bool,
    has_change_permission: bool,
    has_delete_permission: bool,
}

async fn admin_changelist(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let user = require_perm(&req, db, admin.model_meta(), "view").await?;

    let meta = admin.model_meta();
    let has_add = if user.is_superuser {
        true
    } else {
        let codename = action_codename(meta, "add");
        djangors_auth::has_perm(db, user.id, &codename)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };
    let has_change = if user.is_superuser {
        true
    } else {
        let codename = action_codename(meta, "change");
        djangors_auth::has_perm(db, user.id, &codename)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };
    let has_delete = if user.is_superuser {
        true
    } else {
        let codename = action_codename(meta, "delete");
        djangors_auth::has_perm(db, user.id, &codename)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };

    let admin_actions = admin.actions();
    let actions_rows: Vec<AdminActionRow> = admin_actions
        .iter()
        .map(|a| AdminActionRow {
            name: a.name.to_string(),
            label: a.label.to_string(),
        })
        .collect();

    let state = parse_changelist_query_state(&req, admin.as_ref())?;
    let o = state.order;
    let q = state.search;
    let active_filters = state.filters;
    let year = state.year;
    let month = state.month;
    let day = state.day;
    let date_range = state.date_range;

    let year_str = year.map(|y| y.to_string());
    let month_str = month.map(|m| m.to_string());
    let day_str = day.map(|d| d.to_string());

    let list_filter_fields = admin.list_filter_fields();

    let page = match req.query("page") {
        Some(p_str) => {
            let p = p_str
                .parse::<i64>()
                .map_err(|_| DjangorsError::BadRequest("invalid page parameter".to_string()))?;
            if p < 1 {
                return Err(DjangorsError::BadRequest("page must be >= 1".to_string()));
            }
            p
        }
        None => 1,
    };

    let page_data = admin
        .changelist(
            db,
            o,
            page,
            CHANGELIST_PER_PAGE,
            q,
            &active_filters,
            date_range,
        )
        .await?;

    let mut header_cells = Vec::new();
    for col in &page_data.columns {
        let new_o = if o == Some(*col) {
            format!("-{}", col)
        } else {
            col.to_string()
        };
        let mut pairs = vec![("o", Some(new_o.as_str())), ("q", q)];
        for &(f, val) in &active_filters {
            pairs.push((f, Some(if val { "true" } else { "false" })));
        }
        pairs.push(("year", year_str.as_deref()));
        pairs.push(("month", month_str.as_deref()));
        pairs.push(("day", day_str.as_deref()));

        let link = build_query_string(&pairs);
        header_cells.push(HeaderCellData {
            href: link,
            label: col.to_string(),
        });
    }

    let editable_fields = admin.list_editable_fields();
    let mut rows = Vec::new();
    for (row_index, row) in page_data.rows.into_iter().enumerate() {
        let pk = page_data.pks.get(row_index).cloned().unwrap_or_default();
        let mut cells = Vec::new();
        for (i, cell) in row.into_iter().enumerate() {
            let col_name = page_data.columns.get(i).copied().unwrap_or("");
            let escaped_value = djangors_core::html_escape(&cell);
            if i == 0 {
                cells.push(ChangelistCellData {
                    kind: "pk_link",
                    value: escaped_value,
                    field_name: String::new(),
                });
            } else if editable_fields.contains(&col_name) {
                cells.push(ChangelistCellData {
                    kind: "editable",
                    value: escaped_value,
                    field_name: col_name.to_string(),
                });
            } else {
                cells.push(ChangelistCellData {
                    kind: "plain",
                    value: escaped_value,
                    field_name: String::new(),
                });
            }
        }
        rows.push(ChangelistRowData { pk, cells });
    }

    let paginator = djangors_core::Paginator::new(page_data.total, CHANGELIST_PER_PAGE);
    let total_pages = paginator.total_pages();

    let prev_href = if paginator.has_previous(page) {
        let prev_page_str = (page - 1).to_string();
        let mut pairs = vec![("page", Some(prev_page_str.as_str())), ("o", o), ("q", q)];
        for &(f, val) in &active_filters {
            pairs.push((f, Some(if val { "true" } else { "false" })));
        }
        pairs.push(("year", year_str.as_deref()));
        pairs.push(("month", month_str.as_deref()));
        pairs.push(("day", day_str.as_deref()));
        Some(build_query_string(&pairs))
    } else {
        None
    };

    let next_href = if paginator.has_next(page) {
        let next_page_str = (page + 1).to_string();
        let mut pairs = vec![("page", Some(next_page_str.as_str())), ("o", o), ("q", q)];
        for &(f, val) in &active_filters {
            pairs.push((f, Some(if val { "true" } else { "false" })));
        }
        pairs.push(("year", year_str.as_deref()));
        pairs.push(("month", month_str.as_deref()));
        pairs.push(("day", day_str.as_deref()));
        Some(build_query_string(&pairs))
    } else {
        None
    };

    let pager = PagerData {
        prev_href,
        page,
        total_pages,
        total: page_data.total,
        next_href,
    };

    let search_fields = admin.search_fields();
    let search_visible = !search_fields.is_empty();
    let mut hidden_inputs = Vec::new();
    if search_visible {
        if let Some(order_val) = o {
            hidden_inputs.push(HiddenInputData {
                name: "o".to_string(),
                value: order_val.to_string(),
            });
        }
        for &(f, val) in &active_filters {
            hidden_inputs.push(HiddenInputData {
                name: f.to_string(),
                value: (if val { "true" } else { "false" }).to_string(),
            });
        }
        if let Some(ref y) = year_str {
            hidden_inputs.push(HiddenInputData {
                name: "year".to_string(),
                value: y.clone(),
            });
        }
        if let Some(ref m) = month_str {
            hidden_inputs.push(HiddenInputData {
                name: "month".to_string(),
                value: m.clone(),
            });
        }
        if let Some(ref d) = day_str {
            hidden_inputs.push(HiddenInputData {
                name: "day".to_string(),
                value: d.clone(),
            });
        }
    }
    let search = SearchBoxData {
        visible: search_visible,
        hidden_inputs,
        q_value: q.map(djangors_core::html_escape).unwrap_or_default(),
    };

    let mut list_filter_blocks = Vec::new();
    if !list_filter_fields.is_empty() {
        for &filter_field in list_filter_fields {
            let mut pairs_all = vec![("o", o), ("q", q)];
            for &(f, val) in &active_filters {
                if f != filter_field {
                    pairs_all.push((f, Some(if val { "true" } else { "false" })));
                }
            }
            pairs_all.push(("year", year_str.as_deref()));
            pairs_all.push(("month", month_str.as_deref()));
            pairs_all.push(("day", day_str.as_deref()));
            let all_href = build_query_string(&pairs_all);

            let mut pairs_yes = vec![("o", o), ("q", q), (filter_field, Some("true"))];
            for &(f, val) in &active_filters {
                if f != filter_field {
                    pairs_yes.push((f, Some(if val { "true" } else { "false" })));
                }
            }
            pairs_yes.push(("year", year_str.as_deref()));
            pairs_yes.push(("month", month_str.as_deref()));
            pairs_yes.push(("day", day_str.as_deref()));
            let yes_href = build_query_string(&pairs_yes);

            let mut pairs_no = vec![("o", o), ("q", q), (filter_field, Some("false"))];
            for &(f, val) in &active_filters {
                if f != filter_field {
                    pairs_no.push((f, Some(if val { "true" } else { "false" })));
                }
            }
            pairs_no.push(("year", year_str.as_deref()));
            pairs_no.push(("month", month_str.as_deref()));
            pairs_no.push(("day", day_str.as_deref()));
            let no_href = build_query_string(&pairs_no);

            list_filter_blocks.push(FilterBlockData {
                field: filter_field.to_string(),
                all_href,
                yes_href,
                no_href,
            });
        }
    }

    let mut date_hierarchy_visible = false;
    let mut date_hierarchy_breadcrumbs = Vec::new();
    let mut date_hierarchy_links = Vec::new();

    if let Some(field) = admin.date_hierarchy_field() {
        date_hierarchy_visible = true;
        let values =
            date_hierarchy_drilldown_values(db, admin.model_meta().table_name, field, year, month)
                .await?;

        let mut pairs_all = vec![("o", o), ("q", q)];
        for &(f, val) in &active_filters {
            pairs_all.push((f, Some(if val { "true" } else { "false" })));
        }
        let all_link = build_query_string(&pairs_all);

        let mut pairs_y = vec![("o", o), ("q", q), ("year", year_str.as_deref())];
        for &(f, val) in &active_filters {
            pairs_y.push((f, Some(if val { "true" } else { "false" })));
        }
        let year_link = build_query_string(&pairs_y);

        let mut pairs_m = vec![
            ("o", o),
            ("q", q),
            ("year", year_str.as_deref()),
            ("month", month_str.as_deref()),
        ];
        for &(f, val) in &active_filters {
            pairs_m.push((f, Some(if val { "true" } else { "false" })));
        }
        let month_link = build_query_string(&pairs_m);

        if let Some(y) = year {
            if let Some(m) = month {
                let m_name = get_month_name(m);
                if let Some(d) = day {
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: all_link,
                        label: y.to_string(),
                    });
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: year_link,
                        label: m_name.to_string(),
                    });
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: month_link,
                        label: d.to_string(),
                    });
                } else {
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: all_link,
                        label: y.to_string(),
                    });
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: year_link,
                        label: m_name.to_string(),
                    });
                }
            } else {
                date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                    href: all_link,
                    label: y.to_string(),
                });
            }
        }

        if day.is_none() {
            for val in values {
                let val_str = val.to_string();
                let mut pairs_val = vec![("o", o), ("q", q)];
                for &(f, val) in &active_filters {
                    pairs_val.push((f, Some(if val { "true" } else { "false" })));
                }

                let link_text = if year.is_none() {
                    pairs_val.push(("year", Some(val_str.as_str())));
                    val_str.clone()
                } else if month.is_none() {
                    pairs_val.push(("year", year_str.as_deref()));
                    pairs_val.push(("month", Some(val_str.as_str())));
                    get_month_name(val as u32).to_string()
                } else {
                    pairs_val.push(("year", year_str.as_deref()));
                    pairs_val.push(("month", month_str.as_deref()));
                    pairs_val.push(("day", Some(val_str.as_str())));
                    val_str.clone()
                };

                let val_link = build_query_string(&pairs_val);
                date_hierarchy_links.push(DateHierarchyLinkData {
                    href: val_link,
                    label: link_text,
                });
            }
        }
    }

    let date_hierarchy = DateHierarchyData {
        visible: date_hierarchy_visible,
        breadcrumbs: date_hierarchy_breadcrumbs,
        links: date_hierarchy_links,
    };

    let show_save_button = !editable_fields.is_empty();

    let mut export_pairs = vec![("o", o), ("q", q)];
    for &(f, val) in &active_filters {
        export_pairs.push((f, Some(if val { "true" } else { "false" })));
    }
    export_pairs.push(("year", year_str.as_deref()));
    export_pairs.push(("month", month_str.as_deref()));
    export_pairs.push(("day", day_str.as_deref()));
    let export_query = build_query_string(&export_pairs);

    let csrf_token = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| t.0.clone())
        .unwrap_or_default();
    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/changelist.html",
        ChangelistTemplateContext {
            date_hierarchy,
            search,
            list_filter_blocks,
            header_cells,
            rows,
            show_save_button,
            pager,
            export_query,
            site_header: branding.site_header,
            site_title: branding.site_title,
            csrf_token,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
            actions: actions_rows,
            has_add_permission: has_add,
            has_change_permission: has_change,
            has_delete_permission: has_delete,
        },
    )
}

async fn admin_add_get(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    require_perm(&req, db, admin.model_meta(), "add").await?;

    let meta = admin.model_meta();
    let field_names = admin.field_names();
    let submitted_values = std::collections::HashMap::new();
    let errors = std::collections::HashMap::new();

    let csrf_token = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| t.0.clone())
        .unwrap_or_default();
    render_form(
        meta,
        &field_names,
        &submitted_values,
        &errors,
        true,
        &branding,
        csrf_token,
        true,
        false,
        admin.fieldsets(),
        admin.readonly_fields(),
        admin.raw_id_fields(),
    )
}

async fn admin_add_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let user = require_perm(&req, db, admin.model_meta(), "add").await?;

    let Form(form_data) =
        Form::<std::collections::HashMap<String, String>>::from_request(&req).await?;

    match admin.create_from_form(db, &form_data).await? {
        Ok(new_pk) => {
            log_action(
                db,
                user.id,
                admin.model_meta(),
                new_pk,
                ACTION_ADDITION,
                "Added.",
                None,
            )
            .await?;
            Ok(Response::redirect(&format!("/{}/{}/", app, model)))
        }
        Err(errors) => {
            let meta = admin.model_meta();
            let field_names = admin.field_names();
            let csrf_token = req
                .ext::<djangors_core::middleware::CsrfToken>()
                .map(|t| t.0.clone())
                .unwrap_or_default();
            render_form(
                meta,
                &field_names,
                &form_data,
                &errors,
                true,
                &branding,
                csrf_token,
                true,
                false,
                admin.fieldsets(),
                admin.readonly_fields(),
                admin.raw_id_fields(),
            )
        }
    }
}

async fn admin_change_get(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let pk = params
        .get_as::<i64>("pk")
        .map_err(|_| DjangorsError::BadRequest("invalid pk".to_string()))?;

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;

    let meta = admin.model_meta();
    let has_delete = if user.is_superuser {
        true
    } else {
        let codename = action_codename(meta, "delete");
        djangors_auth::has_perm(db, user.id, &codename)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };

    let row_vals = admin
        .get_by_pk(db, pk)
        .await?
        .ok_or(DjangorsError::NotFound)?;

    let mut submitted_values = std::collections::HashMap::new();
    for (name, val) in row_vals {
        submitted_values.insert(name.to_string(), val.to_string());
    }

    let field_names = admin.field_names();
    let errors = std::collections::HashMap::new();

    let csrf_token = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| t.0.clone())
        .unwrap_or_default();
    render_form(
        meta,
        &field_names,
        &submitted_values,
        &errors,
        false,
        &branding,
        csrf_token,
        true,
        has_delete,
        admin.fieldsets(),
        admin.readonly_fields(),
        admin.raw_id_fields(),
    )
}

async fn admin_change_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let pk = params
        .get_as::<i64>("pk")
        .map_err(|_| DjangorsError::BadRequest("invalid pk".to_string()))?;

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;

    let Form(form_data) =
        Form::<std::collections::HashMap<String, String>>::from_request(&req).await?;

    let old_row_vals = admin.get_by_pk(db, pk).await?;

    match admin.update_from_form(db, pk, &form_data).await? {
        Ok(()) => {
            let field_diff = if let Some(old_vals) = old_row_vals {
                let meta = admin.model_meta();
                let mut diff_items = Vec::new();
                for (f_name, old_val) in old_vals {
                    let mut new_val_opt = None;
                    if let Some(f) = meta.fields.iter().find(|f| f.name == f_name) {
                        if !f.auto && !f.primary_key {
                            let raw = form_data.get(f.name).map(|s| s.as_str());
                            if let Ok(nv) = parse_field_value(f, raw) {
                                new_val_opt = Some(nv);
                            }
                        }
                    } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == f_name) {
                        let raw = form_data.get(r.field_name).map(|s| s.as_str());
                        if let Ok(nv) = parse_relation_value(r, raw) {
                            new_val_opt = Some(nv);
                        }
                    }

                    if let Some(new_val) = new_val_opt {
                        if old_val != new_val {
                            diff_items.push(FieldDiffItem {
                                field: f_name.to_string(),
                                old: old_val.to_string(),
                                new: new_val.to_string(),
                            });
                        }
                    }
                }
                if diff_items.is_empty() {
                    None
                } else {
                    serde_json::to_string(&diff_items).ok()
                }
            } else {
                None
            };

            log_action(
                db,
                user.id,
                admin.model_meta(),
                pk,
                ACTION_CHANGE,
                "Changed.",
                field_diff,
            )
            .await?;
            Ok(Response::redirect(&format!("/{}/{}/", app, model)))
        }
        Err(errors) => {
            let row_vals = admin
                .get_by_pk(db, pk)
                .await?
                .ok_or(DjangorsError::NotFound)?;

            let mut merged_form_data = form_data.clone();
            for (name, val) in row_vals {
                let meta = admin.model_meta();
                if let Some(f) = meta.fields.iter().find(|f| f.name == name) {
                    if f.auto || f.primary_key {
                        merged_form_data.insert(name.to_string(), val.to_string());
                    }
                }
            }

            let meta = admin.model_meta();
            let has_delete = if user.is_superuser {
                true
            } else {
                let codename = action_codename(meta, "delete");
                djangors_auth::has_perm(db, user.id, &codename)
                    .await
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?
            };
            let field_names = admin.field_names();
            let csrf_token = req
                .ext::<djangors_core::middleware::CsrfToken>()
                .map(|t| t.0.clone())
                .unwrap_or_default();
            render_form(
                meta,
                &field_names,
                &merged_form_data,
                &errors,
                false,
                &branding,
                csrf_token,
                true,
                has_delete,
                admin.fieldsets(),
                admin.readonly_fields(),
                admin.raw_id_fields(),
            )
        }
    }
}

pub struct RelatedObjectSummary {
    pub app_label: &'static str,
    pub struct_name: &'static str,
    pub field_name: &'static str,
    pub on_delete: djangors_orm::meta::OnDelete,
    pub count: i64,
    pub nested: Vec<RelatedObjectSummary>,
}

async fn collect_related_objects(
    db: &djangors_db::Database,
    target_meta: &'static ModelMeta,
    pk: i64,
) -> Result<Vec<RelatedObjectSummary>, DjangorsError> {
    collect_related_objects_with_depth(
        db,
        target_meta,
        pk,
        2,
        &mut std::collections::HashSet::new(),
    )
    .await
}

fn collect_related_objects_with_depth<'a>(
    db: &'a djangors_db::Database,
    target_meta: &'static ModelMeta,
    pk: i64,
    depth: u32,
    visited: &'a mut std::collections::HashSet<&'static str>,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<Vec<RelatedObjectSummary>, DjangorsError>>
            + Send
            + 'a,
    >,
> {
    Box::pin(async move {
        let mut summaries = Vec::new();
        for related_meta in djangors_orm::meta::all_registered_models() {
            for relation in related_meta.relations {
                if (relation.target)().table_name != target_meta.table_name {
                    continue;
                }
                let sql = format!(
                    "SELECT COUNT(*) FROM {} WHERE {} = $1",
                    related_meta.table_name, relation.field_name
                );
                let count_result: Result<i64, _> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                    .bind(pk)
                    .fetch_one(db.pool())
                    .await;
                let count = match count_result {
                    Ok(c) => c,
                    Err(e) if e.to_string().contains("does not exist") => continue,
                    Err(e) => return Err(DjangorsError::Internal(e.to_string())),
                };
                if count > 0 {
                    let mut nested = Vec::new();
                    if depth > 0 && !visited.contains(related_meta.table_name) {
                        visited.insert(related_meta.table_name);
                        nested = collect_related_objects_with_depth(
                            db,
                            related_meta,
                            pk,
                            depth - 1,
                            visited,
                        )
                        .await?;
                    }
                    summaries.push(RelatedObjectSummary {
                        app_label: related_meta.app_label,
                        struct_name: related_meta.struct_name,
                        field_name: relation.field_name,
                        on_delete: relation.on_delete,
                        count,
                        nested,
                    });
                }
            }
        }
        Ok(summaries)
    })
}

/// Returns the distinct values one drilldown level below the given
/// `(year, month)` state, in ascending order: years if `year` is `None`,
/// months within that year if `month` is `None`, else days within that
/// year+month. Computed from the whole table on `col` alone — see the design
/// doc's scope note on why this isn't combined with the current
/// search/list_filter state.
async fn date_hierarchy_drilldown_values(
    db: &djangors_db::Database,
    table_name: &'static str,
    col: &'static str,
    year: Option<i32>,
    month: Option<u32>,
) -> Result<Vec<i32>, DjangorsError> {
    let (sql, bind_year, bind_month): (String, Option<i32>, Option<i32>) = match (year, month) {
        (None, _) => (
            format!(
                "SELECT DISTINCT EXTRACT(YEAR FROM {col})::int AS v FROM {table_name} \
                 WHERE {col} IS NOT NULL ORDER BY 1"
            ),
            None,
            None,
        ),
        (Some(y), None) => (
            format!(
                "SELECT DISTINCT EXTRACT(MONTH FROM {col})::int AS v FROM {table_name} \
                 WHERE {col} IS NOT NULL AND EXTRACT(YEAR FROM {col})::int = $1 ORDER BY 1"
            ),
            Some(y),
            None,
        ),
        (Some(y), Some(m)) => (
            format!(
                "SELECT DISTINCT EXTRACT(DAY FROM {col})::int AS v FROM {table_name} \
                 WHERE {col} IS NOT NULL AND EXTRACT(YEAR FROM {col})::int = $1 \
                 AND EXTRACT(MONTH FROM {col})::int = $2 ORDER BY 1"
            ),
            Some(y),
            Some(m as i32),
        ),
    };
    let mut query = sqlx::query_scalar(sqlx::AssertSqlSafe(sql));
    if let Some(y) = bind_year {
        query = query.bind(y);
    }
    if let Some(m) = bind_month {
        query = query.bind(m);
    }
    query
        .fetch_all(db.pool())
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))
}

#[derive(serde::Serialize)]
struct DeleteConfirmField {
    name: String,
    value: String,
}

#[derive(serde::Serialize)]
struct DeleteConfirmRelated {
    struct_name: String,
    table_name: String,
    count: i64,
    on_delete: String,
    nested: Vec<DeleteConfirmRelated>,
}

fn flatten_related(rel: RelatedObjectSummary) -> DeleteConfirmRelated {
    let table_name = djangors_orm::meta::all_registered_models()
        .find(|m| m.app_label == rel.app_label && m.struct_name == rel.struct_name)
        .map(|m| m.table_name)
        .unwrap_or("");
    DeleteConfirmRelated {
        struct_name: rel.struct_name.to_string(),
        table_name: table_name.to_string(),
        count: rel.count,
        on_delete: format!("{:?}", rel.on_delete),
        nested: rel.nested.into_iter().map(flatten_related).collect(),
    }
}

#[derive(serde::Serialize)]
struct DeleteConfirmContext {
    fields: Vec<DeleteConfirmField>,
    related: Vec<DeleteConfirmRelated>,
    site_header: String,
    site_title: String,
    csrf_token: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
}

#[derive(serde::Serialize)]
struct BulkDeleteConfirmContext {
    count: usize,
    items: Vec<String>,
    pks: Vec<i64>,
    site_header: String,
    site_title: String,
    csrf_token: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
}

async fn admin_delete_get(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let pk = params
        .get_as::<i64>("pk")
        .map_err(|_| DjangorsError::BadRequest("invalid pk".to_string()))?;

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    require_perm(&req, db, admin.model_meta(), "delete").await?;

    let row_vals = admin
        .get_by_pk(db, pk)
        .await?
        .ok_or(DjangorsError::NotFound)?;

    let meta = admin.model_meta();
    let related = collect_related_objects(db, meta, pk).await?;

    let mut fields = Vec::new();
    for (name, val) in row_vals {
        fields.push(DeleteConfirmField {
            name: name.to_string(),
            value: val.to_string(),
        });
    }

    let mut related_context = Vec::new();
    for rel in related {
        related_context.push(flatten_related(rel));
    }

    let csrf_token = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| t.0.clone())
        .unwrap_or_default();
    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/delete_confirm.html",
        DeleteConfirmContext {
            fields,
            related: related_context,
            site_header: branding.site_header,
            site_title: branding.site_title,
            csrf_token,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
        },
    )
}

async fn admin_delete_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let pk = params
        .get_as::<i64>("pk")
        .map_err(|_| DjangorsError::BadRequest("invalid pk".to_string()))?;

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let user = require_perm(&req, db, admin.model_meta(), "delete").await?;

    let meta = admin.model_meta();
    let related = collect_related_objects(db, meta, pk).await?;
    for rel in &related {
        if rel.on_delete == djangors_orm::meta::OnDelete::Protect && rel.count > 0 {
            return Err(DjangorsError::BadRequest(format!(
                "Cannot delete '{}' because it is protected by related '{}'",
                meta.struct_name, rel.struct_name
            )));
        }
    }

    let deleted = admin.delete_by_pk(db, pk).await?;
    if deleted {
        log_action(
            db,
            user.id,
            admin.model_meta(),
            pk,
            ACTION_DELETION,
            "Deleted.",
            None,
        )
        .await?;
        Ok(Response::redirect(&format!("/{}/{}/", app, model)))
    } else {
        Err(DjangorsError::NotFound)
    }
}

#[derive(serde::Serialize)]
struct BulkActionConfirmContext {
    action_label: String,
    count: usize,
    items: Vec<String>,
    action_name: String,
    pks: Vec<i64>,
    csrf_token: String,
    site_header: String,
    site_title: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
}

#[derive(serde::Serialize)]
struct HistoryEntryView {
    action_time: String,
    action_flag: i32,
    username: String,
    change_message: String,
    field_diff: Option<Vec<FieldDiffItem>>,
}

#[derive(serde::Serialize)]
struct ObjectHistoryContext {
    model_name: String,
    pk: i64,
    history: Vec<HistoryEntryView>,
    site_header: String,
    site_title: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
}

#[allow(clippy::type_complexity)]
async fn admin_history(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let pk = params
        .get_as::<i64>("pk")
        .map_err(|_| DjangorsError::BadRequest("invalid pk".to_string()))?;

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let meta = admin.model_meta();

    let rows: Vec<(
        i64,
        String,
        chrono::DateTime<chrono::Utc>,
        i32,
        String,
        Option<String>,
    )> = sqlx::query_as(sqlx::AssertSqlSafe(
        "SELECT l.user_id, COALESCE(u.username, '?'), l.action_time, l.action_flag, l.change_message, l.field_diff \
         FROM djangors_admin_log l \
         LEFT JOIN auth_user u ON u.id = l.user_id \
         WHERE l.app_label = $1 AND l.model_name = $2 AND l.object_id = $3 \
         ORDER BY l.action_time DESC",
    ))
    .bind(meta.app_label)
    .bind(meta.struct_name)
    .bind(pk)
    .fetch_all(db.pool())
    .await
    .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let history: Vec<HistoryEntryView> = rows
        .into_iter()
        .map(
            |(_, username, action_time, action_flag, change_message, raw_diff)| {
                let field_diff = raw_diff.and_then(|s| {
                    let items: Vec<FieldDiffItem> = serde_json::from_str(&s).ok()?;
                    if items.is_empty() {
                        None
                    } else {
                        Some(items)
                    }
                });
                HistoryEntryView {
                    action_time: action_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    action_flag,
                    username,
                    change_message,
                    field_diff,
                }
            },
        )
        .collect();

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/object_history.html",
        ObjectHistoryContext {
            model_name: meta.struct_name.to_string(),
            pk,
            history,
            site_header: branding.site_header,
            site_title: branding.site_title,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
        },
    )
}

async fn admin_bulk_action_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;

    let Form(pairs) = Form::<Vec<(String, String)>>::from_request(&req).await?;

    let action_name: String = pairs
        .iter()
        .find(|(k, _)| k == "action")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let is_confirm = pairs.iter().any(|(k, v)| k == "confirm" && v == "1");
    let mut pks: Vec<i64> = Vec::new();
    for (k, v) in &pairs {
        if k == "selected" {
            let pk = v
                .parse::<i64>()
                .map_err(|_| DjangorsError::BadRequest("invalid selected pk".to_string()))?;
            pks.push(pk);
        }
    }

    let admin_actions = admin.actions();
    let action = admin_actions
        .iter()
        .find(|a| a.name == action_name)
        .ok_or_else(|| DjangorsError::BadRequest("unknown action".to_string()))?;

    if action.requires_confirm && !is_confirm {
        let mut items = Vec::new();
        for &pk in &pks {
            if let Some(row_vals) = admin.get_by_pk(db, pk).await? {
                let display = row_vals
                    .iter()
                    .map(|(_, v)| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                items.push(display);
            }
        }
        let count = pks.len();
        let csrf_token = req
            .ext::<djangors_core::middleware::CsrfToken>()
            .map(|t| t.0.clone())
            .unwrap_or_default();
        return djangors_template::render(
            &ADMIN_TEMPLATES,
            "admin/bulk_action_confirm.html",
            BulkActionConfirmContext {
                action_label: action.label.to_string(),
                count,
                items,
                action_name: action.name.to_string(),
                pks: pks.clone(),
                csrf_token,
                site_header: branding.site_header,
                site_title: branding.site_title,
                logo_url: branding.logo_url,
                accent_color: branding.accent_color,
            },
        );
    }

    (action.handler)(db, &pks).await?;

    let meta = admin.model_meta();
    for &pk in &pks {
        log_action(db, user.id, meta, pk, ACTION_CHANGE, action.label, None).await?;
    }

    Ok(Response::redirect(&format!("/{}/{}/", app, model)))
}

async fn admin_bulk_delete_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let user = require_perm(&req, db, admin.model_meta(), "delete").await?;

    let Form(pairs) = Form::<Vec<(String, String)>>::from_request(&req).await?;
    let is_confirm = pairs.iter().any(|(k, v)| k == "confirm" && v == "1");
    let mut pks: Vec<i64> = Vec::new();
    for (k, v) in &pairs {
        if k == "selected" {
            let pk = v
                .parse::<i64>()
                .map_err(|_| DjangorsError::BadRequest("invalid selected pk".to_string()))?;
            pks.push(pk);
        }
    }

    if pks.is_empty() {
        // Nothing selected - go back to the changelist, nothing to confirm or do.
        return Ok(Response::redirect(&format!("/{}/{}/", app, model)));
    }

    if !is_confirm {
        // Step 1: render the confirm page, listing each selected object.
        let mut items = Vec::new();
        for &pk in &pks {
            if let Some(row_vals) = admin.get_by_pk(db, pk).await? {
                let display = row_vals
                    .iter()
                    .map(|(_, v)| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                items.push(display);
            }
            // A pk that no longer exists (already deleted concurrently) is just
            // skipped from the listing - it'll be a no-op in step 2 as well.
        }
        let count = pks.len();
        let csrf_token = req
            .ext::<djangors_core::middleware::CsrfToken>()
            .map(|t| t.0.clone())
            .unwrap_or_default();
        return djangors_template::render(
            &ADMIN_TEMPLATES,
            "admin/bulk_delete_confirm.html",
            BulkDeleteConfirmContext {
                count,
                items,
                pks: pks.clone(),
                site_header: branding.site_header,
                site_title: branding.site_title,
                csrf_token,
                logo_url: branding.logo_url,
                accent_color: branding.accent_color,
            },
        );
    }

    // Step 2: actually delete. Best-effort per pk - a pk already gone (race, or
    // simply reselected after step 1 already removed it) is not an error, same
    // "false = already gone, not a failure" reasoning admin_delete_post already
    // uses for the single-object route. A pk protected by a Protect on_delete
    // relation is also silently skipped (not an error).
    let delete_meta = admin.model_meta();
    for &pk in &pks {
        let related = collect_related_objects(db, delete_meta, pk).await?;
        let is_protected = related
            .iter()
            .any(|r| r.on_delete == djangors_orm::meta::OnDelete::Protect && r.count > 0);
        if is_protected {
            continue;
        }
        if admin.delete_by_pk(db, pk).await? {
            log_action(
                db,
                user.id,
                admin.model_meta(),
                pk,
                ACTION_DELETION,
                "Deleted.",
                None,
            )
            .await?;
        }
    }
    Ok(Response::redirect(&format!("/{}/{}/", app, model)))
}

#[derive(serde::Serialize)]
struct SaveChangelistErrorRow {
    pk: i64,
    field: String,
    message: String,
}

#[derive(serde::Serialize)]
struct SaveChangelistErrorContext {
    errors: Vec<SaveChangelistErrorRow>,
    app: String,
    model: String,
    site_header: String,
    site_title: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
}

async fn admin_save_changelist_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;

    let editable_fields = admin.list_editable_fields();
    let Form(pairs) = Form::<Vec<(String, String)>>::from_request(&req).await?;

    // Group by pk, allowlisting field names against list_editable_fields() -
    // a key naming a field that isn't configured as editable is silently
    // dropped, not applied and not an error (defense in depth: mirrors
    // list_filter's route-level allowlist check, in case a crafted request
    // names a real-but-not-editable field).
    let mut by_pk: std::collections::HashMap<i64, std::collections::HashMap<String, String>> =
        std::collections::HashMap::new();
    for (key, value) in &pairs {
        let Some(rest) = key.strip_prefix("edit-") else {
            continue;
        };
        let Some((pk_str, field_name)) = rest.split_once('-') else {
            continue;
        };
        let pk = pk_str
            .parse::<i64>()
            .map_err(|_| DjangorsError::BadRequest("invalid pk in edit field".to_string()))?;
        if !editable_fields.contains(&field_name) {
            continue;
        }
        by_pk
            .entry(pk)
            .or_default()
            .insert(field_name.to_string(), value.clone());
    }

    let mut row_errors: Vec<(i64, std::collections::HashMap<String, String>)> = Vec::new();
    for (pk, fields) in &by_pk {
        match admin.update_fields_from_form(db, *pk, fields).await? {
            Ok(()) => {
                log_action(
                    db,
                    user.id,
                    admin.model_meta(),
                    *pk,
                    ACTION_CHANGE,
                    "Changed.",
                    None,
                )
                .await?;
            }
            Err(errors) => row_errors.push((*pk, errors)),
        }
    }

    if row_errors.is_empty() {
        return Ok(Response::redirect(&format!("/{}/{}/", app, model)));
    }

    let mut errors = Vec::new();
    for (pk, error_map) in &row_errors {
        for (field, msg) in error_map {
            errors.push(SaveChangelistErrorRow {
                pk: *pk,
                field: field.clone(),
                message: msg.clone(),
            });
        }
    }

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/save_changelist_error.html",
        SaveChangelistErrorContext {
            errors,
            app: app.to_string(),
            model: model.to_string(),
            site_header: branding.site_header,
            site_title: branding.site_title,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
        },
    )
}

pub fn favicon_routes(router: Router) -> Router {
    router
        .get("/favicon.ico", favicon_ico)
        .get("/favicon-16x16.png", favicon_16)
        .get("/favicon-32x32.png", favicon_32)
        .get("/apple-touch-icon.png", favicon_apple_touch)
        .get("/android-chrome-192x192.png", favicon_android_192)
        .get("/android-chrome-512x512.png", favicon_android_512)
        .get("/manifest.json", favicon_manifest)
}

async fn favicon_ico(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/x-icon",
        include_bytes!("../static/branding/favicon.ico").to_vec(),
    ))
}

async fn favicon_16(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/favicon-16x16.png").to_vec(),
    ))
}

async fn favicon_32(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/favicon-32x32.png").to_vec(),
    ))
}

async fn favicon_apple_touch(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/apple-touch-icon.png").to_vec(),
    ))
}

async fn favicon_android_192(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/android-chrome-192x192.png").to_vec(),
    ))
}

async fn favicon_android_512(
    _req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "image/png",
        include_bytes!("../static/branding/android-chrome-512x512.png").to_vec(),
    ))
}

async fn favicon_manifest(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::bytes(
        StatusCode::OK,
        "application/json; charset=utf-8",
        include_bytes!("../static/branding/manifest.json").to_vec(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use djangors_auth::SESSION_USER_ID_KEY;
    use djangors_macros::Model as MacroModel;
    use hyper::http::{Extensions, HeaderMap, Method, Uri};

    static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_a")]
    #[allow(dead_code)]
    struct ModelA {
        #[djangors(primary_key, auto)]
        id: i64,
        name: String,
    }

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_b", ordering = "-title")]
    #[allow(dead_code)]
    struct ModelB {
        #[djangors(primary_key, auto)]
        id: i64,
        title: String,
    }

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_c")]
    #[allow(dead_code)]
    struct ModelC {
        #[djangors(primary_key, auto)]
        id: i64,
        parent: djangors_orm::ForeignKey<ModelA>,
    }

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_d")]
    #[allow(dead_code)]
    struct ModelD {
        #[djangors(primary_key, auto)]
        id: i64,
        name: String,
        content: String,
    }

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_e")]
    #[allow(dead_code)]
    struct ModelE {
        #[djangors(primary_key, auto)]
        id: i64,
        name: String,
        is_active: bool,
        is_staff: bool,
    }

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_f")]
    #[allow(dead_code)]
    struct ModelF {
        #[djangors(primary_key, auto)]
        id: i64,
        name: String,
        created_at: chrono::DateTime<chrono::Utc>,
    }

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_g")]
    #[allow(dead_code)]
    struct ModelG {
        #[djangors(primary_key, auto)]
        id: i64,
        parent: djangors_orm::ForeignKey<ModelC>,
    }

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_h")]
    #[allow(dead_code)]
    struct ModelH {
        #[djangors(primary_key, auto)]
        id: i64,
        parent: djangors_orm::ForeignKey<ModelG>,
    }

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_p")]
    #[allow(dead_code)]
    struct ModelP {
        #[djangors(primary_key, auto)]
        id: i64,
        #[djangors(foreign_key(on_delete = "protect"))]
        parent: djangors_orm::ForeignKey<ModelA>,
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_index_endpoints() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_b")
            .execute(db.pool())
            .await;

        // Create auth_user
        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();

        // 1. Create a non-staff user
        let non_staff = User {
            id: 0,
            username: "non_staff".to_string(),
            email: "non_staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        // 2. Create a staff user
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        // Setup AdminSite
        let site = AdminSite::new();
        site.register::<ModelA>();
        site.register::<ModelB>();

        // Mount into a parent router, exactly like real usage -
        // `Router::mount` never merges a sub-router's own state into the
        // parent (see `AdminSite::urls`'s own doc comment), so testing
        // directly against `site.urls()` alone would not catch a
        // regression where `admin_index` went back to depending on
        // sub-router state instead of its closure-captured registry. In
        // real production, `Router::dispatch` (used by `Djangors::run`)
        // automatically attaches the top-level router's own `.with_state`
        // to every request; this test calls `.handle()` directly instead,
        // so `Database` state is attached per-request manually below.
        let router = Router::new().mount("/admin", site.urls());

        // Test 1: GET /admin/ with no auth -> 401 Unauthorized
        let req_no_auth = Request::new(
            Method::GET,
            Uri::from_static("/admin/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_no_auth = router.handle(req_no_auth).await;
        assert!(res_no_auth.is_err());
        assert_eq!(
            res_no_auth.unwrap_err().status_code(),
            StatusCode::UNAUTHORIZED
        );

        // Test 2: GET /admin/ authenticated as non-staff -> 403 Forbidden
        let session_non_staff = djangors_sessions::Session::new_empty();
        session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);
        let mut extensions_non_staff = Extensions::new();
        extensions_non_staff.insert(session_non_staff);
        let req_non_staff = Request::new(
            Method::GET,
            Uri::from_static("/admin/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(extensions_non_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_non_staff = router.handle(req_non_staff).await;
        assert!(res_non_staff.is_err());
        assert_eq!(
            res_non_staff.unwrap_err().status_code(),
            StatusCode::FORBIDDEN
        );

        // Test 3: GET /admin/ authenticated as staff -> 200 OK with correct HTML
        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);
        let mut extensions_staff = Extensions::new();
        extensions_staff.insert(session_staff);
        let req_staff = Request::new(
            Method::GET,
            Uri::from_static("/admin/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(extensions_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_staff = router.handle(req_staff).await.unwrap();
        assert_eq!(res_staff.status(), StatusCode::OK);
        let body = String::from_utf8(res_staff.body().to_vec()).unwrap();
        assert!(body.contains("<li><a href=\"admin_test/modela/\">admin_test.ModelA</a></li>"));
        assert!(body.contains("<li><a href=\"admin_test/modelb/\">admin_test.ModelB</a></li>"));

        // Clean up
        let _ = sqlx::query("DROP TABLE auth_user").execute(db.pool()).await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_changelist_endpoints() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_b")
            .execute(db.pool())
            .await;

        // Create auth_user
        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Create test_model_a
        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Create test_model_b (has ordering = "-title" meta)
        sqlx::query(
            "CREATE TABLE test_model_b (
                id BIGSERIAL PRIMARY KEY,
                title TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();

        // Create users
        let non_staff = User {
            id: 0,
            username: "non_staff".to_string(),
            email: "non_staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        // Seed ModelA rows
        let _row1 = ModelA {
            id: 0,
            name: "Normal Row".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let _row2 = ModelA {
            id: 0,
            name: "<script>alert(1)</script>".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let site = AdminSite::new();
        site.register::<ModelA>();
        site.register::<ModelB>();

        let router = Router::new().mount("/admin", site.urls());

        // Test 1: GET /admin/admin_test/modela/ as staff -> 200, checks headers & XSS escape
        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);
        let mut extensions_staff = Extensions::new();
        extensions_staff.insert(session_staff.clone());
        let req_list = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(extensions_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));

        let res_list = router.handle(req_list).await.unwrap();
        assert_eq!(res_list.status(), StatusCode::OK);
        let body = String::from_utf8(res_list.body().to_vec()).unwrap();

        // Assert header links
        assert!(body.contains("<a href=\"?o=id\">id</a>"));
        assert!(body.contains("<a href=\"?o=name\">name</a>"));

        // Assert row content and XSS escaping
        assert!(body.contains("Normal Row"));
        assert!(body.contains("&lt;script&gt;alert(1)&lt;&#x2F;script&gt;"));
        assert!(!body.contains("<script>"));
        // Assert changelist first cell contains change links
        assert!(body.contains("<a href=\"1/change/\">1</a>"));
        assert!(body.contains("<a href=\"2/change/\">2</a>"));

        // Test 2: Ordering tests
        // Seed rows for sorting
        let _ = sqlx::query("TRUNCATE test_model_a RESTART IDENTITY")
            .execute(db.pool())
            .await;

        let _row_a = ModelA {
            id: 0,
            name: "Row A".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _row_b = ModelA {
            id: 0,
            name: "Row B".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        // GET with ascending order ?o=name
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_asc = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/?o=name"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_asc = router.handle(req_asc).await.unwrap();
        let body_asc = String::from_utf8(res_asc.body().to_vec()).unwrap();
        let idx_a = body_asc.find("Row A").unwrap();
        let idx_b = body_asc.find("Row B").unwrap();
        assert!(idx_a < idx_b);

        // GET with descending order ?o=-name
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_desc = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/?o=-name"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_desc = router.handle(req_desc).await.unwrap();
        let body_desc = String::from_utf8(res_desc.body().to_vec()).unwrap();
        let idx_a_desc = body_desc.find("Row A").unwrap();
        let idx_b_desc = body_desc.find("Row B").unwrap();
        assert!(idx_b_desc < idx_a_desc);

        // GET with invalid field ?o=nonsense -> 400
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_invalid = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/?o=nonsense"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_invalid = router.handle(req_invalid).await;
        assert!(res_invalid.is_err());
        assert_eq!(
            res_invalid.unwrap_err().status_code(),
            StatusCode::BAD_REQUEST
        );

        // Test 3: Pagination tests
        // Seed 3 more rows to have 5 rows in total
        let _ = ModelA {
            id: 0,
            name: "Row C".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _ = ModelA {
            id: 0,
            name: "Row D".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _ = ModelA {
            id: 0,
            name: "Row E".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        // Direct trait call: admin.changelist(&db, None, 2, 2)
        let admin_a = DefaultModelAdmin::<ModelA> {
            config: ModelAdminConfig::default(),
            _marker: PhantomData,
        };
        let page_data = admin_a
            .changelist(&db, None, 2, 2, None, &[], None)
            .await
            .unwrap();
        assert_eq!(page_data.total, 5);
        assert_eq!(page_data.rows.len(), 2); // Row 3 and Row 4
        assert_eq!(page_data.rows[0][1], "Row C");
        assert_eq!(page_data.rows[1][1], "Row D");

        // HTTP assert that ?page=2 with the real constant (100) and few rows returns 200 with empty table body and Previous link
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_page2 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/?page=2"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_page2 = router.handle(req_page2).await.unwrap();
        assert_eq!(res_page2.status(), StatusCode::OK);
        let body_page2 = String::from_utf8(res_page2.body().to_vec()).unwrap();
        assert!(body_page2.contains("<tbody></tbody>"));
        assert!(body_page2.contains("Previous"));

        // Test 4: Auth permissions
        // Unauthenticated -> 401
        let req_unauth = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_unauth = router.handle(req_unauth).await;
        assert!(res_unauth.is_err());
        assert_eq!(
            res_unauth.unwrap_err().status_code(),
            StatusCode::UNAUTHORIZED
        );

        // Non-staff -> 403
        let session_non_staff = djangors_sessions::Session::new_empty();
        session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);
        let mut ext_non_staff = Extensions::new();
        ext_non_staff.insert(session_non_staff);
        let req_non_staff = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_non_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_non_staff = router.handle(req_non_staff).await;
        assert!(res_non_staff.is_err());
        assert_eq!(
            res_non_staff.unwrap_err().status_code(),
            StatusCode::FORBIDDEN
        );

        // Unknown route as staff -> 404
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_unknown = Request::new(
            Method::GET,
            Uri::from_static("/admin/nope/nope/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_unknown = router.handle(req_unknown).await;
        assert!(res_unknown.is_err());
        assert_eq!(
            res_unknown.unwrap_err().status_code(),
            StatusCode::NOT_FOUND
        );

        // Test 5: default order (no ?o=) honors the model's `ordering` meta -
        // ModelB declares ordering = "-title", so with no explicit sort param
        // "Beta" must come before "Alpha" even though "Alpha" was saved first.
        let _ = ModelB {
            id: 0,
            title: "Alpha".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _ = ModelB {
            id: 0,
            title: "Beta".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_default_order = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modelb/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_default_order = router.handle(req_default_order).await.unwrap();
        assert_eq!(res_default_order.status(), StatusCode::OK);
        let body_default_order = String::from_utf8(res_default_order.body().to_vec()).unwrap();
        let idx_beta = body_default_order.find("Beta").unwrap();
        let idx_alpha = body_default_order.find("Alpha").unwrap();
        assert!(idx_beta < idx_alpha);

        // Clean up
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE auth_user").execute(db.pool()).await;
        let _ = sqlx::query("DROP TABLE test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE test_model_b")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_change_form_endpoints() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop/create tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let non_staff = User {
            id: 0,
            username: "non_staff".to_string(),
            email: "non_staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let site = AdminSite::new();
        site.register::<ModelA>();
        let router = Router::new().mount("/admin", site.urls());

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        let session_non_staff = djangors_sessions::Session::new_empty();
        session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);

        // 1. GET add/ as staff -> 200, input per non-auto field (e.g. name)
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/add/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("name=\"name\""));
        assert!(!body.contains("name=\"id\"")); // id is auto, so omitted

        // 2. POST add/ valid data -> redirect + row exists in DB
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/add/"),
            headers,
            Bytes::from("name=New+Added+Row"),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(
            res.headers().get("location").unwrap().to_str().unwrap(),
            "/admin_test/modela/"
        );

        // Verify row exists
        let row = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(name = "New Added Row"))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        let new_pk = row.id;

        // 3. POST add/ with unparseable or validation errors (e.g. name is empty, which is required because it's not nullable) -> 200 re-rendered with error message
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/add/"),
            headers,
            Bytes::from("name="), // empty
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Field &#x27;name&#x27; is required."));

        // 4. GET change/{pk} -> 200 with prefilled value
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::try_from(format!("/admin/admin_test/modela/{}/change/", new_pk)).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("value=\"New Added Row\""));
        assert!(body.contains("id (readonly):"));

        // 5. POST change/{pk} valid -> row updated in DB
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req = Request::new(
            Method::POST,
            Uri::try_from(format!("/admin/admin_test/modela/{}/change/", new_pk)).unwrap(),
            headers,
            Bytes::from("name=Updated+Row+Name"),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);

        let row_updated = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(id = new_pk))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        assert_eq!(row_updated.name, "Updated Row Name");

        // 6. GET change/{pk} for nonexistent pk -> 404
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/99999/change/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().status_code(), StatusCode::NOT_FOUND);

        // 7. Unauthenticated GET add/ -> 401
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/add/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().status_code(), StatusCode::UNAUTHORIZED);

        // 8. Non-staff GET add/ -> 403
        let mut ext = Extensions::new();
        ext.insert(session_non_staff);
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/add/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().status_code(), StatusCode::FORBIDDEN);

        // Clean up
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE auth_user").execute(db.pool()).await;
        let _ = sqlx::query("DROP TABLE test_model_a")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_delete_endpoints() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop/create tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_c (
                id BIGSERIAL PRIMARY KEY,
                parent BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let non_staff = User {
            id: 0,
            username: "non_staff".to_string(),
            email: "non_staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let site = AdminSite::new();
        site.register::<ModelA>();
        site.register::<ModelC>();
        let router = Router::new().mount("/admin", site.urls());

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        let session_non_staff = djangors_sessions::Session::new_empty();
        session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);

        // Insert test data
        let parent_pk = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
            &db,
            vec![(
                "name",
                djangors_orm::expr::Value::Text("Parent Object".to_string()),
            )],
        )
        .await
        .unwrap();

        // Insert child referencing parent
        let child_pk = djangors_orm::queryset::QuerySet::<ModelC>::insert_raw(
            &db,
            vec![("parent", djangors_orm::expr::Value::I64(parent_pk))],
        )
        .await
        .unwrap();

        // a. GET delete confirm page for an existing pk as staff -> 200, contains field values and related ModelC warning showing count 1
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", parent_pk)).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Parent Object"));
        assert!(body.contains("ModelC") || body.contains("test_model_c"));
        assert!(body.contains("1")); // shows count of 1

        // b. GET delete confirm page for a nonexistent pk -> 404
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/99999/delete/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().status_code(), StatusCode::NOT_FOUND);

        // c. Unauthenticated GET -> 401. Non-staff GET -> 403.
        let req = Request::new(
            Method::GET,
            Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", parent_pk)).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().status_code(), StatusCode::UNAUTHORIZED);

        let mut ext = Extensions::new();
        ext.insert(session_non_staff);
        let req = Request::new(
            Method::GET,
            Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", parent_pk)).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().status_code(), StatusCode::FORBIDDEN);

        // d. POST delete for an existing pk
        // First delete child
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::POST,
            Uri::try_from(format!("/admin/admin_test/modelc/{}/delete/", child_pk)).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(
            res.headers().get("location").unwrap().to_str().unwrap(),
            "/admin_test/modelc/"
        );

        // Verify child is gone from DB
        let child_check = djangors_orm::queryset::QuerySet::<ModelC>::new()
            .filter(djangors_orm::q!(id = child_pk))
            .unwrap()
            .get(&db)
            .await;
        assert!(child_check.is_err());

        // e. POST delete for a nonexistent pk -> 404
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modelc/99999/delete/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().status_code(), StatusCode::NOT_FOUND);

        // Clean up
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE auth_user").execute(db.pool()).await;
        let _ = sqlx::query("DROP TABLE test_model_a")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_list_display_and_search() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop/create tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        let _ = sqlx::query(
            "CREATE TABLE test_model_d (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await;

        let _ = sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await;

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        // 1. register_with with a list_display subset that excludes the pk field
        let site = AdminSite::new();
        site.register_with::<ModelD>(ModelAdminConfig {
            list_display: Some(&["content", "name"]),
            search_fields: None,
            ..Default::default()
        });

        let pk_d = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Alice".to_string())),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Hello world".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        let router = Router::new().mount("/admin", site.urls());
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modeld/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();

        assert!(body.contains("content"));
        assert!(body.contains("name"));
        assert!(!body.contains("<th><a href=\"?o=id\">id</a></th>"));
        assert!(!body.contains("<th><a href=\"?o=-id\">id</a></th>"));
        assert!(body.contains(&format!("<a href=\"{}/change/\">Hello world</a>", pk_d)));

        // 2. register_with with a list_display naming a field that doesn't exist on the model -> panics
        let site2 = AdminSite::new();
        let res2 = std::panic::catch_unwind(|| {
            site2.register_with::<ModelD>(ModelAdminConfig {
                list_display: Some(&["nonexistent"]),
                search_fields: None,
                ..Default::default()
            });
        });
        assert!(res2.is_err());

        // 3. register_with with search_fields naming a non-text field -> panics
        let site3 = AdminSite::new();
        let res3 = std::panic::catch_unwind(|| {
            site3.register_with::<ModelD>(ModelAdminConfig {
                list_display: None,
                search_fields: Some(&["id"]),
                ..Default::default()
            });
        });
        assert!(res3.is_err());

        // Clean up rows for search tests
        let _ = sqlx::query("TRUNCATE test_model_d RESTART IDENTITY")
            .execute(db.pool())
            .await;

        // Insert 3 rows
        let _ = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Apple".to_string())),
                (
                    "content",
                    djangors_orm::expr::Value::Text("First".to_string()),
                ),
            ],
        )
        .await
        .unwrap();
        let _ = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                (
                    "name",
                    djangors_orm::expr::Value::Text("Banana".to_string()),
                ),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Second".to_string()),
                ),
            ],
        )
        .await
        .unwrap();
        let _ = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                (
                    "name",
                    djangors_orm::expr::Value::Text("Apricot".to_string()),
                ),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Third".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        // 4. search_fields configured, ?q= matching substring (case-insensitive)
        let site_search = AdminSite::new();
        site_search.register_with::<ModelD>(ModelAdminConfig {
            list_display: None,
            search_fields: Some(&["name"]),
            ..Default::default()
        });
        let router_search = Router::new().mount("/admin", site_search.urls());

        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modeld/?q=ap"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router_search.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Apple"));
        assert!(body.contains("Apricot"));
        assert!(!body.contains("Banana"));
        assert!(body.contains("Total: 2."));

        // 5. search_fields configured but ?q= omitted
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modeld/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router_search.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Apple"));
        assert!(body.contains("Banana"));
        assert!(body.contains("Apricot"));
        assert!(body.contains("Total: 3."));

        // 6. No search_fields configured but ?q=something passed -> q is ignored, all show
        let site_nosearch = AdminSite::new();
        site_nosearch.register_with::<ModelD>(ModelAdminConfig::default());
        let router_nosearch = Router::new().mount("/admin", site_nosearch.urls());

        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modeld/?q=Banana"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router_nosearch.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Apple"));
        assert!(body.contains("Banana"));
        assert!(body.contains("Apricot"));
        assert!(body.contains("Total: 3."));

        // Clean up
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_list_filter() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop/create tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_e")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        let _ = sqlx::query(
            "CREATE TABLE test_model_e (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL
            )",
        )
        .execute(db.pool())
        .await;

        let _ = sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await;

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        // 1. register_with with list_filter: Some(&["is_active"]) on a model with a Boolean field -> registers successfully
        let site = AdminSite::new();
        site.register_with::<ModelE>(ModelAdminConfig {
            list_filter: Some(&["is_active"]),
            ..Default::default()
        });

        // Insert some rows to verify matching logic later
        let _ = djangors_orm::queryset::QuerySet::<ModelE>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Alice".to_string())),
                ("is_active", djangors_orm::expr::Value::Bool(true)),
                ("is_staff", djangors_orm::expr::Value::Bool(false)),
            ],
        )
        .await
        .unwrap();

        let _ = djangors_orm::queryset::QuerySet::<ModelE>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Bob".to_string())),
                ("is_active", djangors_orm::expr::Value::Bool(true)),
                ("is_staff", djangors_orm::expr::Value::Bool(true)),
            ],
        )
        .await
        .unwrap();

        let _ = djangors_orm::queryset::QuerySet::<ModelE>::insert_raw(
            &db,
            vec![
                (
                    "name",
                    djangors_orm::expr::Value::Text("Charlie".to_string()),
                ),
                ("is_active", djangors_orm::expr::Value::Bool(false)),
                ("is_staff", djangors_orm::expr::Value::Bool(false)),
            ],
        )
        .await
        .unwrap();

        let router = Router::new().mount("/admin", site.urls());
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modele/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();

        // contains the filter UI (All/Yes/No links for that field)
        assert!(body.contains("Filter by is_active:"));
        assert!(body.contains("<a href=\"\">All</a>"));
        assert!(body.contains("<a href=\"?is_active=true\">Yes</a>"));
        assert!(body.contains("<a href=\"?is_active=false\">No</a>"));

        // 2. register_with with list_filter naming a field that doesn't exist -> panics
        let site2 = AdminSite::new();
        let res2 = std::panic::catch_unwind(|| {
            site2.register_with::<ModelE>(ModelAdminConfig {
                list_filter: Some(&["nonexistent"]),
                ..Default::default()
            });
        });
        assert!(res2.is_err());

        // 3. register_with with list_filter naming a real but non-Boolean field -> panics
        let site3 = AdminSite::new();
        let res3 = std::panic::catch_unwind(|| {
            site3.register_with::<ModelE>(ModelAdminConfig {
                list_filter: Some(&["name"]),
                ..Default::default()
            });
        });
        assert!(res3.is_err());

        // 4. GET with ?is_active=true -> only the matching rows appear, total count is correct
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req_filter = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modele/?is_active=true"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_filter = router.handle(req_filter).await.unwrap();
        assert_eq!(res_filter.status(), StatusCode::OK);
        let body_filter = String::from_utf8(res_filter.body().to_vec()).unwrap();
        assert!(body_filter.contains("Alice"));
        assert!(body_filter.contains("Bob"));
        assert!(!body_filter.contains("Charlie"));
        assert!(body_filter.contains("Total: 2."));

        // 5. Same setup, GET with ?is_active=true&q=<term> (both active, assuming search_fields configured)
        let site_both = AdminSite::new();
        site_both.register_with::<ModelE>(ModelAdminConfig {
            search_fields: Some(&["name"]),
            list_filter: Some(&["is_active"]),
            ..Default::default()
        });
        let router_both = Router::new().mount("/admin", site_both.urls());

        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req_both = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modele/?is_active=true&q=Bob"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_both = router_both.handle(req_both).await.unwrap();
        assert_eq!(res_both.status(), StatusCode::OK);
        let body_both = String::from_utf8(res_both.body().to_vec()).unwrap();
        assert!(!body_both.contains("Alice"));
        assert!(body_both.contains("Bob"));
        assert!(!body_both.contains("Charlie"));
        assert!(body_both.contains("Total: 1."));

        // 6. GET with a query parameter matching a field name that is real on the model but NOT in list_filter -> ignored entirely
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req_ignored = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modele/?is_staff=true"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_ignored = router.handle(req_ignored).await.unwrap();
        assert_eq!(res_ignored.status(), StatusCode::OK);
        let body_ignored = String::from_utf8(res_ignored.body().to_vec()).unwrap();
        assert!(body_ignored.contains("Alice"));
        assert!(body_ignored.contains("Bob"));
        assert!(body_ignored.contains("Charlie"));
        assert!(body_ignored.contains("Total: 3."));

        // Clean up
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_e")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_part6_3_bulk_delete() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop/create tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        let _ = sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await;

        let _ = sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await;

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let non_staff = User {
            id: 0,
            username: "non_staff".to_string(),
            email: "non_staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        let session_non_staff = djangors_sessions::Session::new_empty();
        session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);

        let site = AdminSite::new();
        site.register::<ModelA>();
        let router = Router::new().mount("/admin", site.urls());

        // Insert 3 rows
        let pk1 = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
            &db,
            vec![("name", djangors_orm::expr::Value::Text("First".to_string()))],
        )
        .await
        .unwrap();

        let pk2 = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
            &db,
            vec![(
                "name",
                djangors_orm::expr::Value::Text("Second".to_string()),
            )],
        )
        .await
        .unwrap();

        let pk3 = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
            &db,
            vec![("name", djangors_orm::expr::Value::Text("Third".to_string()))],
        )
        .await
        .unwrap();

        // 1. Insert 3 rows. POST to bulk-delete/ with selected=<pk1>&selected=<pk2> (no confirm) ->
        //    200, response body lists both selected objects' display values, does NOT mention the third
        //    (unselected) row, and both rows still exist in the DB afterward.
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
            headers.clone(),
            Bytes::from(format!("selected={}&selected={}", pk1, pk2)),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));

        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("First"));
        assert!(body.contains("Second"));
        assert!(!body.contains("Third"));

        // verify rows still exist
        let check1 = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(id = pk1))
            .unwrap()
            .get(&db)
            .await;
        let check2 = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(id = pk2))
            .unwrap()
            .get(&db)
            .await;
        let check3 = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(id = pk3))
            .unwrap()
            .get(&db)
            .await;
        assert!(check1.is_ok());
        assert!(check2.is_ok());
        assert!(check3.is_ok());

        // 2. POST the same selected=<pk1>&selected=<pk2> again, this time WITH confirm=1 ->
        //    302 redirect to the changelist, both rows actually gone from the DB, the third (unselected) row still present.
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
            headers.clone(),
            Bytes::from(format!("selected={}&selected={}&confirm=1", pk1, pk2)),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));

        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(
            res.headers().get("location").unwrap().to_str().unwrap(),
            "/admin_test/modela/"
        );

        let check1 = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(id = pk1))
            .unwrap()
            .get(&db)
            .await;
        let check2 = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(id = pk2))
            .unwrap()
            .get(&db)
            .await;
        let check3 = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(id = pk3))
            .unwrap()
            .get(&db)
            .await;
        assert!(check1.is_err());
        assert!(check2.is_err());
        assert!(check3.is_ok());

        // 3. POST with no selected values at all (empty selection) -> redirects without error, nothing deleted.
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
            headers.clone(),
            Bytes::from(""),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));

        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(
            res.headers().get("location").unwrap().to_str().unwrap(),
            "/admin_test/modela/"
        );

        let check3 = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(id = pk3))
            .unwrap()
            .get(&db)
            .await;
        assert!(check3.is_ok());

        // 4. POST with a selected value that isn't a valid integer -> 400 Bad Request.
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
            headers.clone(),
            Bytes::from("selected=abc"),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));

        let res = router.handle(req).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().status_code(), StatusCode::BAD_REQUEST);

        // 5. Unauthenticated / non-staff POST -> 401 / 403.
        // Unauthenticated:
        let req_unauth = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
            headers.clone(),
            Bytes::from("selected=1"),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_unauth = router.handle(req_unauth).await;
        assert!(res_unauth.is_err());
        assert_eq!(
            res_unauth.unwrap_err().status_code(),
            StatusCode::UNAUTHORIZED
        );

        // Non-staff:
        let mut ext = Extensions::new();
        ext.insert(session_non_staff.clone());
        let req_nonstaff = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
            headers,
            Bytes::from("selected=1"),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_nonstaff = router.handle(req_nonstaff).await;
        assert!(res_nonstaff.is_err());
        assert_eq!(
            res_nonstaff.unwrap_err().status_code(),
            StatusCode::FORBIDDEN
        );

        // Clean up
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_part6_4_date_hierarchy() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop/create tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_f")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        let _ = sqlx::query(
            "CREATE TABLE test_model_f (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(db.pool())
        .await;

        let _ = sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await;

        use chrono::{TimeZone, Utc};

        let now = Utc::now();
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        let dt1 = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).single().unwrap();
        let dt2 = Utc.with_ymd_and_hms(2026, 3, 10, 0, 0, 0).single().unwrap();
        let dt3 = Utc.with_ymd_and_hms(2026, 3, 20, 0, 0, 0).single().unwrap();
        let dt4 = Utc.with_ymd_and_hms(2025, 11, 5, 0, 0, 0).single().unwrap();

        let _ = djangors_orm::queryset::QuerySet::<ModelF>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Row1".to_string())),
                ("created_at", djangors_orm::expr::Value::DateTime(dt1)),
            ],
        )
        .await
        .unwrap();

        let _ = djangors_orm::queryset::QuerySet::<ModelF>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Row2".to_string())),
                ("created_at", djangors_orm::expr::Value::DateTime(dt2)),
            ],
        )
        .await
        .unwrap();

        let _ = djangors_orm::queryset::QuerySet::<ModelF>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Row3".to_string())),
                ("created_at", djangors_orm::expr::Value::DateTime(dt3)),
            ],
        )
        .await
        .unwrap();

        let _ = djangors_orm::queryset::QuerySet::<ModelF>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Row4".to_string())),
                ("created_at", djangors_orm::expr::Value::DateTime(dt4)),
            ],
        )
        .await
        .unwrap();

        // Register site
        let site = AdminSite::new();
        site.register_with::<ModelF>(ModelAdminConfig {
            date_hierarchy: Some("created_at"),
            ..Default::default()
        });

        let router = Router::new().mount("/admin", site.urls());

        // Test 1: GET the changelist with no year/month/day -> 200
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req1 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modelf/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res1 = router.handle(req1).await.unwrap();
        assert_eq!(res1.status(), StatusCode::OK);
        let body1 = String::from_utf8(res1.body().to_vec()).unwrap();
        assert!(body1.contains("2025"));
        assert!(body1.contains("2026"));
        assert!(!body1.contains("January"));
        assert!(!body1.contains("March"));

        // Test 2: GET with ?year=2026 -> 200, only contains Row1, Row2, Row3
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req2 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modelf/?year=2026"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res2 = router.handle(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::OK);
        let body2 = String::from_utf8(res2.body().to_vec()).unwrap();
        assert!(body2.contains("Row1"));
        assert!(body2.contains("Row2"));
        assert!(body2.contains("Row3"));
        assert!(!body2.contains("Row4"));
        assert!(body2.contains("January"));
        assert!(body2.contains("March"));
        assert!(!body2.contains("February"));

        // Test 3: GET with ?year=2026&month=3 -> 200, contains Row2 and Row3
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req3 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modelf/?year=2026&month=3"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res3 = router.handle(req3).await.unwrap();
        assert_eq!(res3.status(), StatusCode::OK);
        let body3 = String::from_utf8(res3.body().to_vec()).unwrap();
        assert!(!body3.contains("Row1"));
        assert!(body3.contains("Row2"));
        assert!(body3.contains("Row3"));
        assert!(body3.contains(">10<"));
        assert!(body3.contains(">20<"));

        // Test 4: GET with ?year=2026&month=3&day=10 -> 200, contains only Row2
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req4 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modelf/?year=2026&month=3&day=10"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res4 = router.handle(req4).await.unwrap();
        assert_eq!(res4.status(), StatusCode::OK);
        let body4 = String::from_utf8(res4.body().to_vec()).unwrap();
        assert!(!body4.contains("Row1"));
        assert!(body4.contains("Row2"));
        assert!(!body4.contains("Row3"));

        // Test 5: GET with ?year=2026&o=name -> 200, sort header contains year=2026
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req5 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modelf/?year=2026&o=name"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res5 = router.handle(req5).await.unwrap();
        assert_eq!(res5.status(), StatusCode::OK);
        let body5 = String::from_utf8(res5.body().to_vec()).unwrap();
        assert!(body5.contains("year=2026"));

        // Test 6: GET with invalid ?month=13 -> 400 Bad Request
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req6 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modelf/?year=2026&month=13"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res6 = router.handle(req6).await;
        assert!(res6.is_err());
        assert_eq!(res6.unwrap_err().status_code(), StatusCode::BAD_REQUEST);

        // Test 7: Registering a model with date_hierarchy: Some("name") (non-DateTime) -> panics
        let site_panic = AdminSite::new();
        let res7 = std::panic::catch_unwind(|| {
            site_panic.register_with::<ModelD>(ModelAdminConfig {
                date_hierarchy: Some("name"),
                ..Default::default()
            });
        });
        assert!(res7.is_err());

        // Clean up
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_f")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_phase5_part6_5_list_editable() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop/create tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_e")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        let _ = sqlx::query(
            "CREATE TABLE test_model_d (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await;

        let _ = sqlx::query(
            "CREATE TABLE test_model_e (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL
            )",
        )
        .execute(db.pool())
        .await;

        let _ = sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await;

        use chrono::Utc;
        let now = Utc::now();
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        let pk1 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Row1".to_string())),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Content1".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        let pk2 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Row2".to_string())),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Content2".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        // 1. Register ModelD and check GET changelist -> 200, input field, save button
        let site = AdminSite::new();
        site.register_with::<ModelD>(ModelAdminConfig {
            list_display: Some(&["name", "content"]),
            list_editable: Some(&["content"]),
            ..Default::default()
        });

        let router = Router::new().mount("/admin", site.urls());

        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req1 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modeld/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res1 = router.handle(req1).await.unwrap();
        assert_eq!(res1.status(), StatusCode::OK);
        let body1 = String::from_utf8(res1.body().to_vec()).unwrap();
        assert!(body1.contains(&format!("name=\"edit-{}-content\"", pk1)));
        assert!(!body1.contains(&format!("name=\"edit-{}-name\"", pk1)));
        assert!(body1.contains("formaction=\"save-changelist/\""));

        // 2. Registration-validation panics
        // (a) list_editable naming a field not in list_display
        let site_panic = AdminSite::new();
        let res_a = std::panic::catch_unwind(|| {
            site_panic.register_with::<ModelD>(ModelAdminConfig {
                list_display: Some(&["name"]),
                list_editable: Some(&["content"]),
                ..Default::default()
            });
        });
        assert!(res_a.is_err());

        // (b) list_editable naming list_display's first column
        let res_b = std::panic::catch_unwind(|| {
            site_panic.register_with::<ModelD>(ModelAdminConfig {
                list_display: Some(&["name", "content"]),
                list_editable: Some(&["name"]),
                ..Default::default()
            });
        });
        assert!(res_b.is_err());

        // (c) list_editable naming a Boolean field
        let res_c = std::panic::catch_unwind(|| {
            site_panic.register_with::<ModelE>(ModelAdminConfig {
                list_display: Some(&["name", "is_active"]),
                list_editable: Some(&["is_active"]),
                ..Default::default()
            });
        });
        assert!(res_c.is_err());

        // (d) list_editable naming a nonexistent field
        let res_d = std::panic::catch_unwind(|| {
            site_panic.register_with::<ModelD>(ModelAdminConfig {
                list_display: Some(&["name", "content"]),
                list_editable: Some(&["nonexistent"]),
                ..Default::default()
            });
        });
        assert!(res_d.is_err());

        // 3. POST save-changelist/ with edit-<pk1>-content=Updated1 for one row -> 302 redirect
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );

        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req3 = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
            headers.clone(),
            Bytes::from(format!("edit-{}-content=Updated1", pk1)),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res3 = router.handle(req3).await.unwrap();
        assert_eq!(res3.status(), StatusCode::FOUND);

        // Verify updated in DB
        let check1 = djangors_orm::queryset::QuerySet::<ModelD>::new()
            .filter(djangors_orm::q!(id = pk1))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        assert_eq!(check1.content, "Updated1");

        // 4. POST with edits for TWO different pks in the same request (edit-<pk1>-content=A and edit-<pk2>-content=B) -> 302 redirect, BOTH rows updated correctly
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req4 = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
            headers.clone(),
            Bytes::from(format!("edit-{}-content=A&edit-{}-content=B", pk1, pk2)),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res4 = router.handle(req4).await.unwrap();
        assert_eq!(res4.status(), StatusCode::FOUND);

        let check1 = djangors_orm::queryset::QuerySet::<ModelD>::new()
            .filter(djangors_orm::q!(id = pk1))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        assert_eq!(check1.content, "A");
        let check2 = djangors_orm::queryset::QuerySet::<ModelD>::new()
            .filter(djangors_orm::q!(id = pk2))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        assert_eq!(check2.content, "B");

        // 5. POST with edit-<pk>-content= (empty string) where content is non-nullable -> 200 (not a redirect), response body mentions content in error message, and DB row's content is UNCHANGED
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req5 = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
            headers.clone(),
            Bytes::from(format!("edit-{}-content=", pk1)),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res5 = router.handle(req5).await.unwrap();
        assert_eq!(res5.status(), StatusCode::OK);
        let body5 = String::from_utf8(res5.body().to_vec()).unwrap();
        assert!(body5.contains("content"));
        // content is still "A"
        let check1 = djangors_orm::queryset::QuerySet::<ModelD>::new()
            .filter(djangors_orm::q!(id = pk1))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        assert_eq!(check1.content, "A");

        // 6. POST with an edit-<pk>-name=Malicious key, where name is NOT in list_editable -> redirect, DB row's name is UNCHANGED
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req6 = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
            headers.clone(),
            Bytes::from(format!("edit-{}-name=Malicious", pk1)),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res6 = router.handle(req6).await.unwrap();
        assert_eq!(res6.status(), StatusCode::FOUND);
        let check1 = djangors_orm::queryset::QuerySet::<ModelD>::new()
            .filter(djangors_orm::q!(id = pk1))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        assert_eq!(check1.name, "Row1");

        // 7. POST with a non-numeric pk in an edit- key (e.g. edit-abc-content=X) -> 400 Bad Request
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req7 = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
            headers.clone(),
            Bytes::from("edit-abc-content=X"),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res7 = router.handle(req7).await;
        assert!(res7.is_err());
        assert_eq!(res7.unwrap_err().status_code(), StatusCode::BAD_REQUEST);

        // 8. Unauthenticated / non-staff POST to save-changelist/ -> 401 / 403
        // Unauthenticated:
        let req8_unauth = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
            headers.clone(),
            Bytes::from(format!("edit-{}-content=Unauth", pk1)),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res8_unauth = router.handle(req8_unauth).await;
        assert!(res8_unauth.is_err());
        assert!(matches!(
            res8_unauth.unwrap_err().status_code(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));

        // Clean up
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_e")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_phase5_part6_6_csv_export() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop/create tables
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        let _ = sqlx::query(
            "CREATE TABLE test_model_d (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await;

        let _ = sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await;

        use chrono::Utc;
        let now = Utc::now();
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        // Insert 3 ModelD rows
        let _pk1 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Apple".to_string())),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Fruit".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        let _pk2 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                (
                    "name",
                    djangors_orm::expr::Value::Text("Banana".to_string()),
                ),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Yellow".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        let _pk3 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                (
                    "name",
                    djangors_orm::expr::Value::Text("Cherry".to_string()),
                ),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Red".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        // 2. Register with default config
        let site = AdminSite::new();
        site.register_with::<ModelD>(ModelAdminConfig {
            search_fields: Some(&["name", "content"]),
            ..Default::default()
        });

        let router = Router::new().mount("/admin", site.urls());

        // GET export-csv/ with no params
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req1 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modeld/export-csv/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res1 = router.handle(req1).await.unwrap();
        assert_eq!(res1.status(), StatusCode::OK);
        let content_type = res1
            .headers()
            .get("Content-Type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(content_type.starts_with("text/csv"));
        let content_disp = res1
            .headers()
            .get("Content-Disposition")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(content_disp.contains("attachment; filename=\"modeld.csv\""));

        let body1 = String::from_utf8(res1.body().to_vec()).unwrap();
        let lines: Vec<&str> = body1.split("\r\n").filter(|s| !s.is_empty()).collect();
        assert_eq!(lines.len(), 4); // header + 3 rows
        assert_eq!(lines[0], "id,name,content");
        assert!(body1.contains("Apple"));
        assert!(body1.contains("Banana"));
        assert!(body1.contains("Cherry"));

        // 3. GET export-csv/?q=Banana
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req2 = Request::new(
            Method::GET,
            "/admin/admin_test/modeld/export-csv/?q=Banana"
                .parse::<Uri>()
                .unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res2 = router.handle(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::OK);
        let body2 = String::from_utf8(res2.body().to_vec()).unwrap();
        let lines2: Vec<&str> = body2.split("\r\n").filter(|s| !s.is_empty()).collect();
        assert_eq!(lines2.len(), 2); // header + 1 row
        assert!(!body2.contains("Apple"));
        assert!(body2.contains("Banana"));
        assert!(!body2.contains("Cherry"));

        // 4. Insert row with comma:
        let _pk4 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                (
                    "name",
                    djangors_orm::expr::Value::Text("Date, Fruit".to_string()),
                ),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Sweet, sticky".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req3 = Request::new(
            Method::GET,
            "/admin/admin_test/modeld/export-csv/?q=sticky"
                .parse::<Uri>()
                .unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res3 = router.handle(req3).await.unwrap();
        assert_eq!(res3.status(), StatusCode::OK);
        let body3 = String::from_utf8(res3.body().to_vec()).unwrap();
        assert!(body3.contains("\"Date, Fruit\""));
        assert!(body3.contains("\"Sweet, sticky\""));

        // 5. Unauthenticated
        let req4 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modeld/export-csv/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res4 = router.handle(req4).await;
        assert!(res4.is_err());
        assert!(matches!(
            res4.unwrap_err().status_code(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));

        // Clean up
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_phase5_computed_columns() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        let _ = sqlx::query(
            "CREATE TABLE test_model_d (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await;

        let _ = sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await;

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_computed".to_string(),
            email: "staff_computed@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        fn format_combo(values: &[(&'static str, djangors_orm::expr::Value)]) -> String {
            let name = values
                .iter()
                .find(|(n, _)| *n == "name")
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            let content = values
                .iter()
                .find(|(n, _)| *n == "content")
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            format!("{} - {}", name, content)
        }

        let _pk = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("Alice".to_string())),
                (
                    "content",
                    djangors_orm::expr::Value::Text("Hello".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        let site = AdminSite::new();
        site.register_with::<ModelD>(ModelAdminConfig {
            list_display: Some(&["name", "combo"]),
            computed_columns: Some(&[("combo", format_combo)]),
            ..Default::default()
        });

        let router = Router::new().mount("/admin", site.urls());
        let mut ext = Extensions::new();
        ext.insert(session_staff);
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modeld/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Alice - Hello"));

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_bulk_action_no_confirm() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE djangors_admin_log (
                id BIGSERIAL PRIMARY KEY,
                user_id BIGINT NOT NULL,
                action_time TIMESTAMPTZ NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_action".to_string(),
            email: "staff_action@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        fn touch_action<'a>(
            _db: &'a djangors_db::Database,
            _pks: &'a [i64],
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), DjangorsError>> + Send + 'a>,
        > {
            Box::pin(async move { Ok(()) })
        }

        let pk = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
            &db,
            vec![(
                "name",
                djangors_orm::expr::Value::Text("ActionTarget".to_string()),
            )],
        )
        .await
        .unwrap();

        let site = AdminSite::new();
        site.register_with::<ModelA>(ModelAdminConfig {
            actions: Some(&[AdminAction {
                name: "custom_touch",
                label: "Touch",
                requires_confirm: false,
                handler: touch_action,
            }]),
            ..Default::default()
        });

        let router = Router::new().mount("/admin", site.urls());

        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );

        let mut ext = Extensions::new();
        ext.insert(session_staff);
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-action/"),
            headers,
            Bytes::from(format!("action=custom_touch&selected={}", pk)),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(
            res.headers().get("location").unwrap().to_str().unwrap(),
            "/admin_test/modela/"
        );

        let logs: Vec<LogEntry> = sqlx::query_as(sqlx::AssertSqlSafe(
            "SELECT id, user_id, action_time, app_label, model_name, object_id, object_repr, action_flag, change_message, field_diff FROM djangors_admin_log ORDER BY id ASC",
        ))
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].action_flag, ACTION_CHANGE);
        assert_eq!(logs[0].change_message, "Touch");

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_bulk_action_with_confirm() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE djangors_admin_log (
                id BIGSERIAL PRIMARY KEY,
                user_id BIGINT NOT NULL,
                action_time TIMESTAMPTZ NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_confirm".to_string(),
            email: "staff_confirm@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);

        fn confirm_action<'a>(
            _db: &'a djangors_db::Database,
            _pks: &'a [i64],
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), DjangorsError>> + Send + 'a>,
        > {
            Box::pin(async move { Ok(()) })
        }

        let pk = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
            &db,
            vec![(
                "name",
                djangors_orm::expr::Value::Text("ConfirmTarget".to_string()),
            )],
        )
        .await
        .unwrap();

        let site = AdminSite::new();
        site.register_with::<ModelA>(ModelAdminConfig {
            actions: Some(&[AdminAction {
                name: "needs_confirmation",
                label: "Needs Confirm",
                requires_confirm: true,
                handler: confirm_action,
            }]),
            ..Default::default()
        });

        let router = Router::new().mount("/admin", site.urls());

        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );

        // Step 1: POST without confirm=1 -> should show confirm page
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-action/"),
            headers.clone(),
            Bytes::from(format!("action=needs_confirmation&selected={}", pk)),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Needs Confirm"));
        assert!(body.contains("ConfirmTarget"));
        assert!(body.contains("confirm"));

        // Step 2: POST with confirm=1 -> should execute and redirect
        let mut ext = Extensions::new();
        ext.insert(session_staff);
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-action/"),
            headers,
            Bytes::from(format!(
                "action=needs_confirmation&selected={}&confirm=1",
                pk
            )),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(
            res.headers().get("location").unwrap().to_str().unwrap(),
            "/admin_test/modela/"
        );

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_permissions_enforcement() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop tables
        let drop_tables = [
            "auth_user_permissions",
            "auth_group_permissions",
            "auth_user_groups",
            "auth_group",
            "auth_permission",
            "auth_user",
            "test_model_a",
            "test_model_b",
        ];
        for table in drop_tables {
            let _ = sqlx::query(djangors_orm::sqlx::AssertSqlSafe(format!(
                "DROP TABLE IF EXISTS {}",
                table
            )))
            .execute(db.pool())
            .await;
        }

        // Create auth_user
        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL UNIQUE,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Create auth_permission
        sqlx::query(
            "CREATE TABLE auth_permission (
                id BIGSERIAL PRIMARY KEY,
                codename VARCHAR(255) NOT NULL UNIQUE,
                name VARCHAR(255) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Create auth_group
        sqlx::query(
            "CREATE TABLE auth_group (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(150) NOT NULL UNIQUE
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Create auth_user_groups
        sqlx::query(
            "CREATE TABLE auth_user_groups (
                id BIGSERIAL PRIMARY KEY,
                \"user\" BIGINT NOT NULL,
                \"group\" BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Create auth_group_permissions
        sqlx::query(
            "CREATE TABLE auth_group_permissions (
                id BIGSERIAL PRIMARY KEY,
                \"group\" BIGINT NOT NULL,
                permission BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Create auth_user_permissions
        sqlx::query(
            "CREATE TABLE auth_user_permissions (
                id BIGSERIAL PRIMARY KEY,
                \"user\" BIGINT NOT NULL,
                permission BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Create test_model_a and test_model_b
        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_b (
                id BIGSERIAL PRIMARY KEY,
                title TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();

        // Create staff, non-superuser user
        let staff_user = User {
            id: 0,
            username: "staff_user".to_string(),
            email: "staff_user@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        // Setup AdminSite
        let site = AdminSite::new();
        site.register::<ModelA>();
        site.register::<ModelB>();
        let router = Router::new().mount("/admin", site.urls());

        // Create session
        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff_user.id);
        let mut ext = Extensions::new();
        ext.insert(session.clone());

        // Test 1: A staff, non-superuser user with no permissions at all -> GET changelist for ModelA -> 403 Forbidden
        let req1 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res1 = router.handle(req1).await;
        assert!(res1.is_err());
        assert_eq!(res1.unwrap_err().status_code(), StatusCode::FORBIDDEN);

        // Test 2: The same user, after being granted the `view` permission for that specific model -> GET changelist -> 200
        let codename_view_a = "admin_test.view_modela";
        let perm_id: i64 = sqlx::query_scalar(
            "INSERT INTO auth_permission (codename, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(codename_view_a)
        .bind("Can view modela")
        .fetch_one(db.pool())
        .await
        .unwrap();

        // Link user to permission
        sqlx::query("INSERT INTO auth_user_permissions (\"user\", permission) VALUES ($1, $2)")
            .bind(staff_user.id)
            .bind(perm_id)
            .execute(db.pool())
            .await
            .unwrap();

        let req2 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res2 = router.handle(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::OK);

        // Test 3: The same permissioned-for-view-only user -> POST to that model's add/ route -> 403
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req3 = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/add/"),
            headers,
            Bytes::from("name=TestingAdd"),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res3 = router.handle(req3).await;
        assert!(res3.is_err());
        assert_eq!(res3.unwrap_err().status_code(), StatusCode::FORBIDDEN);

        // Clean up direct permission for next tests
        sqlx::query("DELETE FROM auth_user_permissions")
            .execute(db.pool())
            .await
            .unwrap();

        // Test 4: A staff, non-superuser user granted view via group membership -> GET changelist -> 200
        let group_id: i64 =
            sqlx::query_scalar("INSERT INTO auth_group (name) VALUES ($1) RETURNING id")
                .bind("Editors")
                .fetch_one(db.pool())
                .await
                .unwrap();

        // Link group to permission
        sqlx::query("INSERT INTO auth_group_permissions (\"group\", permission) VALUES ($1, $2)")
            .bind(group_id)
            .bind(perm_id)
            .execute(db.pool())
            .await
            .unwrap();

        // Link user to group
        sqlx::query("INSERT INTO auth_user_groups (\"user\", \"group\") VALUES ($1, $2)")
            .bind(staff_user.id)
            .bind(group_id)
            .execute(db.pool())
            .await
            .unwrap();

        let req4 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let req_res4 = router.handle(req4).await.unwrap();
        assert_eq!(req_res4.status(), StatusCode::OK);

        // Test 5: admin_index with a non-superuser staff user who has view permission for exactly one of two registered test models -> body contains link for ModelA but not ModelB
        let req5 = Request::new(
            Method::GET,
            Uri::from_static("/admin/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res5 = router.handle(req5).await.unwrap();
        assert_eq!(res5.status(), StatusCode::OK);
        let body5 = String::from_utf8(res5.body().to_vec()).unwrap();
        assert!(body5.contains("admin_test/modela/"));
        assert!(!body5.contains("admin_test/modelb/"));

        // Clean up group membership/permissions to show that superuser succeeds without any records
        sqlx::query("DELETE FROM auth_user_groups")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("DELETE FROM auth_group_permissions")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("DELETE FROM auth_group")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("DELETE FROM auth_permission")
            .execute(db.pool())
            .await
            .unwrap();

        // Test 6: A superuser -> every action succeeds without needing any auth_permission/auth_user_permissions rows at all
        let superuser = User {
            id: 0,
            username: "superuser".to_string(),
            email: "superuser@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let super_session = djangors_sessions::Session::new_empty();
        super_session.set(SESSION_USER_ID_KEY, superuser.id);
        let mut super_ext = Extensions::new();
        super_ext.insert(super_session);

        // GET changelist succeeds
        let req6_get = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(super_ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res6_get = router.handle(req6_get).await.unwrap();
        assert_eq!(res6_get.status(), StatusCode::OK);

        // POST add succeeds
        let mut super_headers = HeaderMap::new();
        super_headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req6_post = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/add/"),
            super_headers,
            Bytes::from("name=SuperuserAdd"),
        )
        .with_extensions(super_ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res6_post = router.handle(req6_post).await.unwrap();
        assert_eq!(res6_post.status(), StatusCode::FOUND);

        // Clean up
        for table in drop_tables {
            let _ = sqlx::query(djangors_orm::sqlx::AssertSqlSafe(format!(
                "DROP TABLE IF EXISTS {}",
                table
            )))
            .execute(db.pool())
            .await;
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_csrf_token_wiring() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Create auth_user and test tables if they don't exist
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_c (
                id BIGSERIAL PRIMARY KEY,
                parent BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_csrf".to_string(),
            email: "staff_csrf@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        // Save a test row
        sqlx::query("INSERT INTO test_model_a (id, name) VALUES (123, 'test_row')")
            .execute(db.pool())
            .await
            .unwrap();

        let site = AdminSite::new();
        site.register::<ModelA>();

        let router = Router::new().mount("/admin", site.urls());

        let test_token = "my-awesome-test-csrf-token-12345";
        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);

        let make_ext = || {
            let mut ext = Extensions::new();
            ext.insert(session.clone());
            ext.insert(djangors_core::middleware::CsrfToken(test_token.to_string()));
            ext
        };

        // 1. Changelist
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(make_ext())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("name=\"csrfmiddlewaretoken\""));
        assert!(body.contains(&format!("value=\"{}\"", test_token)));

        // 2. Add
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/add/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(make_ext())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("name=\"csrfmiddlewaretoken\""));
        assert!(body.contains(&format!("value=\"{}\"", test_token)));

        // 3. Change
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/123/change/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(make_ext())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("name=\"csrfmiddlewaretoken\""));
        assert!(body.contains(&format!("value=\"{}\"", test_token)));

        // 4. Delete Confirm
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/123/delete/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(make_ext())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("name=\"csrfmiddlewaretoken\""));
        assert!(body.contains(&format!("value=\"{}\"", test_token)));

        // 5. Bulk Delete Confirm (POST to bulk-delete/ with selected pk)
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
            headers,
            Bytes::from("selected=123"),
        )
        .with_extensions(make_ext())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("name=\"csrfmiddlewaretoken\""));
        assert!(body.contains(&format!("value=\"{}\"", test_token)));

        // Cleanup
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    async fn test_favicon_serving() {
        let router = favicon_routes(Router::new());

        let test_cases = vec![
            ("/favicon.ico", "image/x-icon"),
            ("/favicon-16x16.png", "image/png"),
            ("/favicon-32x32.png", "image/png"),
            ("/apple-touch-icon.png", "image/png"),
            ("/android-chrome-192x192.png", "image/png"),
            ("/android-chrome-512x512.png", "image/png"),
            ("/manifest.json", "application/json; charset=utf-8"),
        ];

        for (path, expected_ct) in test_cases {
            let req = Request::new(
                Method::GET,
                Uri::try_from(path).unwrap(),
                HeaderMap::new(),
                Bytes::new(),
            );
            let res = router.handle(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "Failed for path: {}", path);
            assert_eq!(
                res.headers().get("content-type").unwrap().to_str().unwrap(),
                expected_ct
            );
            assert!(!res.body().is_empty());
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_branding_overrides() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // AdminSite with logo and accent color set
        let site = AdminSite::new()
            .with_logo_url("/static/my-logo.png")
            .with_accent_color("#ff0000");

        // AdminSite with defaults (no builders)
        let default_site = AdminSite::new();

        let router = Router::new()
            .mount("/admin-brand", site.urls())
            .mount("/admin-default", default_site.urls());

        // Create a staff user for authenticating
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_brand".to_string(),
            email: "staff_brand@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        // Assert branding is rendered for the customized site
        let req_brand = Request::new(
            Method::GET,
            Uri::from_static("/admin-brand/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_brand = router.handle(req_brand).await.unwrap();
        assert_eq!(res_brand.status(), StatusCode::OK);
        let body_brand = String::from_utf8(res_brand.body().to_vec()).unwrap();
        assert!(body_brand.contains("class=\"site-logo\""));
        assert!(body_brand.contains("src=\"/static/my-logo.png\""));
        assert!(body_brand.contains("<style>:root { --accent: #ff0000; }</style>"));

        // Assert branding is NOT rendered for the default site
        let req_default = Request::new(
            Method::GET,
            Uri::from_static("/admin-default/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_default = router.handle(req_default).await.unwrap();
        assert_eq!(res_default.status(), StatusCode::OK);
        let body_default = String::from_utf8(res_default.body().to_vec()).unwrap();
        assert!(!body_default.contains("class=\"site-logo\""));
        assert!(!body_default.contains("src=\"/static/my-logo.png\""));
        assert!(!body_default.contains("<style>:root { --accent:"));

        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_audit_log_add_change_delete() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE djangors_admin_log (
                id BIGSERIAL PRIMARY KEY,
                user_id BIGINT NOT NULL,
                action_time TIMESTAMPTZ NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_audit".to_string(),
            email: "staff_audit@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());

        let site = AdminSite::new();
        site.register::<ModelA>();
        let router = Router::new().mount("/admin", site.urls());

        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req_add = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/add/"),
            headers.clone(),
            Bytes::from("name=Row1"),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_add = router.handle(req_add).await.unwrap();
        assert_eq!(res_add.status(), StatusCode::FOUND);

        let row = djangors_orm::queryset::QuerySet::<ModelA>::new()
            .filter(djangors_orm::q!(name = "Row1"))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        let pk = row.id;

        let req_change = Request::new(
            Method::POST,
            Uri::try_from(format!("/admin/admin_test/modela/{}/change/", pk)).unwrap(),
            headers.clone(),
            Bytes::from("name=Row1Changed"),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_change = router.handle(req_change).await.unwrap();
        assert_eq!(res_change.status(), StatusCode::FOUND);

        let req_delete = Request::new(
            Method::POST,
            Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", pk)).unwrap(),
            headers,
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_delete = router.handle(req_delete).await.unwrap();
        assert_eq!(res_delete.status(), StatusCode::FOUND);

        let logs: Vec<LogEntry> = sqlx::query_as(sqlx::AssertSqlSafe(
            "SELECT id, user_id, action_time, app_label, model_name, object_id, object_repr, action_flag, change_message, field_diff FROM djangors_admin_log ORDER BY id ASC",
        ))
        .fetch_all(db.pool())
        .await
        .unwrap();

        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].action_flag, ACTION_ADDITION);
        assert_eq!(logs[0].object_repr, format!("ModelA object ({})", pk));
        assert_eq!(logs[1].action_flag, ACTION_CHANGE);
        assert_eq!(logs[1].object_repr, format!("ModelA object ({})", pk));
        assert_eq!(logs[2].action_flag, ACTION_DELETION);
        assert_eq!(logs[2].object_repr, format!("ModelA object ({})", pk));

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_audit_log_bulk_delete_and_list_editable() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_d (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE djangors_admin_log (
                id BIGSERIAL PRIMARY KEY,
                user_id BIGINT NOT NULL,
                action_time TIMESTAMPTZ NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_bulk".to_string(),
            email: "staff_bulk@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());

        let pk1 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("R1".to_string())),
                ("content", djangors_orm::expr::Value::Text("C1".to_string())),
            ],
        )
        .await
        .unwrap();

        let pk2 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("R2".to_string())),
                ("content", djangors_orm::expr::Value::Text("C2".to_string())),
            ],
        )
        .await
        .unwrap();

        let site = AdminSite::new();
        site.register_with::<ModelD>(ModelAdminConfig {
            list_display: Some(&["name", "content"]),
            list_editable: Some(&["content"]),
            ..Default::default()
        });
        let router = Router::new().mount("/admin", site.urls());

        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );

        let req_bulk = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modeld/bulk-delete/"),
            headers.clone(),
            Bytes::from(format!("selected={}&selected={}&confirm=1", pk1, pk2)),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_bulk = router.handle(req_bulk).await.unwrap();
        assert_eq!(res_bulk.status(), StatusCode::FOUND);

        let logs: Vec<LogEntry> = sqlx::query_as(sqlx::AssertSqlSafe(
            "SELECT id, user_id, action_time, app_label, model_name, object_id, object_repr, action_flag, change_message, field_diff FROM djangors_admin_log WHERE action_flag = 3 ORDER BY id ASC",
        ))
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(logs.len(), 2);

        let _ = sqlx::query("DELETE FROM djangors_admin_log")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DELETE FROM test_model_d")
            .execute(db.pool())
            .await;

        let pk3 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("R3".to_string())),
                ("content", djangors_orm::expr::Value::Text("C3".to_string())),
            ],
        )
        .await
        .unwrap();

        let pk4 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                ("name", djangors_orm::expr::Value::Text("R4".to_string())),
                ("content", djangors_orm::expr::Value::Text("C4".to_string())),
            ],
        )
        .await
        .unwrap();

        let req_edit = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
            headers,
            Bytes::from(format!(
                "edit-{}-content=UpdatedValid&edit-{}-content=",
                pk3, pk4
            )),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_edit = router.handle(req_edit).await.unwrap();
        assert_eq!(res_edit.status(), StatusCode::OK);

        let logs: Vec<LogEntry> = sqlx::query_as(sqlx::AssertSqlSafe(
            "SELECT id, user_id, action_time, app_label, model_name, object_id, object_repr, action_flag, change_message, field_diff FROM djangors_admin_log WHERE action_flag = 2 ORDER BY id ASC",
        ))
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].object_id, pk3);

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_index_recent_actions() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE djangors_admin_log (
                id BIGSERIAL PRIMARY KEY,
                user_id BIGINT NOT NULL,
                action_time TIMESTAMPTZ NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let user1 = User {
            id: 0,
            username: "staff_act1".to_string(),
            email: "staff_act1@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let user2 = User {
            id: 0,
            username: "staff_act2".to_string(),
            email: "staff_act2@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let _log1 = LogEntry {
            id: 0,
            user_id: user1.id,
            action_time: now - chrono::Duration::try_seconds(10).unwrap(),
            app_label: "admin_test".to_string(),
            model_name: "ModelA".to_string(),
            object_id: 100,
            object_repr: "ModelA object (100)".to_string(),
            action_flag: ACTION_ADDITION,
            change_message: "Added.".to_string(),
            field_diff: None,
        }
        .save(&db)
        .await
        .unwrap();

        let _log2 = LogEntry {
            id: 0,
            user_id: user1.id,
            action_time: now - chrono::Duration::try_seconds(5).unwrap(),
            app_label: "admin_test".to_string(),
            model_name: "ModelA".to_string(),
            object_id: 101,
            object_repr: "ModelA object (101)".to_string(),
            action_flag: ACTION_CHANGE,
            change_message: "Changed.".to_string(),
            field_diff: None,
        }
        .save(&db)
        .await
        .unwrap();

        let site = AdminSite::new();
        let router = Router::new().mount("/admin", site.urls());

        let session_user1 = djangors_sessions::Session::new_empty();
        session_user1.set(SESSION_USER_ID_KEY, user1.id);
        let mut ext1 = Extensions::new();
        ext1.insert(session_user1);

        let req_user1 = Request::new(
            Method::GET,
            Uri::from_static("/admin/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext1)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));

        let res_user1 = router.handle(req_user1).await.unwrap();
        assert_eq!(res_user1.status(), StatusCode::OK);
        let body1 = String::from_utf8(res_user1.body().to_vec()).unwrap();

        assert!(body1.contains("Recent actions"));
        assert!(body1.contains("ModelA object (100)"));
        assert!(body1.contains("ModelA object (101)"));

        let idx101 = body1.find("ModelA object (101)").unwrap();
        let idx100 = body1.find("ModelA object (100)").unwrap();
        assert!(idx101 < idx100);

        let session_user2 = djangors_sessions::Session::new_empty();
        session_user2.set(SESSION_USER_ID_KEY, user2.id);
        let mut ext2 = Extensions::new();
        ext2.insert(session_user2);

        let req_user2 = Request::new(
            Method::GET,
            Uri::from_static("/admin/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext2)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));

        let res_user2 = router.handle(req_user2).await.unwrap();
        assert_eq!(res_user2.status(), StatusCode::OK);
        let body2 = String::from_utf8(res_user2.body().to_vec()).unwrap();

        assert!(!body2.contains("Recent actions"));
        assert!(!body2.contains("ModelA object (100)"));
        assert!(!body2.contains("ModelA object (101)"));

        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_fieldsets() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_d (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_fieldsets".to_string(),
            email: "staff_fieldsets@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        let site = AdminSite::new();
        site.register_with::<ModelD>(ModelAdminConfig {
            fieldsets: Some(&[("Section A", &["name"]), ("Section B", &["content"])]),
            ..Default::default()
        });
        let router = Router::new().mount("/admin", site.urls());

        let req_add = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modeld/add/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req_add).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();

        assert!(body.contains("Section A"));
        assert!(body.contains("Section B"));

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_readonly_fields() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_d (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_ro".to_string(),
            email: "staff_ro@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        let site = AdminSite::new();
        site.register_with::<ModelD>(ModelAdminConfig {
            readonly_fields: Some(&["name"]),
            ..Default::default()
        });
        let router = Router::new().mount("/admin", site.urls());

        let row = ModelD {
            id: 0,
            name: "ro_test".to_string(),
            content: "ro_content".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let req_change = Request::new(
            Method::GET,
            Uri::try_from(format!("/admin/admin_test/modeld/{}/change/", row.id)).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req_change).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();

        assert!(body.contains("(readonly):"));

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_raw_id_fields() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_c (
                id BIGSERIAL PRIMARY KEY,
                parent BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_rawid".to_string(),
            email: "staff_rawid@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        let site = AdminSite::new();
        site.register_with::<ModelC>(ModelAdminConfig {
            raw_id_fields: Some(&["parent"]),
            ..Default::default()
        });
        let router = Router::new().mount("/admin", site.urls());

        let a = ModelA {
            id: 0,
            name: "raw_target".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _c = ModelC {
            id: 0,
            parent: djangors_orm::ForeignKey::new(a.id),
        }
        .save(&db)
        .await
        .unwrap();

        let req_changelist = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modelc/add/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req_changelist).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();

        assert!(body.contains("Look up"));

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_transitive_walk_delete() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_g")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_c (
                id BIGSERIAL PRIMARY KEY,
                parent BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_g (
                id BIGSERIAL PRIMARY KEY,
                parent BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_tw".to_string(),
            email: "staff_tw@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        let site = AdminSite::new();
        site.register::<ModelA>();
        site.register::<ModelC>();
        site.register::<ModelG>();
        let router = Router::new().mount("/admin", site.urls());

        let a = ModelA {
            id: 0,
            name: "transitive_target".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let c = ModelC {
            id: 0,
            parent: djangors_orm::ForeignKey::new(a.id),
        }
        .save(&db)
        .await
        .unwrap();
        let _g = ModelG {
            id: 0,
            parent: djangors_orm::ForeignKey::new(c.id),
        }
        .save(&db)
        .await
        .unwrap();

        let req_delete_get = Request::new(
            Method::GET,
            Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", a.id)).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req_delete_get).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();

        assert!(body.contains("ModelC"));
        assert!(body.contains("ModelG"));

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_c")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_g")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_protect_single_delete_400() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_p")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_p (
                id BIGSERIAL PRIMARY KEY,
                parent BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_prot".to_string(),
            email: "staff_prot@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        let site = AdminSite::new();
        site.register::<ModelA>();
        site.register::<ModelP>();
        let router = Router::new().mount("/admin", site.urls());

        let a = ModelA {
            id: 0,
            name: "protect_target".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _p = ModelP {
            id: 0,
            parent: djangors_orm::ForeignKey::new(a.id),
        }
        .save(&db)
        .await
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req_delete_post = Request::new(
            Method::POST,
            Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", a.id)).unwrap(),
            headers,
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req_delete_post).await;
        match res {
            Ok(r) => assert_eq!(r.status(), StatusCode::BAD_REQUEST),
            Err(e) => assert!(e.to_string().contains("protected")),
        }

        let still_exists: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(
            "SELECT COUNT(*) FROM test_model_a WHERE id = $1",
        ))
        .bind(a.id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(still_exists, 1);

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_p")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_protect_bulk_delete_skip() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_p")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_p (
                id BIGSERIAL PRIMARY KEY,
                parent BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_bulkp".to_string(),
            email: "staff_bulkp@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        let site = AdminSite::new();
        site.register::<ModelA>();
        site.register::<ModelP>();
        let router = Router::new().mount("/admin", site.urls());

        let a1 = ModelA {
            id: 0,
            name: "bulk_protected".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let a2 = ModelA {
            id: 0,
            name: "bulk_unprotected".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let _p = ModelP {
            id: 0,
            parent: djangors_orm::ForeignKey::new(a1.id),
        }
        .save(&db)
        .await
        .unwrap();

        let body = format!("confirm=1&selected={}&selected={}", a1.id, a2.id);
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req_bulk_delete = Request::new(
            Method::POST,
            Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
            headers,
            Bytes::from(body),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req_bulk_delete).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);

        let remaining: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe("SELECT COUNT(*) FROM test_model_a"))
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(remaining, 1);

        let still_alive: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(
            "SELECT COUNT(*) FROM test_model_a WHERE name = 'bulk_protected'",
        ))
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(still_alive, 1);

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_p")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_base_filter() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_bf".to_string(),
            email: "staff_bf@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let _a1 = ModelA {
            id: 0,
            name: "apple".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _a2 = ModelA {
            id: 0,
            name: "banana".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        let site = AdminSite::new();
        site.register_with::<ModelA>(ModelAdminConfig {
            search_fields: Some(&["name"]),
            base_filter: Some(djangors_orm::UnresolvedExpr::And(vec![
                djangors_orm::UnresolvedCompare {
                    field: "name",
                    value: djangors_orm::Value::Text("apple".to_string()),
                },
            ])),
            ..Default::default()
        });
        let router = Router::new().mount("/admin", site.urls());

        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/?q=apple"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();

        assert!(body.contains("apple"));
        assert!(!body.contains("banana"));

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_extra_route() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_er".to_string(),
            email: "staff_er@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        let extra = Router::new().get(
            "/my-custom/",
            |_req: Request, _params: PathParams| async move {
                Ok(Response::text(StatusCode::OK, "custom route works"))
            },
        );

        let site = AdminSite::new().extra_route(extra);
        site.register::<ModelA>();
        let router = Router::new().mount("/admin", site.urls());

        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/my-custom/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("custom route works"));

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_history_page() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(255) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE djangors_admin_log (
                id BIGSERIAL PRIMARY KEY,
                user_id BIGINT NOT NULL,
                action_time TIMESTAMPTZ NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_hist".to_string(),
            email: "staff_hist@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let a = ModelA {
            id: 0,
            name: "hist_test".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let _log = LogEntry {
            id: 0,
            user_id: staff.id,
            action_time: now,
            app_label: "admin_test".to_string(),
            model_name: "ModelA".to_string(),
            object_id: a.id,
            object_repr: format!("ModelA object ({})", a.id),
            action_flag: ACTION_ADDITION,
            change_message: "Created.".to_string(),
            field_diff: None,
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session);

        let site = AdminSite::new();
        site.register::<ModelA>();
        let router = Router::new().mount("/admin", site.urls());

        let req = Request::new(
            Method::GET,
            Uri::try_from(format!("/admin/admin_test/modela/{}/history/", a.id)).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();

        assert!(body.contains("History for ModelA"));
        assert!(body.contains("staff_hist"));
        assert!(body.contains("Created."));

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_phase5_groups_permissions_admin_ui() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let drop_tables = ["auth_user_groups", "auth_group", "auth_user"];
        for table in drop_tables {
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP TABLE IF EXISTS {}",
                table
            )))
            .execute(db.pool())
            .await;
        }

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE auth_group (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(150) NOT NULL UNIQUE
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE auth_user_groups (
                id BIGSERIAL PRIMARY KEY,
                \"user\" BIGINT NOT NULL,
                \"group\" BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_groups".to_string(),
            email: "staff_groups@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session = djangors_sessions::Session::new_empty();
        session.set(SESSION_USER_ID_KEY, staff.id);

        let site = AdminSite::new();
        site.register::<djangors_auth::Permission>();
        site.register_with::<djangors_auth::Group>(ModelAdminConfig {
            search_fields: Some(&["name"]),
            ..Default::default()
        });
        site.register::<djangors_auth::UserGroup>();
        site.register::<djangors_auth::GroupPermission>();
        site.register::<djangors_auth::UserPermission>();
        let router = Router::new().mount("/admin", site.urls());

        // 1. Create a Group through the generic admin add/ page.
        let mut ext = Extensions::new();
        ext.insert(session.clone());
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/djangors_auth/group/add/"),
            headers,
            Bytes::from("name=Admins"),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);

        let group = djangors_orm::queryset::QuerySet::<djangors_auth::Group>::new()
            .filter(djangors_orm::q!(name = "Admins"))
            .unwrap()
            .get(&db)
            .await
            .unwrap();

        // 2. Add a UserGroup row (group membership) through the generic admin.
        let mut ext = Extensions::new();
        ext.insert(session.clone());
        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let req = Request::new(
            Method::POST,
            Uri::from_static("/admin/djangors_auth/usergroup/add/"),
            headers,
            Bytes::from(format!("user={}&group={}", staff.id, group.id)),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);

        let membership_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM auth_user_groups WHERE \"user\" = $1 AND \"group\" = $2",
        )
        .bind(staff.id)
        .bind(group.id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(membership_count, 1);

        // 3. Changelist pages for both models render through the generic admin.
        let mut ext = Extensions::new();
        ext.insert(session.clone());
        let req = Request::new(
            Method::GET,
            Uri::from_static("/admin/djangors_auth/group/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Admins"));

        let drop_tables = ["auth_user_groups", "auth_group", "auth_user"];
        for table in drop_tables {
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP TABLE IF EXISTS {}",
                table
            )))
            .execute(db.pool())
            .await;
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_audit_diffing_and_history_rendering() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_d")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS djangors_admin_log")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE auth_user (
                id BIGSERIAL PRIMARY KEY,
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined TIMESTAMPTZ NOT NULL,
                last_login TIMESTAMPTZ
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE test_model_d (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE djangors_admin_log (
                id BIGSERIAL PRIMARY KEY,
                user_id BIGINT NOT NULL,
                action_time TIMESTAMPTZ NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();
        let staff = User {
            id: 0,
            username: "staff_diff".to_string(),
            email: "staff_diff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: true,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);
        let mut ext = Extensions::new();
        ext.insert(session_staff.clone());

        let pk = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
            &db,
            vec![
                (
                    "name",
                    djangors_orm::expr::Value::Text("OrigName".to_string()),
                ),
                (
                    "content",
                    djangors_orm::expr::Value::Text("OrigContent".to_string()),
                ),
            ],
        )
        .await
        .unwrap();

        let site = AdminSite::new();
        site.register::<ModelD>();
        let router = Router::new().mount("/admin", site.urls());

        let mut headers = HeaderMap::new();
        headers.insert(
            hyper::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );

        // Submit form changing ONLY 'name', leaving 'content' unchanged ("OrigContent")
        let req_change = Request::new(
            Method::POST,
            Uri::try_from(format!("/admin/admin_test/modeld/{}/change/", pk)).unwrap(),
            headers,
            Bytes::from("name=NewName&content=OrigContent"),
        )
        .with_extensions(ext.clone())
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_change = router.handle(req_change).await.unwrap();
        assert_eq!(res_change.status(), StatusCode::FOUND);

        let logs: Vec<LogEntry> = sqlx::query_as(sqlx::AssertSqlSafe(
            "SELECT id, user_id, action_time, app_label, model_name, object_id, object_repr, action_flag, change_message, field_diff FROM djangors_admin_log WHERE action_flag = 2 ORDER BY id ASC",
        ))
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(logs.len(), 1);

        let raw_diff = logs[0]
            .field_diff
            .as_ref()
            .expect("field_diff should be present");
        let diff_items: Vec<FieldDiffItem> = serde_json::from_str(raw_diff).unwrap();
        assert_eq!(diff_items.len(), 1);
        assert_eq!(diff_items[0].field, "name");
        assert_eq!(diff_items[0].old, "OrigName");
        assert_eq!(diff_items[0].new, "NewName");

        // Verify history page renders the diff block
        let req_hist = Request::new(
            Method::GET,
            Uri::try_from(format!("/admin/admin_test/modeld/{}/history/", pk)).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_hist = router.handle(req_hist).await.unwrap();
        assert_eq!(res_hist.status(), StatusCode::OK);
        let body_hist = String::from_utf8(res_hist.body().to_vec()).unwrap();
        assert!(body_hist.contains("name: OrigName"));
        assert!(body_hist.contains("NewName"));

        let drop_tables = ["test_model_d", "auth_user", "djangors_admin_log"];
        for table in drop_tables {
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP TABLE IF EXISTS {}",
                table
            )))
            .execute(db.pool())
            .await;
        }
    }
}
