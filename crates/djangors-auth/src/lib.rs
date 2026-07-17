//! Authentication, users, and permissions for Djangors.
//!
//! Rust-idiomatic equivalent of Django's `AUTH_USER_MODEL` swap is compile-time genericity
//! over the `AuthUser` trait, not a settings string.

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use djangors_macros::Model;
use djangors_orm::Model as _;
use thiserror::Error;

/// Error type for authentication and user operations.
#[derive(Error, Debug)]
pub enum AuthError {
    #[error("password hashing error: {0}")]
    Hashing(String),
    #[error("database error: {0}")]
    Database(#[from] djangors_orm::OrmError),
}

const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$dGVzdHBhc3N3b3Jk";

#[async_trait::async_trait]
pub trait AuthBackend {
    type User: AuthUser;

    async fn authenticate(
        &self,
        db: &djangors_db::Database,
        username: &str,
        password: &str,
    ) -> Result<Option<Self::User>, AuthError>;
}

pub struct ModelBackend;

#[async_trait::async_trait]
impl AuthBackend for ModelBackend {
    type User = User;

    async fn authenticate(
        &self,
        db: &djangors_db::Database,
        username: &str,
        password: &str,
    ) -> Result<Option<User>, AuthError> {
        let mut users = User::objects()
            .filter(djangors_orm::q!(username = username))?
            .all(db)
            .await?;

        let user_opt = if users.len() == 1 {
            Some(users.remove(0))
        } else {
            None
        };

        let hash_to_verify = match &user_opt {
            Some(u) => u.password_hash(),
            None => DUMMY_HASH,
        };

        let verified = verify_password(password, hash_to_verify)?;

        if let Some(user) = user_opt {
            if verified && user.is_active() {
                return Ok(Some(user));
            }
        }

        Ok(None)
    }
}

pub const SESSION_USER_ID_KEY: &str = "_auth_user_id";

/// Establish an authenticated session for `user`. Rotates the session
/// identity first (session-fixation protection: an attacker who fixed a
/// session ID before login gains nothing from the now-authenticated
/// session, since its identity changed), then stores the user's id.
pub fn login<U: AuthUser>(session: &djangors_sessions::Session, user: &U) {
    session.cycle_key();
    session.set(SESSION_USER_ID_KEY, user.id());
}

/// Clear the authenticated session. `Session::clear()` already regenerates
/// the session's internal key as part of clearing (see
/// crates/djangors-sessions/src/lib.rs's `clear()`), so this alone already
/// gives logout a fresh, unrelated session identity - no separate
/// `cycle_key()` call needed here.
pub fn logout(session: &djangors_sessions::Session) {
    session.clear();
}

/// Extracts the currently-authenticated user of type `U`.
#[derive(Debug, Clone)]
pub struct Auth<U: AuthUser>(pub U);

#[async_trait::async_trait]
impl<U: AuthUser> djangors_core::extract::FromRequest for Auth<U> {
    async fn from_request(
        req: &djangors_core::Request,
    ) -> Result<Self, djangors_core::DjangorsError> {
        let session = req.ext::<djangors_sessions::Session>().ok_or_else(|| {
            djangors_core::error::DjangorsError::Unauthorized(
                "session extension absent".to_string(),
            )
        })?;

        let user_id = session.get::<i64>(SESSION_USER_ID_KEY).ok_or_else(|| {
            djangors_core::error::DjangorsError::Unauthorized("not authenticated".to_string())
        })?;

        let db = req.state::<djangors_db::Database>().ok_or_else(|| {
            djangors_core::error::DjangorsError::Internal("database state absent".to_string())
        })?;

        let filter_expr = djangors_orm::q!(id = user_id);

        let user = U::objects()
            .filter(filter_expr)
            .map_err(|e| djangors_core::error::DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| match e {
                djangors_orm::OrmError::NotFound { .. } => {
                    djangors_core::error::DjangorsError::Unauthorized("user not found".to_string())
                }
                _ => djangors_core::error::DjangorsError::Internal(e.to_string()),
            })?;

        if !user.is_active() {
            return Err(djangors_core::error::DjangorsError::Unauthorized(
                "account inactive".to_string(),
            ));
        }

        Ok(Auth(user))
    }
}

/// Anything djangors-auth's login/session/permission machinery can operate on.
/// Real apps that need custom user fields implement this on their own struct instead of using the built-in `User`.
pub trait AuthUser: djangors_orm::Model + djangors_orm::FromRow + Send + Sync + 'static {
    fn id(&self) -> i64;
    fn username(&self) -> &str;
    fn password_hash(&self) -> &str;
    fn set_password_hash(&mut self, hash: String);
    fn is_active(&self) -> bool;
}

/// The default concrete `User` model implementing `AuthUser`.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_user")]
pub struct User {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 150)]
    pub username: String,

    #[djangors(max_length = 254)]
    pub email: String,

    /// PHC-format hash string (algorithm + params + salt + hash all
    /// self-contained) - never the plaintext password.
    pub password: String,

    pub is_active: bool,
    pub is_staff: bool,
    pub is_superuser: bool,

    pub date_joined: chrono::DateTime<chrono::Utc>,
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
}

impl AuthUser for User {
    fn id(&self) -> i64 {
        self.id
    }

    fn username(&self) -> &str {
        &self.username
    }

    fn password_hash(&self) -> &str {
        &self.password
    }

    fn set_password_hash(&mut self, hash: String) {
        self.password = hash;
    }

    fn is_active(&self) -> bool {
        self.is_active
    }
}

/// Hash a plaintext password using Argon2id and generate a random salt.
/// Returns a PHC-formatted hash string.
///
/// For legacy verifiers (e.g. supporting old hash formats when migrating an existing user base),
/// this function or a future `PasswordHasher` trait can be extended. For now, it defaults to Argon2id.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AuthError::Hashing(e.to_string()))?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a PHC-formatted Argon2id hash.
/// Returns `Ok(true)` if valid, `Ok(false)` if the password doesn't match,
/// or `Err(AuthError)` if the hash string is malformed or corrupted.
pub fn verify_password(password: &str, phc_hash: &str) -> Result<bool, AuthError> {
    let parsed = PasswordHash::new(phc_hash).map_err(|e| AuthError::Hashing(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests;
