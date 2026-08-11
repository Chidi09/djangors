#![deny(missing_docs)]
//! Automatic administration interface for Djangors applications.
//!
//! This crate provides a complete database-driven admin interface, mirroring Django's Admin.
//! Key components:
//! - [`AdminSite`]: Central registry. Holds registered [`ModelAdmin`]s, configures branding, and mounts admin routes.
//! - [`ModelAdmin`]: Trait customising lists, forms, validation, and actions for a registered ORM model.
//! - Dynamic Templates: Core pages are rendered using templates (defined in `ADMIN_TEMPLATES` using `LazyLock`
//!   and `djangors-template`) with CSRF protection enabled.

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

/// Struct containing page data for a model's changelist view.
pub struct ChangelistPage {
    /// Field names to display as columns in declaration order.
    pub columns: Vec<&'static str>,
    /// Display-rendered row strings (view escapes).
    pub rows: Vec<Vec<String>>,
    /// Primary key strings corresponding to each row.
    pub pks: Vec<String>,
    /// Total count of matching items across all pages.
    pub total: i64,
    /// 1-based current page number.
    pub page: i64,
    /// Number of items displayed per page.
    pub per_page: i64,
}

/// Configuration for inline child model display on admin forms.
#[derive(Clone)]
pub struct InlineConfig {
    /// The child model's struct name for lookups via `all_registered_models()`.
    pub struct_name: &'static str,
    /// The FK field name on the child model pointing to the parent.
    pub relation_field: &'static str,
    /// Field names to display in the inline table.
    pub fields: &'static [&'static str],
}

/// Configuration options for custom model admin registration.
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
    /// Date or datetime field name to display date hierarchy navigation bar.
    pub date_hierarchy: Option<&'static str>,
    /// List of field names that are editable inline on the changelist page.
    pub list_editable: Option<&'static [&'static str]>,
    /// Computed display columns functions.
    #[allow(clippy::type_complexity)]
    pub computed_columns: Option<
        &'static [(
            &'static str,
            fn(&[(&'static str, djangors_orm::expr::Value)]) -> String,
        )],
    >,
    /// Bulk actions available on the changelist page.
    pub actions: Option<&'static [AdminAction]>,
    /// Groupings of fields into fieldsets on the change form.
    pub fieldsets: Option<&'static [(&'static str, &'static [&'static str])]>,
    /// List of field names rendered as read-only on the change form.
    pub readonly_fields: Option<&'static [&'static str]>,
    /// List of foreign key field names rendered as raw ID inputs.
    pub raw_id_fields: Option<&'static [&'static str]>,
    /// Base filter expression automatically applied to all admin queries.
    pub base_filter: Option<djangors_orm::UnresolvedExpr>,
    /// Inline child model configurations rendered below the add/change form.
    pub inlines: Option<&'static [InlineConfig]>,
}

