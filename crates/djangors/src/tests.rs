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
