use async_trait::async_trait;
use djangors_auth::{Auth, User};
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_orm::meta::{Model, ModelMeta};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

pub(crate) const CHANGELIST_PER_PAGE: i64 = 100;

pub struct ChangelistPage {
    pub columns: Vec<&'static str>, // field names, declaration order
    pub rows: Vec<Vec<String>>,     // Display-rendered, NOT escaped (view escapes)
    pub total: i64,                 // COUNT(*) over the whole table
    pub page: i64,                  // 1-based current page
    pub per_page: i64,
}

#[async_trait]
pub trait ModelAdmin: Send + Sync {
    fn model_meta(&self) -> &'static ModelMeta;
    fn field_names(&self) -> Vec<&'static str>;
    async fn changelist(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>, // raw ?o= value, e.g. "name" or "-name"
        page: i64,           // already-validated >= 1
        per_page: i64,
    ) -> Result<ChangelistPage, DjangorsError>;

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

    async fn create_from_form(
        &self,
        db: &djangors_db::Database,
        form: &std::collections::HashMap<String, String>,
    ) -> Result<Result<i64, std::collections::HashMap<String, String>>, DjangorsError>;
}

/// Blanket impl so any real Model can be registered with zero boilerplate.
pub struct DefaultModelAdmin<M: Model>(PhantomData<M>);

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

    async fn changelist(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>,
        page: i64,
        per_page: i64,
    ) -> Result<ChangelistPage, DjangorsError> {
        let mut qs = M::objects();
        if let Some(o) = order {
            qs = qs.order_by(o).map_err(|e| match e {
                djangors_orm::error::OrmError::FieldNotFound { .. } => {
                    DjangorsError::BadRequest(e.to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;
        }
        let total = M::objects()
            .count(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let offset = (page - 1) * per_page;
        qs = qs.limit(per_page).offset(offset);
        let items = qs
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let mut rows = Vec::new();
        for item in &items {
            let row_vals: Vec<String> = item
                .field_values()
                .into_iter()
                .map(|(_, v)| v.to_string())
                .collect();
            rows.push(row_vals);
        }

        let columns = M::field_names();

        Ok(ChangelistPage {
            columns,
            rows,
            total,
            page,
            per_page,
        })
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

fn render_form(
    meta: &'static ModelMeta,
    field_names: &[&'static str],
    submitted_values: &std::collections::HashMap<String, String>,
    errors: &std::collections::HashMap<String, String>,
    is_add: bool,
) -> String {
    let mut rows_html = String::new();

    for &name in field_names {
        if let Some(field) = meta.fields.iter().find(|f| f.name == name) {
            if field.auto || field.primary_key {
                if is_add {
                    continue;
                } else {
                    let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
                    let escaped_val = djangors_core::html_escape(val);
                    rows_html.push_str(&format!(
                        "<div><label>{} (readonly):</label> <span>{}</span></div>",
                        name, escaped_val
                    ));
                    continue;
                }
            }

            let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
            let escaped_val = djangors_core::html_escape(val);
            let err_html = if let Some(err) = errors.get(name) {
                format!(
                    "<div style=\"color: red;\">{}</div>",
                    djangors_core::html_escape(err)
                )
            } else {
                String::new()
            };

            let input_html = match field.kind {
                djangors_orm::meta::FieldKind::Boolean => {
                    let checked = if val == "on" || val == "true" {
                        " checked"
                    } else {
                        ""
                    };
                    format!(
                        "<input type=\"checkbox\" name=\"{}\" id=\"id_{}\"{}>",
                        name, name, checked
                    )
                }
                djangors_orm::meta::FieldKind::Integer
                | djangors_orm::meta::FieldKind::BigInt
                | djangors_orm::meta::FieldKind::Float => {
                    format!(
                        "<input type=\"number\" name=\"{}\" id=\"id_{}\" value=\"{}\">",
                        name, name, escaped_val
                    )
                }
                _ => {
                    format!(
                        "<input type=\"text\" name=\"{}\" id=\"id_{}\" value=\"{}\">",
                        name, name, escaped_val
                    )
                }
            };

            rows_html.push_str(&format!(
                "<div><label for=\"id_{}\">{}</label> {}{}</div>",
                name, name, input_html, err_html
            ));
        } else if let Some(_rel) = meta.relations.iter().find(|r| r.field_name == name) {
            let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
            let escaped_val = djangors_core::html_escape(val);
            let err_html = if let Some(err) = errors.get(name) {
                format!(
                    "<div style=\"color: red;\">{}</div>",
                    djangors_core::html_escape(err)
                )
            } else {
                String::new()
            };

            let input_html = format!(
                "<input type=\"number\" name=\"{}\" id=\"id_{}\" value=\"{}\">",
                name, name, escaped_val
            );

            rows_html.push_str(&format!(
                "<div><label for=\"id_{}\">{}</label> {}{}</div>",
                name, name, input_html, err_html
            ));
        }
    }

    format!(
        "<form method=\"post\">{}<input type=\"submit\" value=\"Submit\"></form>",
        rows_html
    )
}

pub struct AdminSite {
    registry: Mutex<Vec<Arc<dyn ModelAdmin>>>,
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
        }
    }

    /// Register a model with the default (no customization) ModelAdmin.
    pub fn register<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static>(&self) {
        let mut reg = self.registry.lock().unwrap();
        reg.push(Arc::new(DefaultModelAdmin::<M>(PhantomData)));
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

        let index_admins = snapshot.clone();
        let changelist_admins = admins.clone();
        let add_get_admins = admins.clone();
        let add_post_admins = admins.clone();
        let change_get_admins = admins.clone();
        let change_post_admins = admins.clone();

        Router::new()
            .get("/", move |req: Request, params: PathParams| {
                admin_index(req, params, index_admins.clone())
            })
            .get(
                "/{app:slug}/{model:slug}/",
                move |req: Request, params: PathParams| {
                    admin_changelist(req, params, changelist_admins.clone())
                },
            )
            .get(
                "/{app:slug}/{model:slug}/add/",
                move |req: Request, params: PathParams| {
                    admin_add_get(req, params, add_get_admins.clone())
                },
            )
            .post(
                "/{app:slug}/{model:slug}/add/",
                move |req: Request, params: PathParams| {
                    admin_add_post(req, params, add_post_admins.clone())
                },
            )
            .get(
                "/{app:slug}/{model:slug}/{pk:i64}/change/",
                move |req: Request, params: PathParams| {
                    admin_change_get(req, params, change_get_admins.clone())
                },
            )
            .post(
                "/{app:slug}/{model:slug}/{pk:i64}/change/",
                move |req: Request, params: PathParams| {
                    admin_change_post(req, params, change_post_admins.clone())
                },
            )
    }
}

async fn require_staff(req: &Request) -> Result<(), DjangorsError> {
    let auth = Auth::<User>::from_request(req).await?;
    if !auth.0.is_staff {
        return Err(DjangorsError::Forbidden(
            "staff status required".to_string(),
        ));
    }
    Ok(())
}

async fn admin_index(
    req: Request,
    _params: PathParams,
    registry: Vec<&'static ModelMeta>,
) -> Result<Response, DjangorsError> {
    require_staff(&req).await?;

    let mut body = String::new();
    for meta in &registry {
        body.push_str(&format!(
            "<li><a href=\"{}/{}/\">{}.{}</a></li>",
            meta.app_label,
            meta.struct_name.to_lowercase(),
            meta.app_label,
            meta.struct_name
        ));
    }

    Ok(Response::html(StatusCode::OK, format!("<ul>{}</ul>", body)))
}

async fn admin_changelist(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
) -> Result<Response, DjangorsError> {
    require_staff(&req).await?;

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

    let o = req.query("o");
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

    let page_data = admin.changelist(db, o, page, CHANGELIST_PER_PAGE).await?;

    let meta = admin.model_meta();
    let pk_field_name = meta
        .fields
        .iter()
        .find(|f| f.primary_key)
        .map(|f| f.name)
        .unwrap_or("id");
    let pk_col_idx = page_data
        .columns
        .iter()
        .position(|&col| col == pk_field_name);

    let mut header_html = String::new();
    for col in &page_data.columns {
        let link = if o == Some(*col) {
            format!("?o=-{}", col)
        } else {
            format!("?o={}", col)
        };
        header_html.push_str(&format!("<th><a href=\"{}\">{}</a></th>", link, col));
    }

    let mut body_html = String::new();
    for row in page_data.rows {
        body_html.push_str("<tr>");
        let pk_val = if let Some(idx) = pk_col_idx {
            row.get(idx).cloned().unwrap_or_default()
        } else {
            String::new()
        };
        for (i, cell) in row.into_iter().enumerate() {
            let escaped = djangors_core::html_escape(&cell);
            if i == 0 {
                body_html.push_str(&format!(
                    "<td><a href=\"{}/change/\">{}</a></td>",
                    pk_val, escaped
                ));
            } else {
                body_html.push_str(&format!("<td>{}</td>", escaped));
            }
        }
        body_html.push_str("</tr>");
    }

    let total_pages = if page_data.total == 0 {
        1
    } else {
        (page_data.total + CHANGELIST_PER_PAGE - 1) / CHANGELIST_PER_PAGE
    };
    let mut pager_html = String::new();
    if page > 1 {
        let mut prev_link = format!("?page={}", page - 1);
        if let Some(order_val) = o {
            prev_link.push_str(&format!("&o={}", order_val));
        }
        pager_html.push_str(&format!("<a href=\"{}\">Previous</a> ", prev_link));
    }
    pager_html.push_str(&format!(
        "Page {} of {}. Total: {}. ",
        page, total_pages, page_data.total
    ));
    if page * CHANGELIST_PER_PAGE < page_data.total {
        let mut next_link = format!("?page={}", page + 1);
        if let Some(order_val) = o {
            next_link.push_str(&format!("&o={}", order_val));
        }
        pager_html.push_str(&format!("<a href=\"{}\">Next</a>", next_link));
    }

    let html = format!(
        "<table><thead><tr>{}</tr></thead><tbody>{}</tbody></table><div>{}</div>",
        header_html, body_html, pager_html
    );

    Ok(Response::html(StatusCode::OK, html))
}

async fn admin_add_get(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
) -> Result<Response, DjangorsError> {
    require_staff(&req).await?;

    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let meta = admin.model_meta();
    let field_names = admin.field_names();
    let submitted_values = std::collections::HashMap::new();
    let errors = std::collections::HashMap::new();

    let html = render_form(meta, &field_names, &submitted_values, &errors, true);
    Ok(Response::html(StatusCode::OK, html))
}

async fn admin_add_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
) -> Result<Response, DjangorsError> {
    require_staff(&req).await?;

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

    let Form(form_data) =
        Form::<std::collections::HashMap<String, String>>::from_request(&req).await?;

    match admin.create_from_form(db, &form_data).await? {
        Ok(_new_pk) => Ok(Response::redirect(&format!("/{}/{}/", app, model))),
        Err(errors) => {
            let meta = admin.model_meta();
            let field_names = admin.field_names();
            let html = render_form(meta, &field_names, &form_data, &errors, true);
            Ok(Response::html(StatusCode::OK, html))
        }
    }
}

async fn admin_change_get(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
) -> Result<Response, DjangorsError> {
    require_staff(&req).await?;

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

    let html = render_form(meta, &field_names, &submitted_values, &errors, false);
    Ok(Response::html(StatusCode::OK, html))
}

async fn admin_change_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
) -> Result<Response, DjangorsError> {
    require_staff(&req).await?;

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
            let html = render_form(meta, &field_names, &merged_form_data, &errors, false);
            Ok(Response::html(StatusCode::OK, html))
        }
    }
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

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_index_endpoints() {
        let _guard = DB_MUTEX.lock().unwrap();
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop tables
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
            is_superuser: false,
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
        let admin_a = DefaultModelAdmin::<ModelA>(PhantomData);
        let page_data = admin_a.changelist(&db, None, 2, 2).await.unwrap();
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
            is_superuser: false,
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
        let _ = sqlx::query("DROP TABLE auth_user").execute(db.pool()).await;
        let _ = sqlx::query("DROP TABLE test_model_a")
            .execute(db.pool())
            .await;
    }
}