/// A resolved, request-scoped tenant restriction: the tenant FK field name on the
/// model plus the tenant id every row must match. `None` means "no restriction"
/// (the admin site is not tenant-scoped, or the acting user is a superuser).
///
/// This is passed *per call* rather than stored on the `ModelAdmin`. A
/// `ModelAdmin` is constructed once at registration and shared across every
/// concurrent request via `Arc`, so storing the active tenant on the admin would
/// let one request's tenant leak into another's query while the first is awaiting
/// the database.
pub type TenantFilter = Option<(&'static str, i64)>;

/// Pluggable administration interface trait for managing ORM models.
#[async_trait]
pub trait ModelAdmin: Send + Sync {
    /// Returns the metadata for the administered model.
    fn model_meta(&self) -> &'static ModelMeta;
    /// Returns the list of model field names in declaration order.
    fn field_names(&self) -> Vec<&'static str>;
    /// Queries and returns a paginated [`ChangelistPage`] for this model.
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
        tenant: TenantFilter,
    ) -> Result<ChangelistPage, DjangorsError>;

    /// Queries and returns headers and rows for CSV export.
    #[allow(clippy::too_many_arguments)]
    async fn export_csv_rows(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
        tenant: TenantFilter,
    ) -> Result<(Vec<&'static str>, Vec<Vec<String>>), DjangorsError>;

    /// Returns search field names for this model admin.
    fn search_fields(&self) -> &[&'static str];
    /// Returns list filter field names for this model admin.
    fn list_filter_fields(&self) -> &[&'static str];
    /// Returns the date hierarchy field name if configured.
    fn date_hierarchy_field(&self) -> Option<&'static str>;
    /// Returns list editable field names for this model admin.
    fn list_editable_fields(&self) -> &[&'static str];
    /// Returns configured bulk admin actions for this model admin.
    fn actions(&self) -> Vec<AdminAction> {
        Vec::new()
    }
    /// Returns configured change form fieldsets.
    fn fieldsets(&self) -> Option<&'static [(&'static str, &'static [&'static str])]> {
        None
    }
    /// Returns read-only field names for the change form.
    fn readonly_fields(&self) -> &[&'static str] {
        &[]
    }
    /// Returns raw ID field names for foreign keys.
    fn raw_id_fields(&self) -> &[&'static str] {
        &[]
    }

    /// Returns configured inline child model configurations.
    fn inlines_config(&self) -> &[InlineConfig] {
        &[]
    }

    /// Fetches a single object by primary key as name/value tuples.
    ///
    /// Returns `None` when the row does not exist *or* belongs to another
    /// tenant, so a cross-tenant primary key is indistinguishable from a
    /// missing one.
    async fn get_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        tenant: TenantFilter,
    ) -> Result<Option<Vec<(&'static str, djangors_orm::expr::Value)>>, DjangorsError>;

    /// Updates an existing object from form parameters.
    async fn update_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError>;

    /// Updates specific fields of an existing object from form parameters.
    async fn update_fields_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError>;

    /// Creates a new model instance from form parameters.
    ///
    /// When `tenant` is set, the new row's tenant field is forced to that
    /// tenant, so a submitted form can never plant a row in another tenant.
    async fn create_from_form(
        &self,
        db: &djangors_db::Database,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
    ) -> Result<Result<i64, std::collections::HashMap<String, String>>, DjangorsError>;

    /// Deletes a single object by primary key. Returns `false` without deleting
    /// anything when the row belongs to another tenant.
    async fn delete_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        tenant: TenantFilter,
    ) -> Result<bool, DjangorsError>;
}

