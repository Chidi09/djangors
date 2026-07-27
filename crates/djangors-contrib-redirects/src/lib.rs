#![deny(missing_docs)]
//! Database-backed redirects for Djangors.
//!
//! This crate exposes a plain lookup helper rather than a generic `tower::Layer`:
//! Djangors routing is composed from handlers, and explicit route mounting keeps
//! database lookup and fallthrough behavior easy to test and wire into an app.

use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_macros::Model;
use djangors_orm::{Model, UnresolvedCompare, UnresolvedExpr, Value};

/// A database-backed HTTP redirect model.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_contrib_redirects", table_name = "djangors_redirect")]
pub struct Redirect {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// The incoming URL path to match and redirect away from (e.g. "/old-path/").
    #[djangors(max_length = 255, unique)]
    pub old_path: String,
    /// The destination URL path to redirect to (e.g. "/new-path/").
    #[djangors(max_length = 255)]
    pub new_path: String,
}

/// Returns a redirect response when a row matches, or `None` for clean fallthrough.
pub async fn lookup_redirect(
    req: &Request,
    status: StatusCode,
) -> Result<Option<Response>, DjangorsError> {
    let db = req
        .state::<djangors_orm::djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".into()))?;
    let redirect = Redirect::objects()
        .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
            field: "old_path",
            value: Value::Text(req.path().to_owned()),
        }]))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    Ok(redirect.map(|row| Response::text(status, "").header("Location", &row.new_path)))
}

/// Handler form for an explicitly registered old path.
pub async fn redirect_handler(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    lookup_redirect(&req, StatusCode::PERMANENT_REDIRECT)
        .await?
        .ok_or(DjangorsError::NotFound)
}

/// Registers explicit old paths. Use [`lookup_redirect`] in an outer application
/// service when true pre-routing fallthrough is required.
pub fn redirect_routes<I, S>(mut router: Router, paths: I) -> Router
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for path in paths {
        router = router.get(path.as_ref(), redirect_handler);
    }
    router
}

/// Registers the `Redirect` model with an admin site instance.
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register::<Redirect>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use djangors_core::{AppState, Request};
    use hyper::http::{HeaderMap, Method, Uri};

    #[test]
    fn model_metadata_uses_redirect_table() {
        assert_eq!(Redirect::meta().table_name, "djangors_redirect");
    }

    #[tokio::test]
    async fn missing_database_is_an_error() {
        let req = Request::new(
            Method::GET,
            Uri::from_static("/old"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(AppState::new());
        assert!(matches!(
            lookup_redirect(&req, StatusCode::MOVED_PERMANENTLY).await,
            Err(DjangorsError::Internal(_))
        ));
    }

    #[tokio::test]
    async fn returns_redirect_and_falls_through_for_other_path() {
        let Ok(db) = djangors_test::TestDatabase::connect().await else {
            return;
        };
        db.create_table("CREATE TABLE IF NOT EXISTS djangors_redirect (id BIGSERIAL PRIMARY KEY, old_path VARCHAR(255) UNIQUE NOT NULL, new_path VARCHAR(255) NOT NULL)").await.unwrap();
        sqlx::query("DELETE FROM djangors_redirect")
            .execute(db.database().pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO djangors_redirect (old_path, new_path) VALUES ($1, $2)")
            .bind("/old/")
            .bind("/new/")
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
        let response = lookup_redirect(&req("/old/"), StatusCode::MOVED_PERMANENTLY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(response.headers().get("location").unwrap(), "/new/");
        assert!(
            lookup_redirect(&req("/other/"), StatusCode::MOVED_PERMANENTLY)
                .await
                .unwrap()
                .is_none()
        );
        db.drop_table("djangors_redirect").await.unwrap();
    }
}
