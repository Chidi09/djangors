//! Stub implementations for all dj subcommands.

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

/// Create an admin user.
pub fn createsuperuser() {
    println!("[dj createsuperuser] would create an admin user (not yet implemented)");
}

/// Open a REPL.
pub fn shell() {
    println!("[dj shell] would open a REPL (not yet implemented)");
}

/// Run the test suite.
pub fn test() {
    println!("[dj test] would run the test suite (not yet implemented)");
}
