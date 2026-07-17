//! Stub implementations for all dj subcommands.

use std::path::PathBuf;

/// Create a new Djangors project.
pub fn new(name: &str) {
    println!(
        "[dj new] would create project '{}' (not yet implemented)",
        name
    );
}

/// Create a new app within a project.
pub fn new_app(name: &str) {
    println!(
        "[dj new-app] would create app '{}' (not yet implemented)",
        name
    );
}

/// Start the dev server.
pub fn run(port: u16) {
    println!(
        "[dj run] would start the dev server on port {} (not yet implemented)",
        port
    );
}

/// Apply database migrations.
///
/// Reads the database connection URL from the `DATABASE_URL` environment
/// variable (the standard sqlx/Rust ecosystem convention) and runs
/// [`djangors_migrations::migrate`]. Currently a v1, CreateTable-only
/// engine — see `docs/design/4.3-migrations.md` for scope.
pub async fn migrate() {
    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("[dj migrate] DATABASE_URL environment variable is not set");
            std::process::exit(1);
        }
    };

    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = match djangors_db::Database::connect(&config).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("[dj migrate] failed to connect to database: {e}");
            std::process::exit(1);
        }
    };

    match djangors_migrations::migrate(&db).await {
        Ok(()) => println!("[dj migrate] migrations applied successfully"),
        Err(e) => {
            eprintln!("[dj migrate] migration failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Generate new migrations.
pub fn makemigrations(check: bool) {
    println!(
        "[dj makemigrations] would generate new migrations (check={}) (not yet implemented)",
        check
    );
}

/// Create a superuser in the database.
///
/// This function executes the core logic of creating a superuser:
/// 1. Checks if the username is already taken.
/// 2. Hashes the password using djangors_auth::hash_password.
/// 3. Saves the User model to the database.
pub async fn create_superuser_impl(
    db: &djangors_db::Database,
    username: &str,
    email: &str,
    password: &str,
) -> Result<(), String> {
    use djangors_orm::Model as _;

    let exists = djangors_auth::User::objects()
        .filter(djangors_orm::q!(username = username))
        .map_err(|e| e.to_string())?
        .exists(db)
        .await
        .map_err(|e| e.to_string())?;

    if exists {
        return Err(format!(
            "superuser with username '{}' already exists",
            username
        ));
    }

    let hash = djangors_auth::hash_password(password).map_err(|e| e.to_string())?;

    let user = djangors_auth::User {
        id: 0,
        username: username.to_string(),
        email: email.to_string(),
        password: hash,
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: chrono::Utc::now(),
        last_login: None,
    };

    user.save(db).await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Create an admin user.
///
/// Note: non-interactive only, mirrors Django's `DJANGO_SUPERUSER_PASSWORD` convention;
/// interactive prompting deferred.
pub async fn createsuperuser(username: String, email: String) {
    let password = match std::env::var("DJANGORS_SUPERUSER_PASSWORD") {
        Ok(val) if !val.is_empty() => val,
        _ => {
            eprintln!("[dj createsuperuser] DJANGORS_SUPERUSER_PASSWORD environment variable is not set or empty");
            std::process::exit(1);
        }
    };

    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("[dj createsuperuser] DATABASE_URL environment variable is not set");
            std::process::exit(1);
        }
    };

    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = match djangors_db::Database::connect(&config).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("[dj createsuperuser] failed to connect to database: {e}");
            std::process::exit(1);
        }
    };

    match create_superuser_impl(&db, &username, &email, &password).await {
        Ok(()) => {
            println!("Superuser created successfully: {}", username);
        }
        Err(e) => {
            eprintln!("[dj createsuperuser] failed to create superuser: {e}");
            std::process::exit(1);
        }
    }
}

/// Open a REPL.
pub fn shell() {
    println!("[dj shell] would open a REPL (not yet implemented)");
}

/// Run the test suite.
pub fn test() {
    println!("[dj test] would run the test suite (not yet implemented)");
}

/// Collect static files from source directories into one output directory.
pub fn collectstatic(source: Vec<PathBuf>, output: PathBuf) {
    let sources = if source.is_empty() {
        vec![PathBuf::from("static")]
    } else {
        source
    };

    let sf = djangors_staticfiles::StaticFiles::new(sources);
    match sf.collect(&output) {
        Ok(manifest) => {
            println!(
                "[dj collectstatic] collected {} file(s) into {}",
                manifest.mapping.len(),
                output.display()
            );
        }
        Err(e) => {
            eprintln!("[dj collectstatic] failed: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use djangors_auth::{verify_password, User};
    use djangors_db::Database;
    use djangors_orm::Model as _;

    static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_create_superuser_success() {
        let _guard = DB_MUTEX.lock().unwrap();
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = Database::connect(&config).await.unwrap();

        // Drop table if exists first
        let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        // Create table auth_user
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

        let username = "admin_test_user";
        let email = "admin_test@example.com";
        let password = "super_secure_password";

        let result = create_superuser_impl(&db, username, email, password).await;
        assert!(result.is_ok(), "Expected Ok, got Err: {:?}", result);

        // Fetch from database
        let user = User::objects()
            .filter(djangors_orm::q!(username = username))
            .unwrap()
            .get(&db)
            .await
            .unwrap();

        assert_eq!(user.username, username);
        assert_eq!(user.email, email);
        assert!(user.is_active);
        assert!(user.is_staff);
        assert!(user.is_superuser);

        // Verify password is correct using real auth verify function
        let is_valid = verify_password(password, &user.password).unwrap();
        assert!(is_valid, "Password verification failed");

        // Clean up
        let _ = djangors_orm::sqlx::query("DROP TABLE auth_user")
            .execute(db.pool())
            .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_create_superuser_duplicate_username() {
        let _guard = DB_MUTEX.lock().unwrap();
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = Database::connect(&config).await.unwrap();

        // Drop table if exists first
        let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;

        // Create table auth_user
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

        let username = "admin_test_dup";
        let email1 = "admin1@example.com";
        let email2 = "admin2@example.com";
        let password = "password";

        let result1 = create_superuser_impl(&db, username, email1, password).await;
        if let Err(ref e) = result1 {
            println!("result1 error: {:?}", e);
        }
        assert!(result1.is_ok());

        let result2 = create_superuser_impl(&db, username, email2, password).await;
        assert!(
            result2.is_err(),
            "Expected Err for duplicate username, got Ok"
        );
        let err_msg = result2.unwrap_err();
        assert!(
            err_msg.contains("already exists"),
            "Err message: {}",
            err_msg
        );

        // Clean up
        let _ = djangors_orm::sqlx::query("DROP TABLE auth_user")
            .execute(db.pool())
            .await;
    }
}
