use super::*;
use djangors_orm::q;
use djangors_orm::Model;

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
async fn test_user_db_round_trip() {
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
