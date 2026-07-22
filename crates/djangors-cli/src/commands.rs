//! Stub implementations for all dj subcommands.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub(crate) fn validate_project_name(name: &str) -> Result<(), String> {
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

pub(crate) fn djangors_crates_dir() -> std::path::PathBuf {
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
pub fn new_app(name: &str) -> Result<(), String> {
    if !Path::new("Cargo.toml").is_file() || !Path::new("djangors.toml").is_file() {
        return Err("not a Djangors project (expected Cargo.toml and djangors.toml)".into());
    }
    validate_project_name(name)?;
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("invalid app name '{name}': Rust module names may only contain ASCII letters, numbers, and underscores"));
    }
    if name.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        return Err(format!(
            "invalid app name '{name}': app names cannot start with a digit"
        ));
    }
    let dir = Path::new("src").join(name);
    if dir.exists() {
        return Err(format!("destination 'src/{name}' already exists"));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create '{}': {e}", dir.display()))?;
    let files = [
        (
            "mod.rs",
            "pub mod admin;\npub mod models;\npub mod views;\n".to_string(),
        ),
        (
            "models.rs",
            r#"//! Models for this app.
//!
//! Uncomment and adapt this example after adding the required model dependencies.
// use djangors_macros::Model;
//
// #[derive(Model, Debug, Clone)]
// #[djangors(app = "my_app", table_name = "my_app_student")]
// pub struct Student {
//     #[djangors(primary_key, auto)]
//     pub id: i64,
//     #[djangors(max_length = 100)]
//     pub first_name: String,
//     #[djangors(max_length = 100)]
//     pub last_name: String,
//     #[djangors(max_length = 254, unique)]
//     pub email: String,
//     pub is_active: bool,
// }
"#
            .to_string(),
        ),
        (
            "views.rs",
            r#"use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};

pub async fn index(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "Welcome to this Djangors app"))
}
"#
            .to_string(),
        ),
        (
            "admin.rs",
            r#"//! Admin registrations for this app.
//!
//! Example once a model is enabled in models.rs:
// use djangors_admin::AdminSite;
// use crate::my_app::models::Student;
// site.register::<Student>();
"#
            .to_string(),
        ),
    ];
    for (file, content) in files {
        std::fs::write(dir.join(file), content)
            .map_err(|e| format!("failed to write 'src/{name}/{file}': {e}"))?;
    }
    println!("[dj new-app] created 'src/{name}/'");
    println!("[dj new-app] next: add 'mod {name};' to src/main.rs, and mount its routes");
    Ok(())
}

/// Start the dev server.
pub fn run(port: u16) -> Result<(), String> {
    if !Path::new("Cargo.toml").is_file() || !Path::new("djangors.toml").is_file() {
        return Err("not a Djangors project (expected Cargo.toml and djangors.toml)".into());
    }
    let manifest = std::fs::read_to_string("Cargo.toml").map_err(|e| e.to_string())?;
    let value: toml::Value =
        toml::from_str(&manifest).map_err(|e| format!("invalid Cargo.toml: {e}"))?;
    let package = value
        .get("package")
        .and_then(|v| v.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| "Cargo.toml has no [package].name".to_string())?
        .to_string();
    let source = Path::new("src");
    let mut child = None;
    let mut needs_build = true;
    let mut last_mtime = project_mtime(source).max(file_mtime(Path::new("Cargo.toml")));
    // v1 has no signal-handling dependency in the workspace; Ctrl-C may leave the child running.
    loop {
        if child.is_none() && needs_build {
            println!("[dj run] building...");
            let status = std::process::Command::new("cargo")
                .arg("build")
                .status()
                .map_err(|e| format!("failed to run cargo build: {e}"))?;
            needs_build = false;
            if status.success() {
                let binary = Path::new("target/debug").join(&package);
                let spawned = std::process::Command::new(binary)
                    .env("DJANGORS_PORT", port.to_string())
                    .spawn();
                match spawned {
                    Ok(process) => {
                        child = Some(process);
                        println!("[dj run] serving on http://127.0.0.1:{port}");
                    }
                    Err(e) => eprintln!("[dj run] failed to start binary: {e}"),
                }
            } else {
                println!("[dj run] build failed, watching for changes...");
            }
        }
        std::thread::sleep(Duration::from_millis(500));
        let current = project_mtime(source).max(file_mtime(Path::new("Cargo.toml")));
        if current > last_mtime {
            last_mtime = current;
            println!("[dj run] change detected, rebuilding...");
            if let Some(mut process) = child.take() {
                let _ = process.kill();
                let _ = process.wait();
            }
            needs_build = true;
        }
    }
}