/// Builds the `field = tenant_id` comparison for a resolved [`TenantFilter`].
fn tenant_compare(tenant: TenantFilter) -> Option<djangors_orm::expr::UnresolvedCompare> {
    tenant.map(|(field, id)| djangors_orm::expr::UnresolvedCompare {
        field,
        value: djangors_orm::expr::Value::I64(id),
    })
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

    #[allow(clippy::too_many_arguments)]
    fn build_filtered_queryset(
        &self,
        order: Option<&str>,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
        tenant: TenantFilter,
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
        if let Some(compare) = tenant_compare(tenant) {
            qs = qs
                .filter(djangors_orm::expr::UnresolvedExpr::And(vec![compare]))
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        }
        Ok(qs)
    }

    /// Returns a `QuerySet<M>` filtered by primary key `pk` AND, when tenant scoping
    /// is active, the current tenant — so PK-backed reads/writes never cross tenant
    /// boundaries (`get_by_pk`, `update_from_form`, `delete_by_pk`, etc.).
    fn scoped_pk_queryset(
        &self,
        pk: i64,
        tenant: TenantFilter,
    ) -> Result<djangors_orm::queryset::QuerySet<M>, DjangorsError> {
        let meta = M::meta();
        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;
        let mut compares = vec![djangors_orm::expr::UnresolvedCompare {
            field: pk_field.name,
            value: djangors_orm::expr::Value::I64(pk),
        }];
        if let Some(tc) = tenant_compare(tenant) {
            compares.push(tc);
        }
        M::objects()
            .filter(djangors_orm::expr::UnresolvedExpr::And(compares))
            .map_err(|e| DjangorsError::Internal(e.to_string()))
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

    fn inlines_config(&self) -> &[InlineConfig] {
        self.config.inlines.unwrap_or(&[])
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
        tenant: TenantFilter,
    ) -> Result<ChangelistPage, DjangorsError> {
        let mut qs = self.build_filtered_queryset(order, search, filters, date_range, tenant)?;

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
        tenant: TenantFilter,
    ) -> Result<(Vec<&'static str>, Vec<Vec<String>>), DjangorsError> {
        let qs = self.build_filtered_queryset(order, search, filters, date_range, tenant)?;
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
        tenant: TenantFilter,
    ) -> Result<Option<Vec<(&'static str, djangors_orm::expr::Value)>>, DjangorsError> {
        let row_opt = self.scoped_pk_queryset(pk, tenant)?.get(db).await;

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
        tenant: TenantFilter,
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

        let qs = self.scoped_pk_queryset(pk, tenant)?;

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
        tenant: TenantFilter,
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

        let qs = self.scoped_pk_queryset(pk, tenant)?;
        qs.update(db, sets)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Ok(Ok(()))
    }

    async fn create_from_form(
        &self,
        db: &djangors_db::Database,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
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

        // When tenant scoping is active, force the new row's tenant field to the
        // current tenant so an object created from a scoped add-form always belongs
        // to the right tenant — the form cannot set another tenant's id.
        if let Some((field, id)) = tenant {
            values.push((field, djangors_orm::expr::Value::I64(id)));
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
        tenant: TenantFilter,
    ) -> Result<bool, DjangorsError> {
        // If tenant scoping is active, only delete when the row also belongs to the
        // current tenant — otherwise a cross-tenant PK could be deleted.
        if tenant.is_some() {
            let scoped = self
                .scoped_pk_queryset(pk, tenant)?
                .first(db)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            if scoped.is_none() {
                return Ok(false);
            }
        }
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
        FieldKind::FileField => Ok(Value::Text(raw.to_string())),
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

/// Definition of a custom bulk admin action.
#[derive(Clone)]
pub struct AdminAction {
    /// Internal action key name.
    pub name: &'static str,
    /// Human-readable label displayed in the action dropdown.
    pub label: &'static str,
    /// Whether executing this action requires a confirmation page step.
    pub requires_confirm: bool,
    /// Async handler executing the action against selected primary keys.
    #[allow(clippy::type_complexity)]
    pub handler: for<'a> fn(
        &'a djangors_db::Database,
        &'a [i64],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), DjangorsError>> + Send + 'a>,
    >,
}

/// Serialized row for rendering action options in templates.
#[derive(serde::Serialize)]
pub struct AdminActionRow {
    /// Internal action key name.
    pub name: String,
    /// Human-readable action label.
    pub label: String,
}

/// Log entry action flag for object addition.
pub const ACTION_ADDITION: i32 = 1;
/// Log entry action flag for object modification.
pub const ACTION_CHANGE: i32 = 2;
/// Log entry action flag for object deletion.
pub const ACTION_DELETION: i32 = 3;

/// Struct capturing a field change diff item.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldDiffItem {
    /// Field name that changed.
    pub field: String,
    /// Value before modification.
    pub old: String,
    /// Value after modification.
    pub new: String,
}

/// Audit log entry model recording staff admin actions.
#[derive(DeriveModel, Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[djangors(app = "admin", table_name = "djangors_admin_log")]
pub struct LogEntry {
    /// Primary key log entry ID.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// ID of the staff user who performed the action.
    pub user_id: i64,
    /// Timestamp when the action was logged.
    pub action_time: chrono::DateTime<chrono::Utc>,
    /// Application label of the target model.
    pub app_label: String,
    /// Struct name of the target model.
    pub model_name: String,
    /// Primary key ID of the target object.
    pub object_id: i64,
    /// String representation of the target object.
    pub object_repr: String,
    /// Action flag (`ACTION_ADDITION`, `ACTION_CHANGE`, `ACTION_DELETION`).
    pub action_flag: i32,
    /// Human-readable summary of changes.
    pub change_message: String,
    /// Optional JSON string detailing field-level diffs.
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
            if err_str.contains("djangors_admin_log")
                && (err_str.contains("does not exist") || err_str.contains("no such table"))
            {
                return Ok(());
            }
            Err(DjangorsError::Internal(err_str))
        }
    }
}

/// Custom branding options for the admin interface header and title.
#[derive(Clone)]
pub struct SiteBranding {
    /// Header title displayed at top left of admin pages.
    pub site_header: String,
    /// Browser window/tab title suffix.
    pub site_title: String,
    /// Optional custom logo image URL.
    pub logo_url: Option<String>,
    /// Optional custom CSS accent color hex string.
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
struct InlineRowData {
    values: Vec<String>,
}

/// Tenant-scoping configuration for the admin site.
#[derive(Clone)]
pub struct TenantScope {
    /// The FK field name on every registered model pointing to the Tenant.
    pub tenant_field: &'static str,
    /// Extract the current tenant id from a request. Typically reads
    /// `djangors_contrib_tenancy::CurrentTenant` from request extensions.
    pub extract_tenant_id: fn(&djangors_core::Request) -> Option<i64>,
}

#[derive(serde::Serialize)]
struct InlineTableData {
    struct_name: String,
    fields: Vec<String>,
    rows: Vec<InlineRowData>,
}

#[derive(serde::Serialize)]
struct RenderFormContext {
    rows: Vec<FormFieldRow>,
    inlines: Vec<InlineTableData>,
    site_header: String,
    site_title: String,
    csrf_token: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
    has_change_permission: bool,
    has_delete_permission: bool,
    is_add: bool,
}

async fn build_inline_tables(
    db: &djangors_db::Database,
    inlines: &[InlineConfig],
    pk: i64,
) -> Result<Vec<InlineTableData>, DjangorsError> {
    let mut tables = Vec::new();
    for ic in inlines {
        let child_meta = djangors_orm::meta::all_registered_models()
            .find(|m| m.struct_name == ic.struct_name)
            .ok_or_else(|| DjangorsError::Internal(format!("unknown model {}", ic.struct_name)))?;

        let mut conn = db.conn();
        let dialect = conn.dialect();
        let p1 = dialect.placeholder(1);
        let cols: Vec<String> = ic.fields.iter().map(|f| format!("\"{}\"", f)).collect();
        let cols_str = cols.join(", ");
        let sql = format!(
            "SELECT {cols_str} FROM \"{}\" WHERE \"{}\" = {p1}",
            child_meta.table_name, ic.relation_field
        );
        let rows = conn
            .fetch_all(&sql, &[djangors_db::BindValue::I64(pk)])
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let mut data_rows = Vec::new();
        for row in &rows {
            let mut values = Vec::new();
            for f in ic.fields {
                let val = row
                    .try_string_by_name(f)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "-".to_string());
                values.push(val);
            }
            data_rows.push(InlineRowData { values });
        }

        tables.push(InlineTableData {
            struct_name: ic.struct_name.to_string(),
            fields: ic.fields.iter().map(|s| s.to_string()).collect(),
            rows: data_rows,
        });
    }
    Ok(tables)
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
    inlines_data: Vec<InlineTableData>,
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
            inlines: inlines_data,
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

/// Central administration site registry managing registered models and admin views.
pub struct AdminSite {
    // Thread-safe registry containing all registered model administrations.
    registry: Mutex<Vec<Arc<dyn ModelAdmin>>>,
    // Custom branding details (headers, titles, logos, accents) applied to the admin site.
    branding: SiteBranding,
    // Extra user-registered routers to be mounted alongside default admin urls.
    extra_routes: Vec<std::panic::AssertUnwindSafe<Router>>,
    // Optional tenant scoping configuration.
    tenant_scope: Option<TenantScope>,
}

impl Default for AdminSite {
    fn default() -> Self {
        Self::new()
    }
}

impl AdminSite {
    /// Creates a new empty `AdminSite` with default branding.
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(Vec::new()),
            branding: SiteBranding::default(),
            extra_routes: Vec::new(),
            tenant_scope: None,
        }
    }

    /// Sets the admin site header text.
    pub fn with_site_header(mut self, header: impl Into<String>) -> Self {
        self.branding.site_header = header.into();
        self
    }

    /// Sets the admin site title text.
    pub fn with_site_title(mut self, title: impl Into<String>) -> Self {
        self.branding.site_title = title.into();
        self
    }

    /// Sets a custom logo URL for the admin site header.
    pub fn with_logo_url(mut self, url: impl Into<String>) -> Self {
        self.branding.logo_url = Some(url.into());
        self
    }

    /// Sets a custom CSS accent color for the admin site.
    pub fn with_accent_color(mut self, color: impl Into<String>) -> Self {
        self.branding.accent_color = Some(color.into());
        self
    }

    /// Register an extra route directly on the admin site.
    pub fn extra_route(mut self, router: Router) -> Self {
        self.extra_routes.push(std::panic::AssertUnwindSafe(router));
        self
    }

    /// Enable tenant scoping on the admin. When set, every changelist, add,
    /// change, and delete view is restricted to rows belonging to the current
    /// tenant.
    pub fn with_tenant_scoping(
        mut self,
        tenant_field: &'static str,
        extract_tenant_id: fn(&djangors_core::Request) -> Option<i64>,
    ) -> Self {
        self.tenant_scope = Some(TenantScope {
            tenant_field,
            extract_tenant_id,
        });
        self
    }

    /// Register a model with the default (no customization) ModelAdmin.
    pub fn register<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static>(&self) {
        self.register_with::<M>(ModelAdminConfig::default());
    }

    /// Registers a model `M` with custom [`ModelAdminConfig`].
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
                let has_choices = !field.choices.is_empty();
                assert!(
                    matches!(field.kind, djangors_orm::meta::FieldKind::Boolean) || has_choices,
                    "list_filter field '{}' on model '{}' is not a Boolean field and has no choices set",
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

        if let Some(inlines) = config.inlines {
            for ic in inlines {
                let child_meta = djangors_orm::meta::all_registered_models()
                    .find(|m| m.struct_name == ic.struct_name)
                    .unwrap_or_else(|| {
                        panic!(
                            "inline child model '{}' is not a registered model",
                            ic.struct_name
                        )
                    });
                let rel_exists = child_meta
                    .relations
                    .iter()
                    .any(|r| r.field_name == ic.relation_field);
                assert!(
                    rel_exists,
                    "inline relation_field '{}' does not exist on child model '{}'",
                    ic.relation_field, ic.struct_name
                );
                for fname in ic.fields {
                    let field_exists = child_meta.fields.iter().any(|f| f.name == *fname)
                        || child_meta.relations.iter().any(|r| r.field_name == *fname);
                    assert!(
                        field_exists,
                        "inline field '{}' does not exist on child model '{}'",
                        fname, ic.struct_name
                    );
                }
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

        let changelist_tenant_scope = self.tenant_scope.clone();
        let export_csv_tenant_scope = self.tenant_scope.clone();
        let add_get_tenant_scope = self.tenant_scope.clone();
        let add_post_tenant_scope = self.tenant_scope.clone();
        let change_get_tenant_scope = self.tenant_scope.clone();
        let change_post_tenant_scope = self.tenant_scope.clone();
        let delete_get_tenant_scope = self.tenant_scope.clone();
        let delete_post_tenant_scope = self.tenant_scope.clone();
        let history_tenant_scope = self.tenant_scope.clone();
        let bulk_delete_tenant_scope = self.tenant_scope.clone();
        let bulk_action_tenant_scope = self.tenant_scope.clone();
        let save_changelist_tenant_scope = self.tenant_scope.clone();

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
                        changelist_tenant_scope.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/export-csv/",
                move |req: Request, params: PathParams| {
                    admin_export_csv(
                        req,
                        params,
                        export_csv_admins.clone(),
                        export_csv_tenant_scope.clone(),
                    )
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
                        add_get_tenant_scope.clone(),
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
                        add_post_tenant_scope.clone(),
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
                        change_get_tenant_scope.clone(),
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
                        change_post_tenant_scope.clone(),
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
                        delete_get_tenant_scope.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/{pk:i64}/delete/",
                move |req: Request, params: PathParams| {
                    admin_delete_post(
                        req,
                        params,
                        delete_post_admins.clone(),
                        delete_post_tenant_scope.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/{pk:i64}/history/",
                move |req: Request, params: PathParams| {
                    admin_history(
                        req,
                        params,
                        admins.clone(),
                        history_branding.clone(),
                        history_tenant_scope.clone(),
                    )
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
                        bulk_delete_tenant_scope.clone(),
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
                        save_changelist_tenant_scope.clone(),
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
                        bulk_action_tenant_scope.clone(),
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
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "view").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let state = parse_changelist_query_state(&req, admin.as_ref())?;

    let (columns, rows) = admin
        .export_csv_rows(
            db,
            state.order,
            state.search,
            &state.filters,
            state.date_range,
            tenant,
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

/// Resolves the tenant restriction to apply to this request's admin queries.
///
/// A superuser is deliberately unrestricted, so they see every tenant's rows. For anyone
/// else, when the site is tenant-scoped, the tenant id MUST resolve — a scoped site that
/// cannot determine the current tenant fails closed with `Forbidden` rather than silently
/// running the query unrestricted across every tenant.
fn resolve_tenant(
    user: &User,
    tenant_scope: &Option<TenantScope>,
    req: &Request,
) -> Result<TenantFilter, DjangorsError> {
    if user.is_superuser {
        return Ok(None);
    }
    let Some(ts) = tenant_scope.as_ref() else {
        return Ok(None);
    };
    match (ts.extract_tenant_id)(req) {
        Some(tid) => Ok(Some((ts.tenant_field, tid))),
        None => Err(DjangorsError::Forbidden(
            "no tenant could be resolved for this request".to_string(),
        )),
    }
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

    let db = req.require_state::<djangors_db::Database>()?;

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

    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let sql = format!(
        "SELECT id, user_id, action_time, app_label, model_name, object_id, object_repr, \
         action_flag, change_message, field_diff FROM djangors_admin_log WHERE user_id = {p1} \
         ORDER BY action_time DESC LIMIT 10"
    );
    let params = vec![djangors_db::BindValue::I64(user.id)];
    let db_rows_res = conn.fetch_all(&sql, &params).await;

    let recent_rows = match db_rows_res {
        Ok(rows) => {
            let mut entries = Vec::with_capacity(rows.len());
            let mut err = None;
            for r in &rows {
                let id = match r.try_i64(0) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let user_id = match r.try_i64(1) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let action_time = match r.try_datetime(2) {
                    Ok(Some(v)) => v,
                    Ok(None) => {
                        err = Some(sqlx::Error::Decode("missing action_time".into()));
                        break;
                    }
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let app_label = match r.try_string(3) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let model_name = match r.try_string(4) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let object_id = match r.try_i64(5) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let object_repr = match r.try_string(6) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let action_flag = match r.try_i64(7) {
                    Ok(v) => v.unwrap_or_default() as i32,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let change_message = match r.try_string(8) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let field_diff = match r.try_string(9) {
                    Ok(v) => v,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                entries.push(LogEntry {
                    id,
                    user_id,
                    action_time,
                    app_label,
                    model_name,
                    object_id,
                    object_repr,
                    action_flag,
                    change_message,
                    field_diff,
                });
            }
            if let Some(e) = err {
                return Err(DjangorsError::Internal(e.to_string()));
            }
            entries
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("djangors_admin_log")
                && (err_str.contains("does not exist") || err_str.contains("no such table"))
            {
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

// Holds the parsed query-state (ordering, search, filter flags, and date-hierarchy parameters)
// used to filter and paginate model instances displayed on the admin changelist page.
struct ChangelistQueryState<'a> {
    // Sorting order parameter parsed from ?o= (e.g. "field" or "-field").
    order: Option<&'a str>,
    // Search term parsed from ?q=.
    search: Option<&'a str>,
    // Active boolean filter flags parsed from query fields.
    filters: Vec<(&'static str, bool)>,
    // Optional year filter for date-hierarchy navigation.
    year: Option<i32>,
    // Optional month filter for date-hierarchy navigation.
    month: Option<u32>,
    // Optional day filter for date-hierarchy navigation.
    day: Option<u32>,
    // Derived UTC timestamp range matching the hierarchical date filter.
    date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
}

// Parses URL query parameters (ordering, search query, active filters, and date hierarchical
// parameters) from the incoming request into `ChangelistQueryState`.
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
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "view").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

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
            tenant,
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
            // Note: We escape cell values here in Rust using `djangors_core::html_escape`
            // rather than letting minijinja autoescape them. This is because `html_escape`
            // escapes `/` as `&#x2F;` (uppercase hex), whereas minijinja escapes it as `&#x2f;`
            // (lowercase hex). The integration tests verify the exact uppercase hex representation
            // for XSS checks (e.g. testing `</script>`), and would fail if lowercase was emitted.
            // Correspondingly, the template uses `|safe` to render this pre-escaped value.
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
        // The query string value is also pre-escaped with `djangors_core::html_escape` to preserve
        // uppercase hex `/` escaping in HTML assertions, matching the cell value escaping strategy.
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
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "add").await?;
    let _tenant = resolve_tenant(&user, &tenant_scope, &req)?;

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
        vec![],
    )
}

async fn admin_add_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "add").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let Form(form_data) =
        Form::<std::collections::HashMap<String, String>>::from_request(&req).await?;

    match admin.create_from_form(db, &form_data, tenant).await? {
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
                vec![],
            )
        }
    }
}

async fn admin_change_get(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

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
        .get_by_pk(db, pk, tenant)
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
    let inlines_data = build_inline_tables(db, admin.inlines_config(), pk).await?;
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
        inlines_data,
    )
}

async fn admin_change_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let Form(form_data) =
        Form::<std::collections::HashMap<String, String>>::from_request(&req).await?;

    let old_row_vals = admin.get_by_pk(db, pk, tenant).await?;

    match admin.update_from_form(db, pk, &form_data, tenant).await? {
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
                .get_by_pk(db, pk, tenant)
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
            let inlines_data = build_inline_tables(db, admin.inlines_config(), pk).await?;
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
                inlines_data,
            )
        }
    }
}

/// Summary of related objects affected by a deletion action.
pub struct RelatedObjectSummary {
    /// App label of the referencing model.
    pub app_label: &'static str,
    /// Struct name of the referencing model.
    pub struct_name: &'static str,
    /// Foreign key field name on the referencing model.
    pub field_name: &'static str,
    /// On delete cascade behavior strategy.
    pub on_delete: djangors_orm::meta::OnDelete,
    /// Count of matching dependent objects.
    pub count: i64,
    /// Nested child related object summaries.
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
                let mut conn = db.conn();
                let p1 = conn.dialect().placeholder(1);
                let sql = format!(
                    "SELECT COUNT(*) FROM {} WHERE {} = {p1}",
                    related_meta.table_name, relation.field_name
                );
                let params = vec![djangors_db::BindValue::I64(pk)];
                let count_result = match conn.fetch_one(&sql, &params).await {
                    Ok(row) => row.try_i64(0).map(|opt| opt.unwrap_or(0)),
                    Err(e) => Err(e),
                };
                let count = match count_result {
                    Ok(c) => c,
                    Err(e)
                        if e.to_string().contains("does not exist")
                            || e.to_string().contains("no such table") =>
                    {
                        continue
                    }
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
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let year_expr = dialect.extract_date_part(djangors_db::DatePart::Year, col);
    let month_expr = dialect.extract_date_part(djangors_db::DatePart::Month, col);
    let day_expr = dialect.extract_date_part(djangors_db::DatePart::Day, col);

    let mut params = Vec::new();
    let sql = match (year, month) {
        (None, _) => format!(
            "SELECT DISTINCT {year_expr} AS v FROM {table_name} WHERE {col} IS NOT NULL ORDER BY 1"
        ),
        (Some(y), None) => {
            let p1 = dialect.placeholder(1);
            params.push(djangors_db::BindValue::I64(y as i64));
            format!(
                "SELECT DISTINCT {month_expr} AS v FROM {table_name} WHERE {col} IS NOT NULL AND {year_expr} = {p1} ORDER BY 1"
            )
        }
        (Some(y), Some(m)) => {
            let p1 = dialect.placeholder(1);
            let p2 = dialect.placeholder(2);
            params.push(djangors_db::BindValue::I64(y as i64));
            params.push(djangors_db::BindValue::I64(m as i64));
            format!(
                "SELECT DISTINCT {day_expr} AS v FROM {table_name} WHERE {col} IS NOT NULL AND {year_expr} = {p1} AND {month_expr} = {p2} ORDER BY 1"
            )
        }
    };
    let rows = conn
        .fetch_all(&sql, &params)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let mut result = Vec::new();
    for row in rows {
        if let Some(val) = row
            .try_i64(0)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
        {
            result.push(val as i32);
        }
    }
    Ok(result)
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
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "delete").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let row_vals = admin
        .get_by_pk(db, pk, tenant)
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
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "delete").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

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

    let deleted = admin.delete_by_pk(db, pk, tenant).await?;
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
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "view").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    // The log table has no tenant column, so gate on the target object's visibility instead:
    // if this tenant cannot see the object, it cannot read the object's audit history either.
    if tenant.is_some() && admin.get_by_pk(db, pk, tenant).await?.is_none() {
        return Err(DjangorsError::NotFound);
    }

    let meta = admin.model_meta();

    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);
    let p3 = conn.dialect().placeholder(3);
    let sql = format!(
        "SELECT l.user_id, COALESCE(u.username, '?'), l.action_time, l.action_flag, l.change_message, l.field_diff \
         FROM djangors_admin_log l \
         LEFT JOIN auth_user u ON u.id = l.user_id \
         WHERE l.app_label = {p1} AND l.model_name = {p2} AND l.object_id = {p3} \
         ORDER BY l.action_time DESC"
    );
    let params = vec![
        djangors_db::BindValue::Text(meta.app_label.to_string()),
        djangors_db::BindValue::Text(meta.struct_name.to_string()),
        djangors_db::BindValue::I64(pk),
    ];
    let db_rows = conn
        .fetch_all(&sql, &params)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let mut rows = Vec::with_capacity(db_rows.len());
    for row in db_rows {
        let user_id = row
            .try_i64(0)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .unwrap_or_default();
        let username = row
            .try_string(1)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .unwrap_or_else(|| "?".to_string());
        let action_time = row
            .try_datetime(2)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .ok_or_else(|| DjangorsError::Internal("missing action_time".to_string()))?;
        let action_flag = row
            .try_i64(3)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .unwrap_or_default() as i32;
        let change_message = row
            .try_string(4)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .unwrap_or_default();
        let field_diff = row
            .try_string(5)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        rows.push((
            user_id,
            username,
            action_time,
            action_flag,
            change_message,
            field_diff,
        ));
    }

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
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

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
            if let Some(row_vals) = admin.get_by_pk(db, pk, tenant).await? {
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
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "delete").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    // A HashMap extractor silently drops all but the last value for repeated form keys (every
    // checked "selected" checkbox shares the name "selected"). This would result in only one
    // checked row being processed and bulk-deleted. Hence, we extract form parameters as
    // a Vec<(String, String)> to preserve all occurrences of the "selected" key.
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
            if let Some(row_vals) = admin.get_by_pk(db, pk, tenant).await? {
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
        if admin.delete_by_pk(db, pk, tenant).await? {
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
    tenant_scope: Option<TenantScope>,
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

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

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
        match admin
            .update_fields_from_form(db, *pk, fields, tenant)
            .await?
        {
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

/// Registers default admin favicon routes onto `router`.
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
mod tests;
