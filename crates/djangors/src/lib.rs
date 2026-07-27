#![deny(missing_docs)]
//! The Django of Rust: batteries-included web framework — ORM, migrations, admin, forms, auth, background tasks.

pub use djangors_tasks as tasks;

/// Django-style administration site and model administration.
pub use djangors_admin as admin;
/// Authentication backends and user authentication events.
pub use djangors_auth as auth;
/// In-memory, database, and Redis cache backends.
pub use djangors_cache as cache;
/// Core HTTP types: Router, Request, Response, middleware, and settings.
pub use djangors_core as core;
/// Database configuration and connection abstractions.
pub use djangors_db as db;
/// Form fields, validation, and form errors.
pub use djangors_forms as forms;
/// Internationalization catalogs, locales, and middleware.
pub use djangors_i18n as i18n;
/// Mail messages and delivery backends.
pub use djangors_mail as mail;
/// Database migration operations and planning.
pub use djangors_migrations as migrations;
/// ORM models, querysets, expressions, and metadata.
pub use djangors_orm as orm;
/// REST serialization, authentication, and permissions.
pub use djangors_rest as rest;
/// Session storage and signed cookie sessions.
pub use djangors_sessions as sessions;
/// Static-file collection, storage, and serving.
pub use djangors_staticfiles as staticfiles;
/// Template rendering engine and filters.
pub use djangors_template as template;

#[cfg(test)]
mod tests {
    #[test]
    fn framework_modules_expose_public_items() {
        let _ = std::mem::size_of::<crate::core::Router>();
        let _ = std::mem::size_of::<crate::orm::OrmError>();
        let _ = std::mem::size_of::<crate::migrations::Operation>();
        let _ = std::mem::size_of::<crate::rest::AllowAny>();
        let _ = std::mem::size_of::<crate::admin::ModelAdminConfig>();
        let _ = std::mem::size_of::<crate::auth::ModelBackend>();
        let _ = std::mem::size_of::<crate::forms::CharField>();
        let _ = std::mem::size_of::<crate::sessions::Session>();
        let _ = std::mem::size_of::<crate::template::TemplateEngine>();
        let _ = std::mem::size_of::<crate::staticfiles::StaticFiles>();
        let _ = std::mem::size_of::<crate::cache::InMemoryCache>();
        let _ = std::mem::size_of::<crate::mail::ConsoleBackend>();
        let _ = std::mem::size_of::<crate::i18n::Catalog>();
        let _ = std::mem::size_of::<crate::db::DatabaseConfig>();
    }
}
