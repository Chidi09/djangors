//! Per-request tenant resolution, verified against real membership rows - never trusts a
//! client-supplied tenant identifier without checking the current user actually belongs to it.

use crate::models::TenantMembership;
use djangors_orm::djangors_db::Database;
use djangors_orm::Model;
use std::sync::Arc;
use tower::{Layer, Service};

/// The resolved, membership-verified tenant for the current request. Set into per-request
/// extensions by [`TenantResolutionLayer`] and read via `req.ext::<CurrentTenant>()` from a
/// `djangors_rest::Scoped::scope()` implementation (see [`crate::tenant_scope`]) - `Router::dispatch`
/// propagates a real middleware's `req.extensions_mut()` writes into `Request::ext()`, not
/// `Request::state()` (that's separate, app-wide state set once via `Router::with_state`).
#[derive(Debug, Clone, Copy)]
pub struct CurrentTenant(pub i64);

/// Middleware that reads an `X-Tenant-Id` header, and - if a `CurrentUserId`-equivalent identity
/// is already present on the request from an upstream auth layer - verifies the requesting user
/// actually has a `TenantMembership` row for that tenant before inserting `CurrentTenant`. A
/// missing header, a non-integer header value, or a tenant the user has no membership in all
/// result in no `CurrentTenant` being set (NOT a default/fallback tenant) - downstream
/// `tenant_scope()` calls then correctly reject the request via the same
/// `req.state::<CurrentTenant>()` -> `None` -> `Unauthorized` path already used for `CurrentOwner`.
///
/// This crate does not assume a specific "current authenticated user id" extension type, since
/// that's owned by whatever auth setup the application uses (djangors-auth's session backend, a
/// custom JWT layer, etc.) - `TenantResolutionLayer::new` takes an extractor closure
/// `Fn(&hyper::Request<B>) -> Option<i64>` so the application supplies how to get the current
/// user id from whatever request state its own auth middleware already populated upstream.
#[derive(Clone)]
pub struct TenantResolutionLayer<F> {
    db: Arc<Database>,
    user_id_extractor: F,
}

impl<F> TenantResolutionLayer<F> {
    /// Creates a new layer. `user_id_extractor` must read whatever the application's own upstream
    /// auth middleware already stored on the request (e.g. a `CurrentUserId(i64)` extension) and
    /// return the authenticated user's id, or `None` if unauthenticated.
    pub fn new(db: Database, user_id_extractor: F) -> Self {
        Self {
            db: Arc::new(db),
            user_id_extractor,
        }
    }
}

impl<S, F> Layer<S> for TenantResolutionLayer<F>
where
    F: Clone,
{
    type Service = TenantResolutionService<S, F>;

    fn layer(&self, inner: S) -> Self::Service {
        TenantResolutionService {
            inner,
            db: self.db.clone(),
            user_id_extractor: self.user_id_extractor.clone(),
        }
    }
}

/// The tower `Service` produced by [`TenantResolutionLayer`].
#[derive(Clone)]
pub struct TenantResolutionService<S, F> {
    inner: S,
    db: Arc<Database>,
    user_id_extractor: F,
}

impl<S, F, ReqBody, ResBody> Service<hyper::Request<ReqBody>> for TenantResolutionService<S, F>
where
    S: Service<hyper::Request<ReqBody>, Response = hyper::Response<ResBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    F: Fn(&hyper::Request<ReqBody>) -> Option<i64> + Clone + Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        let db = self.db.clone();

        let tenant_header = req
            .headers()
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok());
        let user_id = (self.user_id_extractor)(&req);

        Box::pin(async move {
            if let (Some(tenant_id), Some(uid)) = (tenant_header, user_id) {
                // Real membership check - never trust the header alone.
                let qs = TenantMembership::objects()
                    .filter(djangors_orm::q!(user = uid))
                    .and_then(|qs| qs.filter(djangors_orm::q!(tenant = tenant_id)));

                if let Ok(qs) = qs {
                    if let Ok(Some(_)) = qs.first(&*db).await {
                        req.extensions_mut().insert(CurrentTenant(tenant_id));
                    }
                }
            }
            inner.call(req).await
        })
    }
}
