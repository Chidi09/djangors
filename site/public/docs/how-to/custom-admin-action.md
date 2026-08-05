# How to Add a Custom Admin Action

## Problem
You want to perform a custom bulk action on selected rows in the Djangors admin changelist (e.g. marking selected articles as published or resetting user accounts).

## Solution
Define an `AdminAction` struct specifying the action name, user-facing label, confirmation requirement, and an async handler function. Register it on your `ModelAdminConfig` passed to `AdminSite::register_with`.

## Code Example

```rust,compile
# use djangors_orm::Model;
# #[derive(djangors_macros::Model, Debug, Clone, serde::Serialize, serde::Deserialize)]
# #[djangors(app = "library", table_name = "library_article")]
# struct Article { #[djangors(primary_key, auto)] id: i64 }
use djangors_admin::{AdminAction, AdminSite, ModelAdminConfig};
use djangors_db::Database;
use djangors_core::DjangorsError;

// 1. Define the async action handler function
fn publish_articles_handler<'a>(
    db: &'a Database,
    pks: &'a [i64],
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DjangorsError>> + Send + 'a>> {
    Box::pin(async move {
        for &pk in pks {
            djangors_orm::sqlx::query("UPDATE library_article SET is_published = TRUE WHERE id = $1")
                .bind(pk)
                .execute(db.pool())
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        }
        Ok(())
    })
}

// 2. Define the AdminAction
pub const PUBLISH_ACTION: AdminAction = AdminAction {
    name: "publish_selected",
    label: "Publish selected articles",
    requires_confirm: true,
    handler: publish_articles_handler,
};

// 3. Register with AdminSite
fn register_admin(site: &mut AdminSite) {
    site.register_with::<Article>(ModelAdminConfig {
        list_display: Some(&["id", "title", "is_published"]),
        actions: Some(&[PUBLISH_ACTION]),
        ..Default::default()
    });
}
```
