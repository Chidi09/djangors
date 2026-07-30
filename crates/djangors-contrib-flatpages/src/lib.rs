#![deny(missing_docs)]
//! Admin-editable flat pages.
//!
//! Flatpage content is served as trusted HTML, matching Django's convention that
//! staff-authored page bodies are rendered as-is. Only trusted administrators
//! should be allowed to edit this model. Djangors v1 has no catch-all route, so
//! callers must register each known flatpage URL explicitly with [`flatpage_routes`].

use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_macros::Model;
use djangors_orm::{Model, UnresolvedCompare, UnresolvedExpr, Value};

/// An admin-editable flat HTML page model.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_contrib_flatpages", table_name = "djangors_flatpage")]
pub struct FlatPage {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// The unique URL path for the flat page (e.g. "/about/").
    #[djangors(max_length = 255, unique)]
    pub url: String,
    /// The human-readable title of the flat page.
    #[djangors(max_length = 255)]
    pub title: String,
    /// The raw HTML content of the flat page.
    pub content: String,
}

/// Looks up and serves the exact request path from the database.
pub async fn flatpage_handler(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
    let page = FlatPage::objects()
        .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
            field: "url",
            value: Value::Text(req.path().to_owned()),
        }]))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .ok_or(DjangorsError::NotFound)?;
    Ok(Response::html(StatusCode::OK, page.content))
}

/// Registers explicit known paths. This is not a catch-all fallback.
pub fn flatpage_routes<I, S>(mut router: Router, paths: I) -> Router
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for path in paths {
        router = router.get(path.as_ref(), flatpage_handler);
    }
    router
}

/// Registers this model with the existing generic admin site.
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register::<FlatPage>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use djangors_core::{AppState, Request};
    use hyper::http::{HeaderMap, Method, Uri};

    #[test]
    fn model_metadata_uses_flatpage_table() {
        assert_eq!(FlatPage::meta().table_name, "djangors_flatpage");
    }

    #[tokio::test]
    async fn missing_database_is_an_error() {
        let req = Request::new(
            Method::GET,
            Uri::from_static("/about/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(AppState::new());
        assert!(matches!(
            flatpage_handler(req, PathParams::new()).await,
            Err(DjangorsError::Internal(_))
        ));
    }

    #[tokio::test]
    async fn serves_matching_page_and_not_found_for_other_path() {
        let Ok(db) = djangors_test::TestDatabase::connect().await else {
            return;
        };
        db.create_table("CREATE TABLE IF NOT EXISTS djangors_flatpage (id BIGSERIAL PRIMARY KEY, url VARCHAR(255) UNIQUE NOT NULL, title VARCHAR(255) NOT NULL, content TEXT NOT NULL)").await.unwrap();
        sqlx::query("DELETE FROM djangors_flatpage")
            .execute(db.database().pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO djangors_flatpage (url, title, content) VALUES ($1, $2, $3)")
            .bind("/about/")
            .bind("About")
            .bind("<h1>About &amp; Us</h1>")
            .execute(db.database().pool())
            .await
            .unwrap();
        let req = |path: &str| {
            Request::new(
                Method::GET,
                path.parse().unwrap(),
                HeaderMap::new(),
                Bytes::new(),
            )
            .with_state(AppState::new().insert(db.database().clone()))
        };
        let served = flatpage_handler(req("/about/"), PathParams::new())
            .await
            .unwrap();
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(served.body(), b"<h1>About &amp; Us</h1>");
        assert!(matches!(
            flatpage_handler(req("/missing/"), PathParams::new()).await,
            Err(DjangorsError::NotFound)
        ));
        db.drop_table("djangors_flatpage").await.unwrap();
    }
}
