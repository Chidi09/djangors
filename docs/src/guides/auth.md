# Authentication & Permissions

`djangors-auth` provides user management, authentication backends, session integration, Argon2id password hashing, and role-based permissions.

## Custom User Models & `AuthUser` Trait

In Djangors, custom user model swapping is achieved at compile time via the `AuthUser` trait (rather than runtime settings strings):

```rust,compile
# use djangors_orm::FromRow;
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
```

The crate provides a built-in concrete `User` model implementing `AuthUser`:

```rust,compile
use djangors_macros::Model;
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_user")]
pub struct User {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 150)]
    pub username: String,
    #[djangors(max_length = 254)]
    pub email: String,
    pub password: String, // PHC Argon2id hash string
    pub is_active: bool,
    pub is_staff: bool,
    pub is_superuser: bool,
    pub date_joined: chrono::DateTime<chrono::Utc>,
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
}
```

---

## Request Extraction (`Auth<U>`)

Extract the authenticated user in handlers using `Auth<User>`:

```rust,compile
use djangors_auth::{Auth, User};
use djangors_core::extract::FromRequest;
use djangors_core::{DjangorsError, Request, Response, StatusCode};

pub async fn profile_view(req: Request) -> Result<Response, DjangorsError> {
    // Extracts authenticated user from session; returns 401 Unauthorized if unauthenticated or inactive
    let Auth(user) = Auth::<User>::from_request(&req).await?;

    Ok(Response::text(StatusCode::OK, "Hello, authenticated user!"))
}
```

---

## Authentication Backends

Authentication is decoupled via the `AuthBackend` trait:

```rust,compile
# pub trait AuthUser: djangors_orm::Model + djangors_orm::FromRow + Send + Sync + 'static {}
# use djangors_auth::AuthError;
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
```

### `ModelBackend`
Authenticates against database user records using Argon2id password verification.
- Emits `LOGIN_SUCCEEDED` / `LOGIN_FAILED` signals.
- Runs a dummy password verification when username is missing to mitigate timing side-channel attacks.

### `RateLimitedBackend`
Wraps any `AuthBackend` to enforce sliding-window rate limiting on login attempts:

```rust,compile
# fn main() {
use djangors_auth::{ModelBackend, RateLimitedBackend};

// Limits to 5 failed login attempts per 15 minutes per username
let backend = RateLimitedBackend::default_login_throttle(ModelBackend);
# }
```

### `PersistentLockoutBackend` (the `django-axes` equivalent)

`RateLimitedBackend` throttles the *rate* of attempts, but a correct password made after the
window resets still succeeds. It doesn't lock the account. `PersistentLockoutBackend` is
different and complementary: after `max_attempts` consecutive failures, it rejects login attempts
with `AuthError::AccountLocked { retry_after_secs }` for `lockout_duration`, **even with the
correct password**. This state is stored in a real `auth_login_lockout` database table, so it
survives process restarts and is shared correctly across multiple app instances pointed at the
same database.

```rust,compile
# fn main() {
use djangors_auth::{ModelBackend, PersistentLockoutBackend};
use std::time::Duration;

// Lock an account for 1 hour after 5 consecutive failed attempts.
let backend = PersistentLockoutBackend::new(ModelBackend, 5, Duration::from_secs(3600));
# }
```

A successful login clears the account's failure streak entirely; an already-expired lockout is
treated as a fresh streak (starting from 1) rather than continuing to accumulate. The
`LoginLockout` model (`#[derive(Model)]`, table `auth_login_lockout`) is a normal registered
model, so `dj makemigrations` picks it up automatically like any other.

---

## Session Management (`login` / `logout`)

### `login`
Establishes an authenticated session for a user. Rotates session key (`session.cycle_key()`) to prevent session fixation attacks, then stores user ID under `_auth_user_id`:

```rust,compile
# async fn test_login() -> Result<(), Box<dyn std::error::Error>> {
# let session = djangors_sessions::Session::new_empty();
# let backend = djangors_auth::ModelBackend;
# let db = djangors_db::Database::connect(&djangors_db::config::DatabaseConfig::new("postgres://postgres:postgres@localhost/djangors_test")).await.unwrap();
# let username = ""; let password = "";
use djangors_auth::{login, AuthBackend};

if let Some(user) = backend.authenticate(&db, &username, &password).await? {
    login(&session, &user);
}
# Ok(())
# }
```

### `logout`
Clears session data (`session.clear()`), generating a fresh session key, and emits the `LOGGED_OUT` signal:

```rust,compile
# async fn test_logout() {
# let session = djangors_sessions::Session::new_empty();
use djangors_auth::logout;

logout(&session).await;
# }
```

---

## Permissions & Groups

`djangors-auth` provides permission management:

- **`Permission`**: Model for permissions with `codename` (`"{app_label}.{action}_{model}"`) and `name`.
- **`Group`**: Role groups (`name`).
- **Join Tables**: `UserGroup`, `GroupPermission`, `UserPermission`.

### Checking Permissions (`has_perm`)
```rust,compile
# async fn test_perm() -> Result<(), Box<dyn std::error::Error>> {
# let user = djangors_auth::User { id: 1, username: String::new(), email: String::new(), password: String::new(), is_active: true, is_staff: true, is_superuser: true, date_joined: chrono::Utc::now(), last_login: None };
# let db = djangors_db::Database::connect(&djangors_db::config::DatabaseConfig::new("postgres://postgres:postgres@localhost/djangors_test")).await.unwrap();
use djangors_auth::{has_perm, AuthUser};

// Superusers bypass perm checks manually or by calling user.is_superuser()
if user.is_superuser() || has_perm(&db, user.id(), "polls.add_question").await? {
    // User is authorized
}
# Ok(())
# }
```

### Permission Synchronization (`sync_permissions` & `dj createpermissions`)
`sync_permissions(&db)` scans all registered models (`all_registered_models()`) and generates the 4 standard permissions (`view`, `add`, `change`, `delete`) in `auth_permission`.

Run via CLI:
```bash
dj createpermissions
```

---

## Password Hashing & Password Reset

- **Password Hashing**: `hash_password(password)` and `verify_password(password, hash)` use Argon2id with random salts.
- **Password Reset**: `generate_password_reset_token`, `verify_password_reset_token`, `request_password_reset`, and `confirm_password_reset` provide HMAC-signed, time-bounded password reset tokens.
