//! Permission policies and combinators for ViewSet routes.

use djangors_auth::AuthUser;
use djangors_core::error::DjangorsError;
use djangors_core::extract::FromRequest;
use djangors_core::request::Request;

use crate::*;

/// A policy deciding whether a request may reach a ViewSet handler.
#[async_trait::async_trait]
pub trait Permission: Send + Sync + 'static {
    /// Determines whether the given request satisfies this permission requirement.
    async fn has_permission(&self, req: &Request) -> bool;
}

/// Explicitly permits unauthenticated requests.
pub struct AllowAny;

#[async_trait::async_trait]
impl Permission for AllowAny {
    async fn has_permission(&self, _req: &Request) -> bool {
        true
    }
}

/// Requires a valid session or API token (and, when enabled, JWT).
pub struct IsAuthenticated;

#[async_trait::async_trait]
impl Permission for IsAuthenticated {
    async fn has_permission(&self, req: &Request) -> bool {
        if djangors_auth::Auth::<djangors_auth::User>::from_request(req)
            .await
            .is_ok()
            || TokenAuth::from_request(req).await.is_ok()
        {
            return true;
        }
        #[cfg(feature = "jwt")]
        {
            return JwtAuth::from_request(req).await.is_ok();
        }
        #[cfg(not(feature = "jwt"))]
        false
    }
}

/// Resolves the authenticated user for a request, checking the same sources as
/// [`IsAuthenticated`]: session auth, then API token, then JWT when enabled.
///
/// Returns `None` for unauthenticated requests.
pub async fn current_user(req: &Request) -> Option<djangors_auth::User> {
    if let Ok(auth) = djangors_auth::Auth::<djangors_auth::User>::from_request(req).await {
        return Some(auth.0);
    }
    if let Ok(token_auth) = TokenAuth::from_request(req).await {
        return Some(token_auth.0);
    }
    #[cfg(feature = "jwt")]
    {
        if let Ok(jwt_auth) = JwtAuth::from_request(req).await {
            return Some(jwt_auth.0);
        }
    }
    None
}

/// Returns the authenticated user, or `Unauthorized` if the request is not
/// authenticated. Wraps [`current_user`] so callers don't need the `Option`
/// pattern themselves.
pub async fn user(req: &Request) -> Result<djangors_auth::User, DjangorsError> {
    current_user(req)
        .await
        .ok_or_else(|| DjangorsError::Unauthorized("not authenticated".into()))
}

/// Requires an authenticated user flagged as staff.
pub struct IsStaff;

#[async_trait::async_trait]
impl Permission for IsStaff {
    async fn has_permission(&self, req: &Request) -> bool {
        current_user(req).await.is_some_and(|user| user.is_staff)
    }
}

/// Requires an authenticated superuser.
pub struct IsSuperuser;

#[async_trait::async_trait]
impl Permission for IsSuperuser {
    async fn has_permission(&self, req: &Request) -> bool {
        use djangors_auth::AuthUser;
        current_user(req)
            .await
            .is_some_and(|user| user.is_superuser())
    }
}

/// Permits only non-mutating requests (`GET`, `HEAD`, `OPTIONS`).
///
/// On its own this makes an endpoint read-only for everyone. Combined with
/// another policy it expresses the common "anyone may read, only some may
/// write" rule:
///
/// ```no_run
/// # use djangors_rest::{IsReadOnly, IsStaff, PermissionExt};
/// // Reads are open; writes require staff.
/// let policy = IsReadOnly.or(IsStaff);
/// ```
pub struct IsReadOnly;

#[async_trait::async_trait]
impl Permission for IsReadOnly {
    async fn has_permission(&self, req: &Request) -> bool {
        matches!(
            *req.method(),
            hyper::http::Method::GET | hyper::http::Method::HEAD | hyper::http::Method::OPTIONS
        )
    }
}

/// Grants access only when both policies do. See [`PermissionExt::and`].
pub struct And<A, B>(pub A, pub B);

#[async_trait::async_trait]
impl<A: Permission, B: Permission> Permission for And<A, B> {
    async fn has_permission(&self, req: &Request) -> bool {
        self.0.has_permission(req).await && self.1.has_permission(req).await
    }
}

/// Grants access when either policy does. See [`PermissionExt::or`].
pub struct Or<A, B>(pub A, pub B);

#[async_trait::async_trait]
impl<A: Permission, B: Permission> Permission for Or<A, B> {
    async fn has_permission(&self, req: &Request) -> bool {
        self.0.has_permission(req).await || self.1.has_permission(req).await
    }
}

/// Inverts a policy. See [`PermissionExt::negate`].
pub struct Not<P>(pub P);

#[async_trait::async_trait]
impl<P: Permission> Permission for Not<P> {
    async fn has_permission(&self, req: &Request) -> bool {
        !self.0.has_permission(req).await
    }
}

/// Combinators for building a composite policy out of simple ones.
pub trait PermissionExt: Permission + Sized {
    /// Require both this policy and `other`.
    fn and<B: Permission>(self, other: B) -> And<Self, B> {
        And(self, other)
    }

    /// Require either this policy or `other`.
    fn or<B: Permission>(self, other: B) -> Or<Self, B> {
        Or(self, other)
    }

    /// Invert this policy.
    fn negate(self) -> Not<Self> {
        Not(self)
    }
}

impl<P: Permission + Sized> PermissionExt for P {}

/// A [`djangors_core::RateLimitKey`] strategy that keys by the currently authenticated user's
/// id (checking session-based [`Auth`](djangors_auth::Auth) first, then [`TokenAuth`], mirroring
/// [`IsAuthenticated`]'s own dual check). Rejects unauthenticated requests with
/// [`DjangorsError::Unauthorized`] rather than falling back to a shared/empty key.
///
/// This lives here rather than in `djangors-core` because it needs `djangors-auth`, which
/// depends on `djangors-core` — `djangors-core` itself cannot depend back on `djangors-auth`
/// without a dependency cycle.
pub struct ByAuthenticatedUser;

#[async_trait::async_trait]
impl djangors_core::RateLimitKey for ByAuthenticatedUser {
    async fn key(&self, req: &Request) -> Result<String, DjangorsError> {
        if let Ok(auth) = djangors_auth::Auth::<djangors_auth::User>::from_request(req).await {
            return Ok(auth.0.id().to_string());
        }
        if let Ok(token_auth) = TokenAuth::from_request(req).await {
            return Ok(token_auth.0.id().to_string());
        }
        Err(DjangorsError::Unauthorized("not authenticated".to_string()))
    }
}
