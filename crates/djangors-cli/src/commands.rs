//! Stub implementations for all dj subcommands.

use std::path::PathBuf;

fn validate_project_name(name: &str) -> Result<(), String> {
    let path = std::path::Path::new(name);
    let crate_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid project name '{name}'"))?;

    if crate_name.is_empty() {
        return Err("project name cannot be empty".to_string());
    }
    if !crate_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "invalid project name '{crate_name}': project names may only contain ASCII letters, numbers, underscores, and hyphens"
        ));
    }
    if crate_name.chars().next().unwrap().is_ascii_digit() {
        return Err(format!(
            "invalid project name '{crate_name}': project name cannot start with a digit"
        ));
    }
    Ok(())
}

fn djangors_crates_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("djangors-cli has a parent directory")
        .to_path_buf()
}

/// Create a new Djangors project.
pub fn new(name: &str) -> Result<(), String> {
    let target_dir = std::path::Path::new(name);
    let crate_name = target_dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid project name '{name}'"))?;

    validate_project_name(name)?;

    if target_dir.exists() {
        return Err(format!("destination '{name}' already exists"));
    }

    let crates_dir = djangors_crates_dir();
    let crates_dir_str = crates_dir.to_string_lossy();

    let cargo_toml = format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "{crate_name}"
path = "src/main.rs"

[dependencies]
djangors-core = {{ path = "{crates_dir_str}/djangors-core" }}
tokio = {{ version = "1", features = ["full"] }}
tower = {{ version = "0.5", features = ["util"] }}
"#
    );

    let main_rs = r#"use djangors_core::{Djangors, DjangorsError, DjangorsSettings, Router};

mod views;

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    djangors_core::logging::init_dev_logging();

    let (settings, warnings) = DjangorsSettings::load()?;
    for w in warnings {
        eprintln!("settings warning: {w}");
    }

    let router = Router::new().get("/", views::welcome);
    let router_service = djangors_core::router::RouterService::new(router, settings.debug);

    let service = tower::ServiceBuilder::new()
        .layer(djangors_core::middleware::security_headers_layer())
        .service(router_service);

    Djangors::new(settings, Router::new()).run_service(service).await
}
"#;

    let views_rs = r#"use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};

pub async fn welcome(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::html(
        StatusCode::OK,
        "<!DOCTYPE html><html><head><title>Djangors</title></head><body>\
<h1>It worked!</h1><p>Congratulations on your first Djangors project.</p>\
</body></html>"
            .to_string(),
    ))
}
"#;

    let gitignore = "/target\n*.env\n";

    let djangors_toml = "debug = true\n";

    let readme = format!(
        r#"# {crate_name}

A Djangors project.

## Run it

    cargo run

Then visit http://127.0.0.1:8000/.

## Next steps

- Add models: see `examples/school` in the djangors repo for a real worked example
  (`#[derive(Model)]`, `AdminSite`, migrations-free `CREATE TABLE` in tests).
- Add the admin: `djangors-admin`'s `AdminSite` needs a `djangors_db::Database` in router state —
  see `examples/polls/src/main.rs` for the full wiring (DB connection, sessions, CSRF).
"#
    );

    std::fs::create_dir_all(target_dir.join("src"))
        .map_err(|e| format!("failed to create directory '{name}/src': {e}"))?;

    std::fs::write(target_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("failed to write '{name}/Cargo.toml': {e}"))?;

    std::fs::write(target_dir.join("src/main.rs"), main_rs)
        .map_err(|e| format!("failed to write '{name}/src/main.rs': {e}"))?;

    std::fs::write(target_dir.join("src/views.rs"), views_rs)
        .map_err(|e| format!("failed to write '{name}/src/views.rs': {e}"))?;

    std::fs::write(target_dir.join(".gitignore"), gitignore)
        .map_err(|e| format!("failed to write '{name}/.gitignore': {e}"))?;

    std::fs::write(target_dir.join("djangors.toml"), djangors_toml)
        .map_err(|e| format!("failed to write '{name}/djangors.toml': {e}"))?;

    std::fs::write(target_dir.join("README.md"), readme)
        .map_err(|e| format!("failed to write '{name}/README.md': {e}"))?;

    let git_res = std::process::Command::new("git")
        .arg("init")
        .current_dir(target_dir)
        .output();

    if git_res.is_err() || !git_res.as_ref().unwrap().status.success() {
        eprintln!(
            "warning: 'git init' failed (is git installed?), skipping — the project files were still created successfully"
        );
    }

    println!("[dj new] created '{name}'");
    println!("[dj new] next: cd {name} && cargo run");

    Ok(())
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