fn file_mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn project_mtime(path: &Path) -> SystemTime {
    let mut newest = file_mtime(path);
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            let mtime = if p.is_dir() {
                project_mtime(&p)
            } else if p.extension().is_some_and(|e| e == "rs") {
                file_mtime(&p)
            } else {
                SystemTime::UNIX_EPOCH
            };
            if mtime > newest {
                newest = mtime;
            }
        }
    }
    newest
}

/// Check externally observable project settings and structure. Model/admin checks are
/// intentionally unavailable because those registries only exist in the target binary.
pub fn check() -> Result<Vec<String>, String> {
    let mut issues = Vec::new();
    match djangors_core::DjangorsSettings::load() {
        Ok((settings, warnings)) => {
            issues.extend(
                warnings
                    .into_iter()
                    .map(|w| format!("settings warning: {w}")),
            );
            if let Err(e) = settings.validate() {
                issues.push(e.to_string());
            }
        }
        Err(e) => issues.push(e.to_string()),
    }
    let manifest_path = Path::new("Cargo.toml");
    if !manifest_path.is_file() {
        issues.push("Cargo.toml is missing".into());
    } else {
        let content = std::fs::read_to_string(manifest_path).map_err(|e| e.to_string())?;
        let value: toml::Value =
            toml::from_str(&content).map_err(|e| format!("invalid Cargo.toml: {e}"))?;
        if value
            .get("bin")
            .and_then(toml::Value::as_array)
            .is_none_or(|bins| bins.is_empty())
        {
            issues.push("Cargo.toml has no [[bin]] table".into());
        }
    }
    if !Path::new("src/main.rs").is_file() {
        issues.push("src/main.rs is missing".into());
    }
    Ok(issues)
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
    static FS_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    /// Slow because it builds a generated standalone project; run with `--ignored`.
    #[test]
    #[ignore]
    fn test_new_app_generates_module() {
        let _guard = FS_MUTEX.lock().unwrap();
        let root = std::env::temp_dir().join(format!("dj_test_app_{}", std::process::id()));
        let project = root.join("project");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        new(project.to_str().unwrap()).unwrap();
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(&project).unwrap();
        new_app("accounts").unwrap();
        assert!(project.join("src/accounts/mod.rs").exists());
        assert!(project.join("src/accounts/models.rs").exists());
        assert!(project.join("src/accounts/views.rs").exists());
        assert!(project.join("src/accounts/admin.rs").exists());
        std::fs::write(
            project.join("src/main.rs"),
            format!(
                "{}\nmod accounts;\n",
                std::fs::read_to_string(project.join("src/main.rs")).unwrap()
            ),
        )
        .unwrap();
        let status = std::process::Command::new("cargo")
            .args(["build", "--offline"])
            .current_dir(&project)
            .status()
            .unwrap();
        std::env::set_current_dir(old).unwrap();
        let _ = std::fs::remove_dir_all(root);
        assert!(status.success());
    }

    #[test]
    fn test_check_generated_project_and_invalid_settings() {
        let _guard = FS_MUTEX.lock().unwrap();
        let root = std::env::temp_dir().join(format!("dj_test_check_{}", std::process::id()));
        let project = root.join("project");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        new(project.to_str().unwrap()).unwrap();
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(&project).unwrap();
        assert!(check().unwrap().is_empty());
        std::fs::write("djangors.toml", "debug = false\n").unwrap();
        let issues = check().unwrap();
        std::env::set_current_dir(old).unwrap();
        let _ = std::fs::remove_dir_all(root);
        assert!(issues
            .iter()
            .any(|issue| issue.contains("SECRET_KEY cannot be empty")));
    }

    /// End-to-end proof that a generated project compiles and serves the welcome page over HTTP.
    /// Run explicitly via `cargo test -p djangors-cli -- --ignored`.
    #[test]
    #[ignore]
    fn test_new_generated_project_builds_and_serves() {
        let _guard = FS_MUTEX.lock().unwrap();
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

    /// Slow end-to-end proof of the hand-rolled build/watch/restart loop.
    /// Run with `cargo test -p djangors-cli -- --ignored`.
    #[test]
    #[ignore]
    fn test_run_rebuilds_after_source_change() {
        let _guard = FS_MUTEX.lock().unwrap();
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::time::{Duration, Instant};
        let root = std::env::temp_dir().join(format!("dj_test_run_{}", std::process::id()));
        let project = root.join("project");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        new(project.to_str().unwrap()).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        let mut runner = std::process::Command::new("cargo")
            .args(["run", "--manifest-path"])
            .arg(&manifest)
            .args([
                "-p",
                "djangors-cli",
                "--",
                "run",
                "--port",
                &port.to_string(),
            ])
            .current_dir(&project)
            .spawn()
            .unwrap();
        let request = |expected: &str| -> bool {
            if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
                let _ = stream.write_all(
                    "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n".as_bytes(),
                );
                let mut body = String::new();
                let _ = stream.read_to_string(&mut body);
                body.contains(expected)
            } else {
                false
            }
        };
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(90) && !request("It worked!") {
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(request("It worked!"), "watch server did not start");
        let views = project.join("src/views.rs");
        let content = std::fs::read_to_string(&views)
            .unwrap()
            .replace("It worked!", "Changed by watcher");
        std::fs::write(views, content).unwrap();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(90) && !request("Changed by watcher") {
            std::thread::sleep(Duration::from_millis(250));
        }
        let changed = request("Changed by watcher");
        let _ = runner.kill();
        let _ = runner.wait();
        // `runner` is the `cargo run` process; `dj run`'s own watch loop spawns the
        // generated server binary as ITS child. A hard kill of `runner` does not
        // cascade to that grandchild (SIGKILL never propagates to descendants), so
        // it survives as an orphan holding the port (and, inheriting this test
        // process's stdio, would otherwise hang anything piping this test's output
        // waiting for EOF that never comes). Find and kill it directly by matching
        // its cmdline against this test's own unique binary path.
        kill_processes_matching(&project.join("target/debug"));
        let _ = std::fs::remove_dir_all(root);
        assert!(changed, "watch server did not serve rebuilt content");
    }

    /// Best-effort: SIGKILL any running process whose *resolved executable path*
    /// is under `dir` (Linux-only, via /proc — acceptable since this whole test
    /// suite already assumes a Linux/Postgres environment). Must check
    /// `/proc/<pid>/exe` (which always resolves to the real absolute path, even
    /// for a since-deleted binary), not `cmdline` — `dj run` spawns the built
    /// binary via a *relative* path (`target/debug/<pkg>`, since it runs with the
    /// project directory as its cwd), so `cmdline` alone would never match an
    /// absolute `dir`.
    fn kill_processes_matching(dir: &Path) {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return;
        };
        for entry in entries.flatten() {
            let pid_str = entry.file_name();
            let Some(pid_str) = pid_str.to_str() else {
                continue;
            };
            if pid_str.parse::<u32>().is_err() {
                continue;
            }
            let exe_path = entry.path().join("exe");
            let Ok(resolved) = std::fs::read_link(&exe_path) else {
                continue;
            };
            if resolved.starts_with(dir) {
                let _ = std::process::Command::new("kill")
                    .args(["-9", pid_str])
                    .status();
            }
        }
    }
}
