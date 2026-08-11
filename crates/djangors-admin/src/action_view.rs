use crate::log::{log_action, ACTION_CHANGE};
use crate::perms::{require_perm, resolve_tenant};
use crate::{ModelAdmin, SiteBranding, TenantScope, ADMIN_TEMPLATES};
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response};
use std::sync::Arc;

#[derive(serde::Serialize)]
pub(crate) struct BulkActionConfirmContext {
    pub(crate) action_label: String,
    pub(crate) count: usize,
    pub(crate) items: Vec<String>,
    pub(crate) action_name: String,
    pub(crate) pks: Vec<i64>,
    pub(crate) csrf_token: String,
    pub(crate) site_header: String,
    pub(crate) site_title: String,
    pub(crate) logo_url: Option<String>,
    pub(crate) accent_color: Option<String>,
}

pub(crate) async fn admin_bulk_action_post(
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
