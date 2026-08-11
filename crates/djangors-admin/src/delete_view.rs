use crate::log::{log_action, ACTION_DELETION};
use crate::perms::{require_perm, resolve_tenant};
use crate::{ModelAdmin, SiteBranding, TenantScope, ADMIN_TEMPLATES};
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response};
use djangors_orm::meta::ModelMeta;
use std::sync::Arc;

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

pub(crate) async fn collect_related_objects(
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

pub(crate) fn collect_related_objects_with_depth<'a>(
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

#[derive(serde::Serialize)]
pub(crate) struct DeleteConfirmField {
    pub(crate) name: String,
    pub(crate) value: String,
}

#[derive(serde::Serialize)]
pub(crate) struct DeleteConfirmRelated {
    pub(crate) struct_name: String,
    pub(crate) table_name: String,
    pub(crate) count: i64,
    pub(crate) on_delete: String,
    pub(crate) nested: Vec<DeleteConfirmRelated>,
}

pub(crate) fn flatten_related(rel: RelatedObjectSummary) -> DeleteConfirmRelated {
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
pub(crate) struct DeleteConfirmContext {
    pub(crate) fields: Vec<DeleteConfirmField>,
    pub(crate) related: Vec<DeleteConfirmRelated>,
    pub(crate) site_header: String,
    pub(crate) site_title: String,
    pub(crate) csrf_token: String,
    pub(crate) logo_url: Option<String>,
    pub(crate) accent_color: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct BulkDeleteConfirmContext {
    pub(crate) count: usize,
    pub(crate) items: Vec<String>,
    pub(crate) pks: Vec<i64>,
    pub(crate) site_header: String,
    pub(crate) site_title: String,
    pub(crate) csrf_token: String,
    pub(crate) logo_url: Option<String>,
    pub(crate) accent_color: Option<String>,
}

pub(crate) async fn admin_delete_get(
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

    let row_opt = admin.get_by_pk(db, pk, tenant).await?;
    let row_vals = row_opt.ok_or(DjangorsError::NotFound)?;

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

pub(crate) async fn admin_delete_post(
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

pub(crate) async fn admin_bulk_delete_post(
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
