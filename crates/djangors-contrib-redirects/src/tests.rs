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
    // Build the DDL and the seed row through the dialect/ORM rather than raw Postgres SQL:
    // `BIGSERIAL`, the `$1` placeholders, and `Database::pool()` are all Postgres-only, so the
    // original version of this test panicked outright under `TEST_BACKEND=sqlite`.
    let auto_pk = db.database().dialect().auto_pk_type();
    db.create_table(&format!(
        "CREATE TABLE IF NOT EXISTS djangors_redirect (id {auto_pk}, old_path VARCHAR(255) UNIQUE NOT NULL, new_path VARCHAR(255) NOT NULL)"
    ))
    .await
    .unwrap();
    Redirect::objects().delete(db.database()).await.unwrap();
    Redirect {
        id: 0,
        old_path: "/old/".to_string(),
        new_path: "/new/".to_string(),
    }
    .save(db.database())
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
