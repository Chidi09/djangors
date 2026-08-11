use super::tests::{TestArticle, TestCategory};
use super::*;
use bytes::Bytes;
use djangors_orm::queryset::QuerySet;
use hyper::http::{HeaderMap, Method, Uri};
use std::str::FromStr;

fn req(query: &str) -> Request {
    Request::new(
        Method::GET,
        Uri::from_str(&format!("/articles?{query}")).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
}

/// The backends build a `QuerySet`, so assert on the SQL it compiles to.
fn sql_of(qs: QuerySet<TestArticle>) -> String {
    qs.debug_sql()
}

#[test]
fn field_filter_supports_lookup_suffixes() {
    let backend = FieldFilter::new(&["view_count", "title"]);
    let qs = backend
        .filter_queryset(&req("view_count__gte=10"), QuerySet::<TestArticle>::new())
        .unwrap();
    let sql = sql_of(qs);
    assert!(sql.contains(">="), "expected >= in `{sql}`");
}

#[test]
fn field_filter_in_lookup_splits_on_commas() {
    let backend = FieldFilter::new(&["view_count"]);
    let qs = backend
        .filter_queryset(&req("view_count__in=1,2,3"), QuerySet::<TestArticle>::new())
        .unwrap();
    let sql = sql_of(qs);
    assert!(sql.contains("IN ($1, $2, $3)"), "got `{sql}`");
}

#[test]
fn field_filter_ignores_fields_outside_the_allowlist() {
    // `title` is not allowlisted here, so the parameter must be dropped
    // rather than reaching SQL.
    let backend = FieldFilter::new(&["view_count"]);
    let qs = backend
        .filter_queryset(
            &req("title__icontains=secret"),
            QuerySet::<TestArticle>::new(),
        )
        .unwrap();
    let sql = sql_of(qs);
    assert!(
        !sql.contains("title"),
        "non-allowlisted field leaked into `{sql}`"
    );
}

#[test]
fn field_filter_ignores_unknown_lookup_suffixes() {
    let backend = FieldFilter::new(&["view_count"]);
    let qs = backend
        .filter_queryset(
            &req("view_count__droptable=1"),
            QuerySet::<TestArticle>::new(),
        )
        .unwrap();
    let sql = sql_of(qs);
    assert!(!sql.contains("droptable"), "got `{sql}`");
}

#[test]
fn search_filter_ors_across_configured_fields() {
    let backend = SearchFilter::new(&["title"]);
    let qs = backend
        .filter_queryset(&req("search=rust"), QuerySet::<TestArticle>::new())
        .unwrap();
    let sql = sql_of(qs);
    assert!(sql.contains("ILIKE"), "got `{sql}`");
}

#[test]
fn search_filter_is_a_no_op_without_the_parameter() {
    let backend = SearchFilter::new(&["title"]);
    let qs = backend
        .filter_queryset(&req("page=1"), QuerySet::<TestArticle>::new())
        .unwrap();
    assert!(!sql_of(qs).contains("ILIKE"));
}

#[test]
fn ordering_filter_honours_the_descending_prefix() {
    let backend = OrderingFilter::new(&["view_count"]);
    let qs = backend
        .filter_queryset(&req("ordering=-view_count"), QuerySet::<TestArticle>::new())
        .unwrap();
    let sql = sql_of(qs);
    assert!(sql.contains("DESC"), "got `{sql}`");
}

#[test]
fn ordering_filter_ignores_fields_outside_the_allowlist() {
    let backend = OrderingFilter::new(&["view_count"]);
    let qs = backend
        .filter_queryset(&req("ordering=title"), QuerySet::<TestArticle>::new())
        .unwrap();
    let sql = sql_of(qs);
    assert!(
        !sql.contains("ORDER BY \"title\""),
        "non-allowlisted ordering leaked into `{sql}`"
    );
}

#[test]
fn backends_compose_in_order() {
    let backends: Vec<std::sync::Arc<dyn FilterBackend<TestArticle>>> = vec![
        std::sync::Arc::new(FieldFilter::new(&["view_count"])),
        std::sync::Arc::new(SearchFilter::new(&["title"])),
        std::sync::Arc::new(OrderingFilter::new(&["view_count"])),
    ];
    let qs = apply_backends(
        &backends,
        &req("view_count__gte=5&search=rust&ordering=-view_count"),
        QuerySet::<TestArticle>::new(),
    )
    .unwrap();
    let sql = sql_of(qs);
    assert!(sql.contains(">="), "got `{sql}`");
    assert!(sql.contains("ILIKE"), "got `{sql}`");
    assert!(sql.contains("DESC"), "got `{sql}`");
}

#[test]
fn nested_serializer_embeds_the_related_object() {
    let article = TestArticle {
        id: 1,
        title: "Nested".to_string(),
        view_count: 3,
        is_published: true,
        published_at: chrono::Utc::now(),
        category: djangors_orm::ForeignKey::new(7),
    };
    let category = TestCategory {
        id: 7,
        name: "Rust".to_string(),
    };

    let serializer = NestedSerializer::new(
        ModelSerializer::<TestArticle>::default(),
        "category",
        ModelSerializer::<TestCategory>::default(),
    );

    let flat = serializer.render(&article, None);
    assert_eq!(
        flat.get("category").and_then(|v| v.as_i64()),
        Some(7),
        "without a loaded relation the raw id must survive"
    );

    let nested = serializer.render(&article, Some(&category));
    let embedded = nested.get("category").expect("category key");
    assert_eq!(
        embedded.get("name").and_then(|v| v.as_str()),
        Some("Rust"),
        "expected the related object embedded, got {embedded:?}"
    );
}
