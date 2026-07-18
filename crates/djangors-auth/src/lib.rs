//! Authentication, users, and permissions for Djangors.
//!
//! Rust-idiomatic equivalent of Django's `AUTH_USER_MODEL` swap is compile-time genericity
//! over the `AuthUser` trait, not a settings string.

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use base64::Engine as _;
use djangors_core::signals::Signal;
use djangors_macros::Model;
use djangors_orm::ForeignKey;
use djangors_orm::Model as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

/// Error type for authentication and user operations.
#[derive(Error, Debug)]
pub enum AuthError {
    #[error("password hashing error: {0}")]
    Hashing(String),
    #[error("database error: {0}")]
    Database(#[from] djangors_orm::OrmError),
    #[error("too many login attempts, try again later")]
    RateLimited,
    #[error("invalid or expired token")]
    InvalidToken,
    #[error("mail sending error: {0}")]
    Mail(String),
}

const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$dGVzdHBhc3N3b3Jk";

#[derive(Debug, Clone)]
pub struct LoginSucceeded {
    pub user_id: i64,
    pub username: String,
}

/// Payload for the [`LOGIN_FAILED`] signal.
///
/// NOTE: The `username` string is attacker-controlled input. It must never be
/// used unescaped in any context where that could pose a security risk (e.g. html).
#[derive(Debug, Clone)]
pub struct LoginFailed {
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct LoggedOut {
    pub user_id: Option<i64>,
}

pub static LOGIN_SUCCEEDED: LazyLock<Signal<LoginSucceeded>> = LazyLock::new(Signal::new);
pub static LOGIN_FAILED: LazyLock<Signal<LoginFailed>> = LazyLock::new(Signal::new);
pub static LOGGED_OUT: LazyLock<Signal<LoggedOut>> = LazyLock::new(Signal::new);

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
                LOGIN_SUCCEEDED
                    .send(LoginSucceeded {
                        user_id: user.id(),
                        username: username.to_string(),
                    })
                    .await;
                return Ok(Some(user));
            }
        }

        LOGIN_FAILED
            .send(LoginFailed {
                username: username.to_string(),
            })
            .await;
        Ok(None)
    }
}

/// A single-process, in-memory sliding-window login rate limiter around an
/// inner [`AuthBackend`]. State lives only in this process's memory, so it
/// does not coordinate across multiple app instances/processes — a
/// distributed (e.g. cache-backed) limiter is future work once a shared
/// cache crate exists, not this v1.
pub struct RateLimitedBackend<B: AuthBackend> {
    inner: B,
    limiter: Mutex<HashMap<String, Vec<Instant>>>,
    max_attempts: u32,
    window: Duration,
}

impl<B: AuthBackend> RateLimitedBackend<B> {
    pub fn new(inner: B, max_attempts: u32, window: Duration) -> Self {
        Self {
            inner,
            limiter: Mutex::new(HashMap::new()),
            max_attempts,
            window,
        }
    }

    pub fn default_login_throttle(inner: B) -> Self {
        Self::new(inner, 5, Duration::from_secs(15 * 60))
    }
}

#[async_trait::async_trait]
impl<B: AuthBackend + Send + Sync> AuthBackend for RateLimitedBackend<B> {
    type User = B::User;

