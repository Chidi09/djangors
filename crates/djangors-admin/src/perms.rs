use crate::{TenantFilter, TenantScope};
use djangors_auth::{Auth, User};
use djangors_core::extract::FromRequest;
use djangors_core::{DjangorsError, Request};
use djangors_orm::meta::ModelMeta;

pub(crate) async fn require_staff(req: &Request) -> Result<User, DjangorsError> {
    let auth = Auth::<User>::from_request(req).await?;
    if !auth.0.is_staff {
        return Err(DjangorsError::Forbidden(
            "staff status required".to_string(),
        ));
    }
    Ok(auth.0)
}

pub(crate) fn action_codename(meta: &'static ModelMeta, action: &str) -> String {
    format!(
        "{}.{}_{}",
        meta.app_label,
        action,
        meta.struct_name.to_lowercase()
    )
}

pub(crate) async fn require_perm(
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
pub(crate) fn resolve_tenant(
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
