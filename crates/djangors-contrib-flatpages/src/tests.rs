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
    // Dialect-aware rather than hardcoded Postgres. This test previously used BIGSERIAL,
    // `$n` placeholders, and `Database::pool()` - all three are Postgres-only, and `pool()`
    // panics outright on a SQLite handle. It never surfaced because nothing ran this crate
    // against SQLite until the test-sqlite CI job existed.
    let dialect = db.database().dialect();
    db.create_table(&format!(
            "CREATE TABLE IF NOT EXISTS djangors_flatpage (id {}, url VARCHAR(255) UNIQUE NOT NULL, title VARCHAR(255) NOT NULL, content TEXT NOT NULL)",
            dialect.auto_pk_type()
        ))
        .await
        .unwrap();
    db.database()
        .conn()
        .execute("DELETE FROM djangors_flatpage", &[])
        .await
        .unwrap();
    let insert_sql = format!(
        "INSERT INTO djangors_flatpage (url, title, content) VALUES ({}, {}, {})",
        dialect.placeholder(1),
        dialect.placeholder(2),
        dialect.placeholder(3)
    );
    db.database()
        .conn()
        .execute(
            &insert_sql,
            &[
                djangors_orm::BindValue::Text("/about/".to_string()),
                djangors_orm::BindValue::Text("About".to_string()),
                djangors_orm::BindValue::Text("<h1>About &amp; Us</h1>".to_string()),
            ],
        )
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
