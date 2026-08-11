use crate::log::FieldDiffItem;
use crate::perms::{require_perm, resolve_tenant};
use crate::{ModelAdmin, SiteBranding, TenantScope, ADMIN_TEMPLATES};
use djangors_core::{DjangorsError, PathParams, Request, Response};
use std::sync::Arc;

#[derive(serde::Serialize)]
pub(crate) struct HistoryEntryView {
    pub(crate) action_time: String,
    pub(crate) action_flag: i32,
    pub(crate) username: String,
    pub(crate) change_message: String,
    pub(crate) field_diff: Option<Vec<FieldDiffItem>>,
}

#[derive(serde::Serialize)]
pub(crate) struct ObjectHistoryContext {
    pub(crate) model_name: String,
    pub(crate) pk: i64,
    pub(crate) history: Vec<HistoryEntryView>,
    pub(crate) site_header: String,
    pub(crate) site_title: String,
    pub(crate) logo_url: Option<String>,
    pub(crate) accent_color: Option<String>,
}

#[allow(clippy::type_complexity)]
pub(crate) async fn admin_history(
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
