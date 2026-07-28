use std::sync::Mutex;

use djangors_orm::djangors_db::Database;
use djangors_orm::ForeignKey;
use tower::Layer;

use crate::models::{Tenant, TenantMembership};
use crate::CurrentTenant;

static DB_MUTEX: Mutex<()> = Mutex::new(());

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/postgres".to_string())
}

async fn connect_db() -> Database {
    let config = djangors_db::config::DatabaseConfig::new(db_url());
    Database::connect(&config).await.unwrap()
}

const CREATE_TENANT_TABLE: &str = "CREATE TABLE IF NOT EXISTS djangors_tenancy_tenant (
    id BIGSERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(100) NOT NULL UNIQUE,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL
)";

const CREATE_MEMBERSHIP_TABLE: &str = "CREATE TABLE IF NOT EXISTS djangors_tenancy_membership (
    id BIGSERIAL PRIMARY KEY,
    \"user\" BIGINT NOT NULL,
    tenant BIGINT NOT NULL,
    role VARCHAR(50) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(\"user\", tenant)
)";

async fn setup_tables(db: &Database) {
    sqlx::query(CREATE_TENANT_TABLE)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(CREATE_MEMBERSHIP_TABLE)
        .execute(db.pool())
        .await
        .unwrap();
    // These tests run against a real, persistent (not per-test-isolated) database and use fixed
    // slugs/ids - truncate on every setup so re-running the suite against the same database
    // (e.g. a shared dev Postgres, not a fresh one per CI run) doesn't hit stale unique-constraint
    // violations from a prior run's leftover rows.
    sqlx::query(
        "TRUNCATE djangors_tenancy_membership, djangors_tenancy_tenant RESTART IDENTITY CASCADE",
    )
    .execute(db.pool())
    .await
    .unwrap();
}

// ── Model round-trip tests ──────────────────────────────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn tenant_round_trip() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let db = connect_db().await;
    setup_tables(&db).await;

    let now = chrono::Utc::now();
    let tenant = Tenant {
        id: 0,
        name: "Test School".to_string(),
        slug: "test-school".to_string(),
        is_active: true,
        created_at: now,
    };
    let saved = tenant.save(&db).await.unwrap();
    assert!(saved.id > 0);
    assert_eq!(saved.name, "Test School");
    assert_eq!(saved.slug, "test-school");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn membership_round_trip_and_unique_constraint() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let db = connect_db().await;
    setup_tables(&db).await;

    let now = chrono::Utc::now();
    let tenant = Tenant {
        id: 0,
        name: "Membership Test".to_string(),
        slug: "membership-test".to_string(),
        is_active: true,
        created_at: now,
    }
    .save(&db)
    .await
    .unwrap();

    let m1 = TenantMembership {
        id: 0,
        user: ForeignKey::new(100),
        tenant: ForeignKey::new(tenant.id),
        role: "admin".to_string(),
        created_at: now,
    };
    let saved1 = m1.save(&db).await.unwrap();
    assert!(saved1.id > 0);
    assert_eq!(saved1.role, "admin");

    let m2 = TenantMembership {
        id: 0,
        user: ForeignKey::new(200),
        tenant: ForeignKey::new(tenant.id),
        role: "student".to_string(),
        created_at: now,
    };
    let saved2 = m2.save(&db).await.unwrap();
    assert!(saved2.id > 0);
    assert_ne!(saved1.id, saved2.id);

    // Duplicate (user, tenant) pair should be rejected
    let dup = TenantMembership {
        id: 0,
        user: ForeignKey::new(100),
        tenant: ForeignKey::new(tenant.id),
        role: "teacher".to_string(),
        created_at: now,
    };
    let result = dup.save(&db).await;
    assert!(
        result.is_err(),
        "unique_together on (user, tenant) should reject duplicate pair"
    );
}

