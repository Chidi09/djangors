//! Authentication, users, and permissions for Djangors.
//!
//! Rust-idiomatic equivalent of Django's `AUTH_USER_MODEL` swap is compile-time genericity
//! over the `AuthUser` trait, not a settings string.

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use djangors_macros::Model;
use thiserror::Error;

/// Error type for authentication and user operations.
#[derive(Error, Debug)]
pub enum AuthError {
    #[error("password hashing error: {0}")]
    Hashing(String),
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
