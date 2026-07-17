use djangors_auth::{Auth, User};
use djangors_core::extract::FromRequest;
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_orm::meta::{Model, ModelMeta};
use std::marker::PhantomData;
use std::sync::Mutex;

/// A single registered model in the admin site.
pub trait ModelAdmin: Send + Sync {
    fn model_meta(&self) -> &'static ModelMeta;
}

/// Blanket impl so any real Model can be registered with zero boilerplate.
pub struct DefaultModelAdmin<M: Model>(PhantomData<M>);

impl<M: Model + Send + Sync> ModelAdmin for DefaultModelAdmin<M> {
    fn model_meta(&self) -> &'static ModelMeta {
        M::meta()
    }
}

pub struct AdminSite {
    registry: Mutex<Vec<Box<dyn ModelAdmin>>>,
}

impl Default for AdminSite {
    fn default() -> Self {
        Self::new()
    }
}

impl AdminSite {
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(Vec::new()),
        }
    }

    /// Register a model with the default (no customization) ModelAdmin.
    pub fn register<M: Model + Send + Sync + 'static>(&self) {
        let mut reg = self.registry.lock().unwrap();
        reg.push(Box::new(DefaultModelAdmin::<M>(PhantomData)));
    }

    /// Build a Router with GET "/" route.
    ///
    /// The registry snapshot is captured directly in the handler closure,
    /// not via `Router::with_state` — `Router::mount` (the real, documented
    /// way a caller composes `site.urls()` into a larger app router) only
    /// copies routes, never a sub-router's own state, so any state attached
    /// here would silently never reach `admin_index` once mounted. Capturing
    /// it in the closure sidesteps that entirely: it's baked into the
    /// handler itself, independent of whatever router it ends up mounted
    /// under. `Auth::<User>::from_request` below still relies on
    /// `Request::state::<Database>()`, which *is* expected to come from the
    /// caller's own top-level `.with_state(db)` call (the same state that
    /// already correctly reaches every route, mounted or not, since
    /// `Router::dispatch` always attaches whichever router's `dispatch` is
    /// actually running its own `self.state` — see `Router::dispatch`).
    pub fn urls(&self) -> Router {
        let reg = self.registry.lock().unwrap();
        let snapshot: Vec<&'static ModelMeta> = reg.iter().map(|item| item.model_meta()).collect();
        Router::new().get("/", move |req: Request, params: PathParams| {
            admin_index(req, params, snapshot.clone())
        })
    }
}

async fn admin_index(
    req: Request,
    _params: PathParams,
    registry: Vec<&'static ModelMeta>,
) -> Result<Response, DjangorsError> {
    let auth = Auth::<User>::from_request(&req).await?;
    if !auth.0.is_staff {
        return Err(DjangorsError::Forbidden(
            "staff status required".to_string(),
        ));
    }

    let mut body = String::new();
    for meta in &registry {
        body.push_str(&format!("<li>{}.{}</li>", meta.app_label, meta.struct_name));
    }

    Ok(Response::html(StatusCode::OK, format!("<ul>{}</ul>", body)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use djangors_auth::SESSION_USER_ID_KEY;
    use djangors_macros::Model as MacroModel;
    use hyper::http::{Extensions, HeaderMap, Method, Uri};

    static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_a")]
    #[allow(dead_code)]
    struct ModelA {
        #[djangors(primary_key, auto)]
        id: i64,
        name: String,
    }

    #[derive(MacroModel, Debug, Clone)]
    #[djangors(app = "admin_test", table_name = "test_model_b")]
    #[allow(dead_code)]
    struct ModelB {
        #[djangors(primary_key, auto)]
        id: i64,
        title: String,
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_index_endpoints() {
        let _guard = DB_MUTEX.lock().unwrap();
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        // Drop tables
        let _ = sqlx::query("DROP TABLE IF EXISTS auth_user")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS test_model_b")
            .execute(db.pool())
            .await;

        // Create auth_user
        sqlx::query(
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

        let now = chrono::Utc::now();

        // 1. Create a non-staff user
        let non_staff = User {
            id: 0,
            username: "non_staff".to_string(),
            email: "non_staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        // 2. Create a staff user
        let staff = User {
            id: 0,
            username: "staff".to_string(),
            email: "staff@example.com".to_string(),
            password: "hash".to_string(),
            is_active: true,
            is_staff: true,
            is_superuser: false,
            date_joined: now,
            last_login: Some(now),
        }
        .save(&db)
        .await
        .unwrap();

        // Setup AdminSite
        let site = AdminSite::new();
        site.register::<ModelA>();
        site.register::<ModelB>();

        // Mount into a parent router, exactly like real usage -
        // `Router::mount` never merges a sub-router's own state into the
        // parent (see `AdminSite::urls`'s own doc comment), so testing
        // directly against `site.urls()` alone would not catch a
        // regression where `admin_index` went back to depending on
        // sub-router state instead of its closure-captured registry. In
        // real production, `Router::dispatch` (used by `Djangors::run`)
        // automatically attaches the top-level router's own `.with_state`
        // to every request; this test calls `.handle()` directly instead,
        // so `Database` state is attached per-request manually below.
        let router = Router::new().mount("/admin", site.urls());

        // Test 1: GET /admin/ with no auth -> 401 Unauthorized
        let req_no_auth = Request::new(
            Method::GET,
            Uri::from_static("/admin/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_no_auth = router.handle(req_no_auth).await;
        assert!(res_no_auth.is_err());
        assert_eq!(
            res_no_auth.unwrap_err().status_code(),
            StatusCode::UNAUTHORIZED
        );

        // Test 2: GET /admin/ authenticated as non-staff -> 403 Forbidden
        let session_non_staff = djangors_sessions::Session::new_empty();
        session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);
        let mut extensions_non_staff = Extensions::new();
        extensions_non_staff.insert(session_non_staff);
        let req_non_staff = Request::new(
            Method::GET,
            Uri::from_static("/admin/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(extensions_non_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_non_staff = router.handle(req_non_staff).await;
        assert!(res_non_staff.is_err());
        assert_eq!(
            res_non_staff.unwrap_err().status_code(),
            StatusCode::FORBIDDEN
        );

        // Test 3: GET /admin/ authenticated as staff -> 200 OK with correct HTML
        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);
        let mut extensions_staff = Extensions::new();
        extensions_staff.insert(session_staff);
        let req_staff = Request::new(
            Method::GET,
            Uri::from_static("/admin/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(extensions_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_staff = router.handle(req_staff).await.unwrap();
        assert_eq!(res_staff.status(), StatusCode::OK);
        let body = String::from_utf8(res_staff.body().to_vec()).unwrap();
        assert!(body.contains("<li>admin_test.ModelA</li>"));
        assert!(body.contains("<li>admin_test.ModelB</li>"));

        // Clean up
        let _ = sqlx::query("DROP TABLE auth_user").execute(db.pool()).await;
    }
}