pub async fn createpermissions() {
    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("[dj createpermissions] DATABASE_URL environment variable is not set");
            std::process::exit(1);
        }
    };
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = match djangors_db::Database::connect(&config).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("[dj createpermissions] failed to connect to database: {e}");
            std::process::exit(1);
        }
    };
    match djangors_auth::sync_permissions(&db).await {
        Ok(count) => println!("[dj createpermissions] synced {} permission(s)", count),
        Err(e) => {
            eprintln!("[dj createpermissions] failed: {e}");
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

    #[test]
    fn test_new_generates_expected_files() {
        let temp_dir = std::env::temp_dir().join(format!("dj_test_new_gen_{}", std::process::id()));
        let project_path = temp_dir.join("my_test_proj");

        let _ = std::fs::remove_dir_all(&project_path);
        let _ = std::fs::remove_dir_all(&temp_dir);

        let res = new(project_path.to_str().unwrap());
        assert!(res.is_ok(), "new() failed: {:?}", res);

        assert!(project_path.join("Cargo.toml").exists());
        assert!(project_path.join("src/main.rs").exists());
        assert!(project_path.join("src/views.rs").exists());
        assert!(project_path.join(".gitignore").exists());
        assert!(project_path.join("djangors.toml").exists());
        assert!(project_path.join("README.md").exists());

        let cargo_toml_content = std::fs::read_to_string(project_path.join("Cargo.toml")).unwrap();
        assert!(cargo_toml_content.contains("name = \"my_test_proj\""));
        assert!(cargo_toml_content.contains("djangors-core = { path = "));

        let path_line = cargo_toml_content
            .lines()
            .find(|l| l.contains("djangors-core ="))
            .expect("Cargo.toml missing djangors-core line");
        let start_quote = path_line.find('"').unwrap();
        let end_quote = path_line.rfind('"').unwrap();
        let extracted_core_path = &path_line[start_quote + 1..end_quote];

        assert!(
            std::path::Path::new(extracted_core_path)
                .join("Cargo.toml")
                .exists(),
            "Extracted djangors-core path '{}' does not exist",
            extracted_core_path
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_new_rejects_existing_directory() {
        let temp_dir =
            std::env::temp_dir().join(format!("dj_test_new_exists_{}", std::process::id()));
        let project_path = temp_dir.join("existing_proj");

        let _ = std::fs::remove_dir_all(&project_path);
        let _ = std::fs::remove_dir_all(&temp_dir);

        std::fs::create_dir_all(&project_path).unwrap();

        let res = new(project_path.to_str().unwrap());
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.contains("already exists"), "Error message: {err}");

        let entry_count = std::fs::read_dir(&project_path).unwrap().count();
        assert_eq!(
            entry_count, 0,
            "Directory should remain empty after rejection"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_new_invalid_project_name() {
        assert!(new("123invalid").is_err());
        assert!(new("invalid name!").is_err());
        assert!(new("").is_err());
    }

    /// End-to-end proof that a generated project compiles and serves the welcome page over HTTP.
    /// Run explicitly via `cargo test -p djangors-cli -- --ignored`.
    #[test]
    #[ignore]
    fn test_new_generated_project_builds_and_serves() {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::time::{Duration, Instant};

        let temp_dir = std::env::temp_dir().join(format!("dj_test_e2e_{}", std::process::id()));
        let project_path = temp_dir.join("test_e2e_serves");

        let _ = std::fs::remove_dir_all(&project_path);
        let _ = std::fs::remove_dir_all(&temp_dir);

        let res = new(project_path.to_str().unwrap());
        assert!(res.is_ok(), "new() failed: {:?}", res);

        let build_status = std::process::Command::new("cargo")
            .args(["build", "--offline"])
            .current_dir(&project_path)
            .status()
            .expect("failed to run cargo build on generated project");
        assert!(
            build_status.success(),
            "cargo build failed for generated project"
        );

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind port");
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let bin_path = project_path
            .join("target")
            .join("debug")
            .join("test_e2e_serves");
        let mut child = std::process::Command::new(&bin_path)
            .env("DJANGORS_PORT", port.to_string())
            .spawn()
            .expect("failed to spawn generated binary");

        let start = Instant::now();
        let mut stream_opt = None;
        while start.elapsed() < Duration::from_secs(15) {
            if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
                stream_opt = Some(stream);
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let mut stream = match stream_opt {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Generated server did not start and bind to port {port} within 15 seconds");
            }
        };

        let request =
            format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .expect("failed to send request");

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("failed to read response");

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&temp_dir);

        assert!(
            response.contains("200 OK"),
            "Expected 200 OK, got: {response}"
        );
        assert!(
            response.contains("It worked!"),
            "Expected 'It worked!', got: {response}"
        );
    }
}
