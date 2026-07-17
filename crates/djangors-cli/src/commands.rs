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
pub fn migrate() {
    println!("[dj migrate] would apply database migrations (not yet implemented)");
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