// ── Middleware tests ─────────────────────────────────────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn middleware_sets_current_tenant_for_valid_membership() {
    use bytes::Bytes;
    use http_body_util::Full;
    use std::convert::Infallible;
    use tower::Service;
    use tower::ServiceExt;

    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let db = connect_db().await;
    setup_tables(&db).await;

    let now = chrono::Utc::now();
    let tenant = Tenant {
        id: 0,
        name: "Middleware Test".to_string(),
        slug: "middleware-test".to_string(),
        is_active: true,
        created_at: now,
    }
    .save(&db)
    .await
    .unwrap();

    TenantMembership {
        id: 0,
        user: ForeignKey::new(42),
        tenant: ForeignKey::new(tenant.id),
        role: "member".to_string(),
        created_at: now,
    }
    .save(&db)
    .await
    .unwrap();

    let layer =
        crate::TenantResolutionLayer::new(db, |_: &hyper::Request<Full<Bytes>>| Some(42i64));

    let mut svc = layer.layer(tower::service_fn(
        |req: hyper::Request<Full<Bytes>>| async move {
            let ct = req.extensions().get::<CurrentTenant>().copied();
            let body = match ct {
                Some(c) => format!("tenant={}", c.0),
                None => "no-tenant".to_string(),
            };
            Ok::<_, Infallible>(hyper::Response::new(Full::new(Bytes::from(body))))
        },
    ));

    let req = hyper::Request::builder()
        .header("x-tenant-id", tenant.id.to_string())
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    let body = String::from_utf8(
        http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(body, format!("tenant={}", tenant.id));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn middleware_skips_current_tenant_for_no_membership() {
    use bytes::Bytes;
    use http_body_util::Full;
    use std::convert::Infallible;
    use tower::Service;
    use tower::ServiceExt;

    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let db = connect_db().await;
    setup_tables(&db).await;

    let now = chrono::Utc::now();
    let tenant = Tenant {
        id: 0,
        name: "No Membership Tenant".to_string(),
        slug: "no-membership".to_string(),
        is_active: true,
        created_at: now,
    }
    .save(&db)
    .await
    .unwrap();

    // User 99 has NO membership row for this tenant
    let layer =
        crate::TenantResolutionLayer::new(db, |_: &hyper::Request<Full<Bytes>>| Some(99i64));

    let mut svc = layer.layer(tower::service_fn(
        |req: hyper::Request<Full<Bytes>>| async move {
            let ct = req.extensions().get::<CurrentTenant>().copied();
            assert!(
                ct.is_none(),
                "user without membership should not get CurrentTenant"
            );
            Ok::<_, Infallible>(hyper::Response::new(Full::new(Bytes::from("ok"))))
        },
    ));

    let req = hyper::Request::builder()
        .header("x-tenant-id", tenant.id.to_string())
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn middleware_skips_current_tenant_for_missing_header() {
    use bytes::Bytes;
    use http_body_util::Full;
    use std::convert::Infallible;
    use tower::Service;
    use tower::ServiceExt;

    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let db = connect_db().await;
    setup_tables(&db).await;

    let layer =
        crate::TenantResolutionLayer::new(db, |_: &hyper::Request<Full<Bytes>>| Some(42i64));

    let mut svc = layer.layer(tower::service_fn(
        |req: hyper::Request<Full<Bytes>>| async move {
            let ct = req.extensions().get::<CurrentTenant>().copied();
            assert!(ct.is_none(), "no X-Tenant-Id header means no CurrentTenant");
            Ok::<_, Infallible>(hyper::Response::new(Full::new(Bytes::from("ok"))))
        },
    ));

    let req = hyper::Request::builder()
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    assert_eq!(resp.status(), 200);
}

// ── Scope tests ──────────────────────────────────────────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn tenant_scope_rejects_no_current_tenant() {
    use djangors_core::error::DjangorsError;
    use djangors_core::request::Request;
    use djangors_orm::queryset::QuerySet;

    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let db = connect_db().await;
    setup_tables(&db).await;

    let req = Request::new(
        hyper::Method::GET,
        hyper::Uri::from_static("http://example.com/"),
        hyper::HeaderMap::new(),
        bytes::Bytes::new(),
    );

    let qs = QuerySet::<Tenant>::new();
    let result = crate::tenant_scope(&req, qs, "id");
    match result {
        Err(DjangorsError::Unauthorized(msg)) => {
            assert!(msg.contains("no current tenant"), "msg: {msg}");
        }
        other => panic!("expected Unauthorized, got: {other:?}"),
    }
}

/// End-to-end proof that `tenant_scope()` actually connects to what
/// [`crate::TenantResolutionLayer`] sets - not exercised by any other test here, since every other
/// test checks the middleware and `tenant_scope()` in isolation. `CurrentTenant` is inserted into a
/// real `hyper::http::Extensions` (exactly how `TenantResolutionLayer` does it via
/// `req.extensions_mut().insert(...)`, and exactly what `Router::dispatch` propagates into
/// `Request::with_extensions`), then read back through the real `Request::ext::<CurrentTenant>()`
/// path `tenant_scope()` uses. This is the regression test for a real bug caught in review: an
/// earlier draft of `tenant_scope()` read `req.state::<CurrentTenant>()` (app-wide state, set once
/// via `Router::with_state`) instead of `req.ext::<CurrentTenant>()` (per-request extensions,
/// populated fresh by middleware) - a mismatch that would silently reject every real request even
/// with a valid membership, since the two are backed by entirely separate storage.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn tenant_scope_reads_current_tenant_set_via_extensions_like_real_middleware_does() {
    use djangors_core::request::Request;
    use djangors_orm::queryset::QuerySet;
    use hyper::http::Extensions;

    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let db = connect_db().await;
    setup_tables(&db).await;

    let now = chrono::Utc::now();
    let tenant_a = Tenant {
        id: 0,
        name: "Tenant A".to_string(),
        slug: "tenant-a-scope-test".to_string(),
        is_active: true,
        created_at: now,
    }
    .save(&db)
    .await
    .unwrap();
    let _tenant_b = Tenant {
        id: 0,
        name: "Tenant B".to_string(),
        slug: "tenant-b-scope-test".to_string(),
        is_active: true,
        created_at: now,
    }
    .save(&db)
    .await
    .unwrap();

    let mut extensions = Extensions::new();
    extensions.insert(CurrentTenant(tenant_a.id));
    let req = Request::new(
        hyper::Method::GET,
        hyper::Uri::from_static("http://example.com/"),
        hyper::HeaderMap::new(),
        bytes::Bytes::new(),
    )
    .with_extensions(extensions);

    let qs = QuerySet::<Tenant>::new();
    let scoped =
        crate::tenant_scope(&req, qs, "id").expect("must resolve CurrentTenant from extensions");
    let rows = scoped.all(&db).await.unwrap();

    assert_eq!(
        rows.len(),
        1,
        "must see exactly tenant A's own row, not tenant B's or both"
    );
    assert_eq!(rows[0].id, tenant_a.id);
}