    async fn authenticate(
        &self,
        db: &djangors_db::Database,
        username: &str,
        password: &str,
    ) -> Result<Option<Self::User>, AuthError> {
        let now = Instant::now();
        let limit_exceeded = {
            let mut limiter = self.limiter.lock().unwrap();
            let attempts = limiter.entry(username.to_string()).or_default();
            attempts.retain(|&t| now.duration_since(t) < self.window);

            if attempts.len() as u32 >= self.max_attempts {
                true
            } else {
                attempts.push(now);
                false
            }
        };

        if limit_exceeded {
            LOGIN_FAILED
                .send(LoginFailed {
                    username: username.to_string(),
                })
                .await;
            return Err(AuthError::RateLimited);
        }

        self.inner.authenticate(db, username, password).await
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
pub async fn logout(session: &djangors_sessions::Session) {
    let user_id = session.get::<i64>(SESSION_USER_ID_KEY);
    session.clear();
    LOGGED_OUT.send(LoggedOut { user_id }).await;
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
#[async_trait::async_trait]
pub trait AuthUser: djangors_orm::Model + djangors_orm::FromRow + Send + Sync + 'static {
    fn id(&self) -> i64;
    fn username(&self) -> &str;
    fn password_hash(&self) -> &str;
    fn set_password_hash(&mut self, hash: String);
    fn is_active(&self) -> bool;
    fn is_superuser(&self) -> bool;
    async fn update_user(&self, db: &djangors_db::Database) -> Result<(), djangors_orm::OrmError>;
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

#[async_trait::async_trait]
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

    fn is_superuser(&self) -> bool {
        self.is_superuser
    }

    async fn update_user(&self, db: &djangors_db::Database) -> Result<(), djangors_orm::OrmError> {
        self.update(db).await
    }
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_permission")]
pub struct Permission {
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// "{app_label}.{action}_{model_name_lowercase}", e.g. "polls.add_question".
    #[djangors(max_length = 255, unique)]
    pub codename: String,
    /// Human-readable label, e.g. "Can add question".
    #[djangors(max_length = 255)]
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_group")]
pub struct Group {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 150, unique)]
    pub name: String,
}

/// Join table: which groups a user belongs to.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_user_groups")]
pub struct UserGroup {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub user: ForeignKey<User>,
    pub group: ForeignKey<Group>,
}

/// Join table: which permissions a group grants.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_group_permissions")]
pub struct GroupPermission {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub group: ForeignKey<Group>,
    pub permission: ForeignKey<Permission>,
}

/// Join table: permissions granted directly to a user (bypassing any group).
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_user_permissions")]
pub struct UserPermission {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub user: ForeignKey<User>,
    pub permission: ForeignKey<Permission>,
}

/// Checks whether `user_id` has been granted `codename`, either directly
/// (`UserPermission`) or via any group they belong to (`UserGroup` ->
/// `GroupPermission`). Does NOT special-case superusers - callers that want
/// "superusers can do anything" should check `user.is_superuser()` first and
/// skip calling this entirely, avoiding a DB round-trip for the common case.
pub async fn has_perm(
    db: &djangors_db::Database,
    user_id: i64,
    codename: &str,
) -> Result<bool, AuthError> {
    let direct: i64 = djangors_orm::sqlx::query_scalar(djangors_orm::sqlx::AssertSqlSafe(
        "SELECT COUNT(*) FROM auth_user_permissions up \
         JOIN auth_permission p ON p.id = up.permission \
         WHERE up.\"user\" = $1 AND p.codename = $2"
            .to_string(),
    ))
    .bind(user_id)
    .bind(codename)
    .fetch_one(db.pool())
    .await
    .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    if direct > 0 {
        return Ok(true);
    }

    let via_group: i64 = djangors_orm::sqlx::query_scalar(djangors_orm::sqlx::AssertSqlSafe(
        "SELECT COUNT(*) FROM auth_user_groups ug \
         JOIN auth_group_permissions gp ON gp.\"group\" = ug.\"group\" \
         JOIN auth_permission p ON p.id = gp.permission \
         WHERE ug.\"user\" = $1 AND p.codename = $2"
            .to_string(),
    ))
    .bind(user_id)
    .bind(codename)
    .fetch_one(db.pool())
    .await
    .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    Ok(via_group > 0)
}

/// Ensures the 4 standard permissions (view/add/change/delete) exist for
/// every model currently registered via `djangors_orm::meta::all_registered_models()`.
/// This covers every `#[derive(Model)]` struct in the binary, using the same pre-existing
/// global registry that 5.5's `collect_related_objects` already established as
/// the source of truth for "every model in the project." Idempotent: safe
/// to call on every app startup or as a repeatable CLI step. Returns the
/// number of (model, action) pairs considered (not the number newly
/// inserted - `ON CONFLICT DO NOTHING` makes re-runs cheap no-ops without
/// needing to track that distinction).
pub async fn sync_permissions(db: &djangors_db::Database) -> Result<usize, AuthError> {
    let mut count = 0;
    for meta in djangors_orm::meta::all_registered_models() {
        for action in ["view", "add", "change", "delete"] {
            let codename = format!(
                "{}.{}_{}",
                meta.app_label,
                action,
                meta.struct_name.to_lowercase()
            );
            let name = format!("Can {} {}", action, meta.struct_name.to_lowercase());
            djangors_orm::sqlx::query(djangors_orm::sqlx::AssertSqlSafe(
                "INSERT INTO auth_permission (codename, name) VALUES ($1, $2) \
                 ON CONFLICT (codename) DO NOTHING"
                    .to_string(),
            ))
            .bind(&codename)
            .bind(&name)
            .execute(db.pool())
            .await
            .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
            count += 1;
        }
    }
    Ok(count)
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

/// Generates a signed password reset token.
///
/// The token embeds the user's ID, an expiry timestamp, and a prefix of the user's current password hash.
/// This ensures that the token becomes invalid as soon as the user's password changes.
pub fn generate_password_reset_token<U: AuthUser>(
    user: &U,
    secret: &[u8],
    ttl: Duration,
) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let expiry_unix_secs = (now + ttl).as_secs();

    let user_id_str = user.id().to_string();
    let b64_user_id = base64::engine::general_purpose::STANDARD.encode(user_id_str.as_bytes());
    let b64_expiry =
        base64::engine::general_purpose::STANDARD.encode(expiry_unix_secs.to_string().as_bytes());

    let hash = user.password_hash();
    let prefix_len = std::cmp::min(30, hash.len());
    let password_hash_prefix = &hash[..prefix_len];

    let msg = format!(
        "{}.{}.{}",
        user.id(),
        expiry_unix_secs,
        password_hash_prefix
    );

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(msg.as_bytes());
    let mac_result = mac.finalize().into_bytes();
    let b64_mac = base64::engine::general_purpose::STANDARD.encode(mac_result);

    format!("{}.{}.{}", b64_user_id, b64_expiry, b64_mac)
}

/// Verifies a signed password reset token against a user.
///
/// Returns `true` if the token is valid, has not expired, was generated for the correct user,
/// and the user's password hash has not changed since the token was generated.
pub fn verify_password_reset_token<U: AuthUser>(user: &U, token: &str, secret: &[u8]) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    let b64_user_id = parts[0];
    let b64_expiry = parts[1];
    let b64_mac = parts[2];

    // Decode and verify user_id
    let user_id_bytes = match base64::engine::general_purpose::STANDARD.decode(b64_user_id) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let user_id_str = match std::str::from_utf8(&user_id_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let token_user_id: i64 = match user_id_str.parse() {
        Ok(id) => id,
        Err(_) => return false,
    };
    if token_user_id != user.id() {
        return false;
    }

    // Decode and verify expiry
    let expiry_bytes = match base64::engine::general_purpose::STANDARD.decode(b64_expiry) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let expiry_str = match std::str::from_utf8(&expiry_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let expiry_unix_secs: u64 = match expiry_str.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    };
    if expiry_unix_secs <= now {
        return false;
    }

    // Verify HMAC using the user's current password hash prefix
    let hash = user.password_hash();
    let prefix_len = std::cmp::min(30, hash.len());
    let password_hash_prefix = &hash[..prefix_len];

    let msg = format!(
        "{}.{}.{}",
        user.id(),
        expiry_unix_secs,
        password_hash_prefix
    );

    let mac_bytes = match base64::engine::general_purpose::STANDARD.decode(b64_mac) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(msg.as_bytes());
    mac.verify_slice(&mac_bytes).is_ok()
}

/// Looks up an active user by email and, if found, sends a password reset link to them.
///
/// To prevent user enumeration attacks, this function always returns `Ok(())` and runs in
/// a similar time frame regardless of whether the email was registered.
pub async fn request_password_reset<U: AuthUser>(
    db: &djangors_db::Database,
    mail: &dyn djangors_mail::MailBackend,
    email: &str,
    secret: &[u8],
    reset_link_base: &str,
) -> Result<(), AuthError> {
    let mut users = U::objects()
        .filter(djangors_orm::q!(email = email))?
        .all(db)
        .await?;

    let user_opt = if users.len() == 1 {
        let u = users.remove(0);
        if u.is_active() {
            Some(u)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(user) = user_opt {
        // Use a default 1-hour TTL for the token.
        let token = generate_password_reset_token(&user, secret, Duration::from_secs(3600));
        let reset_link = format!("{}{}", reset_link_base, token);
        let message = djangors_mail::Message {
            to: vec![email.to_string()],
            from: "noreply@localhost".to_string(),
            subject: "Password Reset Request".to_string(),
            body: format!(
                "You are receiving this email because you requested a password reset for your user account.\n\
                 Please click the following link to choose a new password:\n\n\
                 {}\n\n\
                 If you did not request this reset, you can safely ignore this email.",
                reset_link
            ),
        };
        mail.send(&message)
            .await
            .map_err(|e| AuthError::Mail(e.to_string()))?;
    }

    Ok(())
}

/// Confirms the password reset by verifying the token, hashing the new password, and updating the database.
pub async fn confirm_password_reset<U: AuthUser>(
    db: &djangors_db::Database,
    user_id: i64,
    token: &str,
    new_password: &str,
    secret: &[u8],
) -> Result<(), AuthError> {
    let mut users = U::objects()
        .filter(djangors_orm::q!(id = user_id))?
        .all(db)
        .await?;

    if users.len() != 1 {
        return Err(AuthError::InvalidToken);
    }
    let mut user = users.remove(0);

    if !verify_password_reset_token(&user, token, secret) {
        return Err(AuthError::InvalidToken);
    }

    let hashed = hash_password(new_password)?;
    user.set_password_hash(hashed);
    user.update_user(db).await?;

    Ok(())
}

#[cfg(test)]
mod tests;
