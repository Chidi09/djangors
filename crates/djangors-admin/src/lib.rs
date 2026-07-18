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
        Ok(qs)
    }

    fn row_values(&self, item: &M, columns: &[&'static str]) -> Vec<String> {
        let field_values = item.field_values();
        columns
            .iter()
            .map(|col| {
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
}

#[derive(Clone)]
pub struct SiteBranding {
    pub site_header: String,
    pub site_title: String,
}

impl Default for SiteBranding {
    fn default() -> Self {
        Self {
            site_header: "Djangors Administration".to_string(),
            site_title: "Djangors site admin".to_string(),
        }
    }
}

#[derive(serde::Serialize)]
struct RenderFormContext {
    rows: Vec<FormFieldRow>,
    site_header: String,
    site_title: String,
}

fn render_form(
    meta: &'static ModelMeta,
    field_names: &[&'static str],
    submitted_values: &std::collections::HashMap<String, String>,
    errors: &std::collections::HashMap<String, String>,
    is_add: bool,
    branding: &SiteBranding,
) -> Result<Response, DjangorsError> {
    let mut rows = Vec::new();

    for &name in field_names {
        if let Some(field) = meta.fields.iter().find(|f| f.name == name) {
            if field.auto || field.primary_key {
                if is_add {
                    continue;
                } else {
                    let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
                    rows.push(FormFieldRow {
                        kind: "readonly",
                        name: name.to_string(),
                        value: val.to_string(),
                        checked: false,
                        error: None,
                    });
                    continue;
                }
            }

            let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
            let err = errors.get(name).cloned();

            let kind = match field.kind {
                djangors_orm::meta::FieldKind::Boolean => "checkbox",
                djangors_orm::meta::FieldKind::Integer
                | djangors_orm::meta::FieldKind::BigInt
                | djangors_orm::meta::FieldKind::Float => "number",
                _ => "text",
            };

            let checked = kind == "checkbox" && (val == "on" || val == "true");

            rows.push(FormFieldRow {
                kind,
                name: name.to_string(),
                value: val.to_string(),
                checked,
                error: err,
            });
        } else if let Some(_rel) = meta.relations.iter().find(|r| r.field_name == name) {
            let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
            let err = errors.get(name).cloned();

            rows.push(FormFieldRow {
                kind: "number",
                name: name.to_string(),
                value: val.to_string(),
                checked: false,
                error: err,
            });
        }
    }

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/render_form.html",
        RenderFormContext {
            rows,
            site_header: branding.site_header.clone(),
            site_title: branding.site_title.clone(),
        },
    )
}

pub struct AdminSite {
    registry: Mutex<Vec<Arc<dyn ModelAdmin>>>,
    branding: SiteBranding,
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
                    meta.fields.iter().any(|f| f.name == *name),
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

        Router::new()
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
}

#[derive(serde::Serialize)]
struct IndexContext {
    models: Vec<IndexModelLink>,
    site_header: String,
    site_title: String,
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
        if !user.is_superuser {
            let codename = action_codename(meta, "view");
            let allowed = djangors_auth::has_perm(db, user.id, &codename)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            if !allowed {
                continue;
            }
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
        });
    }

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/index.html",
        IndexContext {
            models,
            site_header: branding.site_header,
            site_title: branding.site_title,
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
    header_cells: Vec<HeaderCellData>,
    rows: Vec<ChangelistRowData>,
    show_save_button: bool,
    pager: PagerData,
    export_query: String,
    site_header: String,
    site_title: String,
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

    require_perm(&req, db, admin.model_meta(), "view").await?;

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

    let total_pages = if page_data.total == 0 {
        1
    } else {
        (page_data.total + CHANGELIST_PER_PAGE - 1) / CHANGELIST_PER_PAGE
    };

    let prev_href = if page > 1 {
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

    let next_href = if page * CHANGELIST_PER_PAGE < page_data.total {
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

    render_form(
        meta,
        &field_names,
        &submitted_values,
        &errors,
        true,
        &branding,
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

    require_perm(&req, db, admin.model_meta(), "add").await?;

    let Form(form_data) =
        Form::<std::collections::HashMap<String, String>>::from_request(&req).await?;

    match admin.create_from_form(db, &form_data).await? {
        Ok(_new_pk) => Ok(Response::redirect(&format!("/{}/{}/", app, model))),
        Err(errors) => {
            let meta = admin.model_meta();
            let field_names = admin.field_names();
            render_form(meta, &field_names, &form_data, &errors, true, &branding)
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

    require_perm(&req, db, admin.model_meta(), "change").await?;

    let row_vals = admin
        .get_by_pk(db, pk)
        .await?
        .ok_or(DjangorsError::NotFound)?;

    let mut submitted_values = std::collections::HashMap::new();
    for (name, val) in row_vals {
        submitted_values.insert(name.to_string(), val.to_string());
    }

    let meta = admin.model_meta();
    let field_names = admin.field_names();
    let errors = std::collections::HashMap::new();

    render_form(
        meta,
        &field_names,
        &submitted_values,
        &errors,
        false,
        &branding,
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

    require_perm(&req, db, admin.model_meta(), "change").await?;

    let Form(form_data) =
        Form::<std::collections::HashMap<String, String>>::from_request(&req).await?;

    match admin.update_from_form(db, pk, &form_data).await? {
        Ok(()) => Ok(Response::redirect(&format!("/{}/{}/", app, model))),
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
            let field_names = admin.field_names();
            render_form(
                meta,
                &field_names,
                &merged_form_data,
                &errors,
                false,
                &branding,
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
}

async fn collect_related_objects(
    db: &djangors_db::Database,
    target_meta: &'static ModelMeta,
    pk: i64,
) -> Result<Vec<RelatedObjectSummary>, DjangorsError> {
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
            let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .bind(pk)
                .fetch_one(db.pool())
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            if count > 0 {
                summaries.push(RelatedObjectSummary {
                    app_label: related_meta.app_label,
                    struct_name: related_meta.struct_name,
                    field_name: relation.field_name,
                    on_delete: relation.on_delete,
                    count,
                });
            }
        }
    }
    Ok(summaries)
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
}

#[derive(serde::Serialize)]
struct DeleteConfirmContext {
    fields: Vec<DeleteConfirmField>,
    related: Vec<DeleteConfirmRelated>,
    site_header: String,
    site_title: String,
}

#[derive(serde::Serialize)]
struct BulkDeleteConfirmContext {
    count: usize,
    items: Vec<String>,
    pks: Vec<i64>,
    site_header: String,
    site_title: String,
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
        let table_name = djangors_orm::meta::all_registered_models()
            .find(|m| m.app_label == rel.app_label && m.struct_name == rel.struct_name)
            .map(|m| m.table_name)
            .unwrap_or("");
        related_context.push(DeleteConfirmRelated {
            struct_name: rel.struct_name.to_string(),
            table_name: table_name.to_string(),
            count: rel.count,
            on_delete: format!("{:?}", rel.on_delete),
        });
    }

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/delete_confirm.html",
        DeleteConfirmContext {
            fields,
            related: related_context,
            site_header: branding.site_header,
            site_title: branding.site_title,
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

    require_perm(&req, db, admin.model_meta(), "delete").await?;

    let deleted = admin.delete_by_pk(db, pk).await?;
    if deleted {
        Ok(Response::redirect(&format!("/{}/{}/", app, model)))
    } else {
        Err(DjangorsError::NotFound)
    }
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

    require_perm(&req, db, admin.model_meta(), "delete").await?;

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
        return djangors_template::render(
            &ADMIN_TEMPLATES,
            "admin/bulk_delete_confirm.html",
            BulkDeleteConfirmContext {
                count,
                items,
                pks: pks.clone(),
                site_header: branding.site_header,
                site_title: branding.site_title,
            },
        );
    }

    // Step 2: actually delete. Best-effort per pk - a pk already gone (race, or
    // simply reselected after step 1 already removed it) is not an error, same
    // "false = already gone, not a failure" reasoning admin_delete_post already
    // uses for the single-object route.
    for &pk in &pks {
        admin.delete_by_pk(db, pk).await?;
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

    require_perm(&req, db, admin.model_meta(), "change").await?;

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
            Ok(()) => {}
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
        },
    )
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

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_index_endpoints() {
        let _guard = DB_MUTEX.lock().unwrap();
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
        let _guard = DB_MUTEX.lock().unwrap();
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
        let _guard = DB_MUTEX.lock().unwrap();
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
        let _guard = DB_MUTEX.lock().unwrap();
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
        let _guard = DB_MUTEX.lock().unwrap();
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
        let _guard = DB_MUTEX.lock().unwrap();
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
        let _guard = DB_MUTEX.lock().unwrap();
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
        let _guard = DB_MUTEX.lock().unwrap();
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
        let _guard = DB_MUTEX.lock().unwrap();
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
        let _guard = DB_MUTEX.lock().unwrap();
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

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_permissions_enforcement() {
        let _guard = DB_MUTEX.lock().unwrap();
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
}
