use async_trait::async_trait;
use djangors_auth::{Auth, User};
use djangors_core::extract::FromRequest;
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_orm::meta::{Model, ModelMeta};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

pub(crate) const CHANGELIST_PER_PAGE: i64 = 100;

pub struct ChangelistPage {
    pub columns: Vec<&'static str>, // field names, declaration order
    pub rows: Vec<Vec<String>>,     // Display-rendered, NOT escaped (view escapes)
    pub total: i64,                 // COUNT(*) over the whole table
    pub page: i64,                  // 1-based current page
    pub per_page: i64,
}

#[async_trait]
pub trait ModelAdmin: Send + Sync {
    fn model_meta(&self) -> &'static ModelMeta;
    async fn changelist(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>, // raw ?o= value, e.g. "name" or "-name"
        page: i64,           // already-validated >= 1
        per_page: i64,
    ) -> Result<ChangelistPage, DjangorsError>;
}

/// Blanket impl so any real Model can be registered with zero boilerplate.
pub struct DefaultModelAdmin<M: Model>(PhantomData<M>);

#[async_trait]
impl<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static> ModelAdmin
    for DefaultModelAdmin<M>
{
    fn model_meta(&self) -> &'static ModelMeta {
        M::meta()
    }

    async fn changelist(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>,
        page: i64,
        per_page: i64,
    ) -> Result<ChangelistPage, DjangorsError> {
        let mut qs = M::objects();
        if let Some(o) = order {
            qs = qs.order_by(o).map_err(|e| match e {
                djangors_orm::error::OrmError::FieldNotFound { .. } => {
                    DjangorsError::BadRequest(e.to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;
        }
        let total = M::objects()
            .count(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let offset = (page - 1) * per_page;
        qs = qs.limit(per_page).offset(offset);
        let items = qs
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let mut rows = Vec::new();
        for item in &items {
            let row_vals: Vec<String> = item
                .field_values()
                .into_iter()
                .map(|(_, v)| v.to_string())
                .collect();
            rows.push(row_vals);
        }

        let columns = M::field_names();

        Ok(ChangelistPage {
            columns,
            rows,
            total,
            page,
            per_page,
        })
    }
}

pub struct AdminSite {
    registry: Mutex<Vec<Arc<dyn ModelAdmin>>>,
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
    pub fn register<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static>(&self) {
        let mut reg = self.registry.lock().unwrap();
        reg.push(Arc::new(DefaultModelAdmin::<M>(PhantomData)));
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
        let admins: Vec<Arc<dyn ModelAdmin>> = reg.iter().cloned().collect();
        let snapshot: Vec<&'static ModelMeta> =
            admins.iter().map(|item| item.model_meta()).collect();

        let index_admins = snapshot.clone();
        let changelist_admins = admins.clone();

        Router::new()
            .get("/", move |req: Request, params: PathParams| {
                admin_index(req, params, index_admins.clone())
            })
            .get(
                "/{app:slug}/{model:slug}/",
                move |req: Request, params: PathParams| {
                    admin_changelist(req, params, changelist_admins.clone())
                },
            )
    }
}

async fn require_staff(req: &Request) -> Result<(), DjangorsError> {
    let auth = Auth::<User>::from_request(req).await?;
    if !auth.0.is_staff {
        return Err(DjangorsError::Forbidden(
            "staff status required".to_string(),
        ));
    }
    Ok(())
}

async fn admin_index(
    req: Request,
    _params: PathParams,
    registry: Vec<&'static ModelMeta>,
) -> Result<Response, DjangorsError> {
    require_staff(&req).await?;

    let mut body = String::new();
    for meta in &registry {
        body.push_str(&format!(
            "<li><a href=\"{}/{}/\">{}.{}</a></li>",
            meta.app_label,
            meta.struct_name.to_lowercase(),
            meta.app_label,
            meta.struct_name
        ));
    }

    Ok(Response::html(StatusCode::OK, format!("<ul>{}</ul>", body)))
}

async fn admin_changelist(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
) -> Result<Response, DjangorsError> {
    require_staff(&req).await?;

    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

    let o = req.query("o");
    let page = match req.query("page") {
        Some(p_str) => {
            let p = p_str
                .parse::<i64>()
                .map_err(|_| DjangorsError::BadRequest("invalid page parameter".to_string()))?;
            if p < 1 {
                return Err(DjangorsError::BadRequest("page must be >= 1".to_string()));
            }
            p
        }
        None => 1,
    };

    let page_data = admin.changelist(db, o, page, CHANGELIST_PER_PAGE).await?;

    let mut header_html = String::new();
    for col in &page_data.columns {
        let link = if o == Some(*col) {
            format!("?o=-{}", col)
        } else {
            format!("?o={}", col)
        };
        header_html.push_str(&format!("<th><a href=\"{}\">{}</a></th>", link, col));
    }

    let mut body_html = String::new();
    for row in page_data.rows {
        body_html.push_str("<tr>");
        for cell in row {
            body_html.push_str(&format!("<td>{}</td>", djangors_core::html_escape(&cell)));
        }
        body_html.push_str("</tr>");
    }

    let total_pages = if page_data.total == 0 {
        1
    } else {
        (page_data.total + CHANGELIST_PER_PAGE - 1) / CHANGELIST_PER_PAGE
    };
    let mut pager_html = String::new();
    if page > 1 {
        let mut prev_link = format!("?page={}", page - 1);
        if let Some(order_val) = o {
            prev_link.push_str(&format!("&o={}", order_val));
        }
        pager_html.push_str(&format!("<a href=\"{}\">Previous</a> ", prev_link));
    }
    pager_html.push_str(&format!(
        "Page {} of {}. Total: {}. ",
        page, total_pages, page_data.total
    ));
    if page * CHANGELIST_PER_PAGE < page_data.total {
        let mut next_link = format!("?page={}", page + 1);
        if let Some(order_val) = o {
            next_link.push_str(&format!("&o={}", order_val));
        }
        pager_html.push_str(&format!("<a href=\"{}\">Next</a>", next_link));
    }

    let html = format!(
        "<table><thead><tr>{}</tr></thead><tbody>{}</tbody></table><div>{}</div>",
        header_html, body_html, pager_html
    );

    Ok(Response::html(StatusCode::OK, html))
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
    #[djangors(app = "admin_test", table_name = "test_model_b", ordering = "-title")]
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
        assert!(body.contains("<li><a href=\"admin_test/modela/\">admin_test.ModelA</a></li>"));
        assert!(body.contains("<li><a href=\"admin_test/modelb/\">admin_test.ModelB</a></li>"));

        // Clean up
        let _ = sqlx::query("DROP TABLE auth_user").execute(db.pool()).await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_admin_changelist_endpoints() {
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

        // Create test_model_a
        sqlx::query(
            "CREATE TABLE test_model_a (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Create test_model_b (has ordering = "-title" meta)
        sqlx::query(
            "CREATE TABLE test_model_b (
                id BIGSERIAL PRIMARY KEY,
                title TEXT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let now = chrono::Utc::now();

        // Create users
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

        // Seed ModelA rows
        let _row1 = ModelA {
            id: 0,
            name: "Normal Row".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let _row2 = ModelA {
            id: 0,
            name: "<script>alert(1)</script>".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let site = AdminSite::new();
        site.register::<ModelA>();
        site.register::<ModelB>();

        let router = Router::new().mount("/admin", site.urls());

        // Test 1: GET /admin/admin_test/modela/ as staff -> 200, checks headers & XSS escape
        let session_staff = djangors_sessions::Session::new_empty();
        session_staff.set(SESSION_USER_ID_KEY, staff.id);
        let mut extensions_staff = Extensions::new();
        extensions_staff.insert(session_staff.clone());
        let req_list = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(extensions_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));

        let res_list = router.handle(req_list).await.unwrap();
        assert_eq!(res_list.status(), StatusCode::OK);
        let body = String::from_utf8(res_list.body().to_vec()).unwrap();

        // Assert header links
        assert!(body.contains("<a href=\"?o=id\">id</a>"));
        assert!(body.contains("<a href=\"?o=name\">name</a>"));

        // Assert row content and XSS escaping
        assert!(body.contains("Normal Row"));
        assert!(body.contains("&lt;script&gt;alert(1)&lt;&#x2F;script&gt;"));
        assert!(!body.contains("<script>"));

        // Test 2: Ordering tests
        // Seed rows for sorting
        let _ = sqlx::query("TRUNCATE test_model_a RESTART IDENTITY")
            .execute(db.pool())
            .await;

        let _row_a = ModelA {
            id: 0,
            name: "Row A".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _row_b = ModelA {
            id: 0,
            name: "Row B".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        // GET with ascending order ?o=name
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_asc = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/?o=name"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_asc = router.handle(req_asc).await.unwrap();
        let body_asc = String::from_utf8(res_asc.body().to_vec()).unwrap();
        let idx_a = body_asc.find("Row A").unwrap();
        let idx_b = body_asc.find("Row B").unwrap();
        assert!(idx_a < idx_b);

        // GET with descending order ?o=-name
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_desc = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/?o=-name"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_desc = router.handle(req_desc).await.unwrap();
        let body_desc = String::from_utf8(res_desc.body().to_vec()).unwrap();
        let idx_a_desc = body_desc.find("Row A").unwrap();
        let idx_b_desc = body_desc.find("Row B").unwrap();
        assert!(idx_b_desc < idx_a_desc);

        // GET with invalid field ?o=nonsense -> 400
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_invalid = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/?o=nonsense"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_invalid = router.handle(req_invalid).await;
        assert!(res_invalid.is_err());
        assert_eq!(
            res_invalid.unwrap_err().status_code(),
            StatusCode::BAD_REQUEST
        );

        // Test 3: Pagination tests
        // Seed 3 more rows to have 5 rows in total
        let _ = ModelA {
            id: 0,
            name: "Row C".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _ = ModelA {
            id: 0,
            name: "Row D".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _ = ModelA {
            id: 0,
            name: "Row E".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        // Direct trait call: admin.changelist(&db, None, 2, 2)
        let admin_a = DefaultModelAdmin::<ModelA>(PhantomData);
        let page_data = admin_a.changelist(&db, None, 2, 2).await.unwrap();
        assert_eq!(page_data.total, 5);
        assert_eq!(page_data.rows.len(), 2); // Row 3 and Row 4
        assert_eq!(page_data.rows[0][1], "Row C");
        assert_eq!(page_data.rows[1][1], "Row D");

        // HTTP assert that ?page=2 with the real constant (100) and few rows returns 200 with empty table body and Previous link
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_page2 = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/?page=2"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_page2 = router.handle(req_page2).await.unwrap();
        assert_eq!(res_page2.status(), StatusCode::OK);
        let body_page2 = String::from_utf8(res_page2.body().to_vec()).unwrap();
        assert!(body_page2.contains("<tbody></tbody>"));
        assert!(body_page2.contains("Previous"));

        // Test 4: Auth permissions
        // Unauthenticated -> 401
        let req_unauth = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_unauth = router.handle(req_unauth).await;
        assert!(res_unauth.is_err());
        assert_eq!(
            res_unauth.unwrap_err().status_code(),
            StatusCode::UNAUTHORIZED
        );

        // Non-staff -> 403
        let session_non_staff = djangors_sessions::Session::new_empty();
        session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);
        let mut ext_non_staff = Extensions::new();
        ext_non_staff.insert(session_non_staff);
        let req_non_staff = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modela/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_non_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_non_staff = router.handle(req_non_staff).await;
        assert!(res_non_staff.is_err());
        assert_eq!(
            res_non_staff.unwrap_err().status_code(),
            StatusCode::FORBIDDEN
        );

        // Unknown route as staff -> 404
        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_unknown = Request::new(
            Method::GET,
            Uri::from_static("/admin/nope/nope/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_unknown = router.handle(req_unknown).await;
        assert!(res_unknown.is_err());
        assert_eq!(
            res_unknown.unwrap_err().status_code(),
            StatusCode::NOT_FOUND
        );

        // Test 5: default order (no ?o=) honors the model's `ordering` meta -
        // ModelB declares ordering = "-title", so with no explicit sort param
        // "Beta" must come before "Alpha" even though "Alpha" was saved first.
        let _ = ModelB {
            id: 0,
            title: "Alpha".to_string(),
        }
        .save(&db)
        .await
        .unwrap();
        let _ = ModelB {
            id: 0,
            title: "Beta".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let mut ext_staff = Extensions::new();
        ext_staff.insert(session_staff.clone());
        let req_default_order = Request::new(
            Method::GET,
            Uri::from_static("/admin/admin_test/modelb/"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_extensions(ext_staff)
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res_default_order = router.handle(req_default_order).await.unwrap();
        assert_eq!(res_default_order.status(), StatusCode::OK);
        let body_default_order = String::from_utf8(res_default_order.body().to_vec()).unwrap();
        let idx_beta = body_default_order.find("Beta").unwrap();
        let idx_alpha = body_default_order.find("Alpha").unwrap();
        assert!(idx_beta < idx_alpha);

        // Clean up
        let _ = sqlx::query("DROP TABLE auth_user").execute(db.pool()).await;
        let _ = sqlx::query("DROP TABLE test_model_a")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE test_model_b")
            .execute(db.pool())
            .await;
    }
}
