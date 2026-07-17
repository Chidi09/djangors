use super::*;
use djangors_orm::q;
use djangors_orm::Model;

static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "test_auth_user")]
pub struct TestUser {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 150)]
    pub username: String,

    #[djangors(max_length = 254)]
    pub email: String,

    pub password: String,

    pub is_active: bool,
    pub is_staff: bool,
    pub is_superuser: bool,

    pub date_joined: chrono::DateTime<chrono::Utc>,
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
}

impl AuthUser for TestUser {
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

#[test]
fn test_user_derives_and_metadata() {
    let meta = User::meta();
    assert_eq!(meta.struct_name, "User");
    assert_eq!(meta.app_label, "djangors_auth");
    assert_eq!(meta.table_name, "auth_user");

    // Check that we have the expected number of fields
    assert_eq!(meta.fields.len(), 9);
}

#[test]
fn test_hash_password_properties() {
    let password = "my_secure_password";
    let hash1 = hash_password(password).expect("hashing should succeed");

    // Must start with $argon2id$
    assert!(
        hash1.starts_with("$argon2id$"),
        "Hash '{}' should start with $argon2id$",
        hash1
    );

    // Calling twice must produce different hashes (random salt)
    let hash2 = hash_password(password).expect("hashing should succeed");
    assert_ne!(
        hash1, hash2,
        "Hashes should be different due to random salt"
    );
}

#[test]
fn test_verify_password_correct_and_incorrect() {
    let password = "my_secure_password";
    let hash = hash_password(password).expect("hashing should succeed");

    // Correct password
    let result_correct = verify_password(password, &hash).expect("verification should run");
    assert!(result_correct);

    // Incorrect password
    let result_incorrect =
        verify_password("wrong_password", &hash).expect("verification should run");
    assert!(!result_incorrect);
}

#[test]
fn test_verify_password_malformed_hash() {
    // Malformed/corrupted hash string should return Err
    let malformed_hashes = vec![
        "not_a_hash",
        "$argon2id$v=19$m=4096,t=3,p=1$invalid_salt$invalid_hash",
        "",
    ];

    for hash in malformed_hashes {
        let result = verify_password("password", hash);
        assert!(
            result.is_err(),
            "Expected error for malformed hash '{}', got {:?}",
            hash,
            result
        );
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_user_db_round_trip() {
    let _guard = DB_MUTEX.lock().unwrap();
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_orm::djangors_db::config::DatabaseConfig::new(db_url);

    // Connect to database
    let db = match djangors_orm::djangors_db::Database::connect(&config).await {
        Ok(db) => db,
        Err(e) => {
            // If DB is unreachable, fail as expected in the environment
            panic!("Could not connect to database: {:?}", e);
        }
    };

    // Clean up test table
    djangors_orm::sqlx::query("DROP TABLE IF EXISTS test_auth_user")
        .execute(db.pool())
        .await
        .unwrap();

    // Create test table
    djangors_orm::sqlx::query(
        "CREATE TABLE test_auth_user (
            id BIGSERIAL PRIMARY KEY,
            username VARCHAR(150) NOT NULL,
            email VARCHAR(254) NOT NULL,
            password TEXT NOT NULL,
            is_active BOOLEAN NOT NULL,
            is_staff BOOLEAN NOT NULL,
            is_superuser BOOLEAN NOT NULL,
            date_joined TIMESTAMPTZ NOT NULL,
            last_login TIMESTAMPTZ
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // Construct a User with a hashed password
    let plaintext = "supersecret";
    let hash = hash_password(plaintext).unwrap();
    let now = chrono::Utc::now();

    let mut user = TestUser {
        id: 0,
        username: "testuser".to_string(),
        email: "test@example.com".to_string(),
        password: "".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    user.set_password_hash(hash);

    // Save user
    let saved_user = user.save(&db).await.unwrap();
    assert_ne!(saved_user.id, 0);
    assert_eq!(saved_user.username, "testuser");
    assert_eq!(saved_user.email, "test@example.com");
    assert!(saved_user.is_active);
    assert_eq!(saved_user.date_joined.timestamp(), now.timestamp());
    assert_eq!(saved_user.last_login.unwrap().timestamp(), now.timestamp());

    // Fetch user back
    let fetched_user = TestUser::objects()
        .filter(q!(id = saved_user.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();

    // Verify password against round-tripped password field
    let verified = verify_password(plaintext, fetched_user.password_hash()).unwrap();
    assert!(verified);

    // Cleanup
    djangors_orm::sqlx::query("DROP TABLE test_auth_user")
        .execute(db.pool())
        .await
        .unwrap();
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_model_backend_authenticate() {
    let _guard = DB_MUTEX.lock().unwrap();
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_orm::djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_orm::djangors_db::Database::connect(&config)
        .await
        .unwrap();

    djangors_orm::sqlx::query("DROP TABLE IF EXISTS auth_user")
        .execute(db.pool())
        .await
        .unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE auth_user (
            id BIGSERIAL PRIMARY KEY,
            username VARCHAR(150) NOT NULL,
            email VARCHAR(254) NOT NULL,
            password TEXT NOT NULL,
            is_active BOOLEAN NOT NULL,
            is_staff BOOLEAN NOT NULL,
            is_superuser BOOLEAN NOT NULL,
            date_joined TIMESTAMPTZ NOT NULL,
            last_login TIMESTAMPTZ
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let plaintext = "correct_password";
    let hash = hash_password(plaintext).unwrap();
    let now = chrono::Utc::now();

    // 1. Create an active user
    let active_user_raw = User {
        id: 0,
        username: "active_user".to_string(),
        email: "active@example.com".to_string(),
        password: hash.clone(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let _active_user = active_user_raw.save(&db).await.unwrap();

    // 2. Create an inactive user
    let inactive_user_raw = User {
        id: 0,
        username: "inactive_user".to_string(),
        email: "inactive@example.com".to_string(),
        password: hash.clone(),
        is_active: false,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let _ = inactive_user_raw.save(&db).await.unwrap();

    let backend = ModelBackend;

    // Test correct credentials -> Some(user)
    let auth_res = backend
        .authenticate(&db, "active_user", plaintext)
        .await
        .unwrap();
    assert!(auth_res.is_some());
    assert_eq!(auth_res.unwrap().username, "active_user");

    // Test wrong password -> None
    let auth_res = backend
        .authenticate(&db, "active_user", "wrong_password")
        .await
        .unwrap();
    assert!(auth_res.is_none());

    // Test nonexistent username -> None
    let auth_res = backend
        .authenticate(&db, "nonexistent", plaintext)
        .await
        .unwrap();
    assert!(auth_res.is_none());

    // Test inactive user with correct password -> None
    let auth_res = backend
        .authenticate(&db, "inactive_user", plaintext)
        .await
        .unwrap();
    assert!(auth_res.is_none());

    // Cleanup
    djangors_orm::sqlx::query("DROP TABLE auth_user")
        .execute(db.pool())
        .await
        .unwrap();
}

#[test]
fn test_login_session_mechanics() {
    let secret = b"super-secret-key-for-testing-purposes-only";
    let store = djangors_sessions::SignedCookieStore::new(secret);
    let session = djangors_sessions::Session::new_empty();

    let user = User {
        id: 42,
        username: "testuser".to_string(),
        email: "test@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: chrono::Utc::now(),
        last_login: None,
    };

    let cookie_val_before = store.encode(&session);

    login(&session, &user);

    let cookie_val_after = store.encode(&session);

    // Prove pre-rotation and post-rotation cookie strings are different (cycle_key logic)
    assert_ne!(cookie_val_before, cookie_val_after);

    // Verify session sets the _auth_user_id
    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), Some(42));
}

#[test]
fn test_logout_session_mechanics() {
    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, 42i64);
    session.set("other_key", "value".to_string());

    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), Some(42));

    logout(&session);

    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), None);
    assert!(session.is_empty());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_auth_extractor() {
    let _guard = DB_MUTEX.lock().unwrap();
    use bytes::Bytes;
    use djangors_core::extract::FromRequest;
    use djangors_core::Request;
    use hyper::http::{Extensions, HeaderMap, Method, Uri};

    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_orm::djangors_db::config::DatabaseConfig::new(db_url);
    let db = djangors_orm::djangors_db::Database::connect(&config)
        .await
        .unwrap();

    djangors_orm::sqlx::query("DROP TABLE IF EXISTS auth_user")
        .execute(db.pool())
        .await
        .unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE auth_user (
            id BIGSERIAL PRIMARY KEY,
            username VARCHAR(150) NOT NULL,
            email VARCHAR(254) NOT NULL,
            password TEXT NOT NULL,
            is_active BOOLEAN NOT NULL,
            is_staff BOOLEAN NOT NULL,
            is_superuser BOOLEAN NOT NULL,
            date_joined TIMESTAMPTZ NOT NULL,
            last_login TIMESTAMPTZ
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let now = chrono::Utc::now();
    let active_user_raw = User {
        id: 0,
        username: "alice".to_string(),
        email: "alice@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let active_user = active_user_raw.save(&db).await.unwrap();

    let inactive_user_raw = User {
        id: 0,
        username: "inactive".to_string(),
        email: "inactive@example.com".to_string(),
        password: "hash".to_string(),
        is_active: false,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let inactive_user = inactive_user_raw.save(&db).await.unwrap();

    // 1. no session extension present -> Err(Unauthorized)
    let req_no_session = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::default());
    let res = Auth::<User>::from_request(&req_no_session).await;
    assert!(res.is_err());
    assert!(matches!(
        res.err().unwrap(),
        djangors_core::error::DjangorsError::Unauthorized(_)
    ));

    // 2. session present but no _auth_user_id set -> Err(Unauthorized)
    let session = djangors_sessions::Session::new_empty();
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let req_empty_session = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(djangors_core::state::AppState::default());
    let res = Auth::<User>::from_request(&req_empty_session).await;
    assert!(res.is_err());
    assert!(matches!(
        res.err().unwrap(),
        djangors_core::error::DjangorsError::Unauthorized(_)
    ));

    // 3. valid _auth_user_id pointing at a real active user -> Ok(Auth(user))
    let app_state = djangors_core::state::AppState::new().insert(db.clone());

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, active_user.id);
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let req_valid = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(app_state.clone());
    let res = Auth::<User>::from_request(&req_valid).await;
    assert!(res.is_ok());
    assert_eq!(res.unwrap().0.username, "alice");

    // 4. valid _auth_user_id pointing at an inactive user -> Err(Unauthorized)
    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, inactive_user.id);
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let req_inactive = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(app_state.clone());
    let res = Auth::<User>::from_request(&req_inactive).await;
    assert!(res.is_err());
    assert!(matches!(
        res.err().unwrap(),
        djangors_core::error::DjangorsError::Unauthorized(_)
    ));

    // 5. valid _auth_user_id pointing at a since-deleted row -> Err(Unauthorized)
    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, 9999i64); // Nonexistent ID
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let req_deleted = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(app_state.clone());
    let res = Auth::<User>::from_request(&req_deleted).await;
    assert!(res.is_err());
    assert!(matches!(
        res.err().unwrap(),
        djangors_core::error::DjangorsError::Unauthorized(_)
    ));

    // Cleanup
    djangors_orm::sqlx::query("DROP TABLE auth_user")
        .execute(db.pool())
        .await
        .unwrap();
}
