//! REST framework core for Djangors: generic serialization, ViewSets, and router mounting.
//!
//! # Security Warning
//!
//! **IMPORTANT**: This initial version (Phase 8.1) includes **ZERO access control / permissions**.
//! Every route mounted via [`viewset_routes`] is open to unauthenticated access. Do **NOT** mount
//! these routes in production or on endpoints serving real sensitive data until Phase 8.2
//! (Authentication & Permissions) lands.

use std::collections::HashMap;
use std::marker::PhantomData;

use djangors_core::error::DjangorsError;
use djangors_core::pagination::Paginator;
use djangors_core::path_params::PathParams;
use djangors_core::request::Request;
use djangors_core::response::Response;
use djangors_core::Router;
use djangors_orm::expr::{SetExpr, UnresolvedCompare, UnresolvedExpr, Value};
use djangors_orm::meta::{FieldKind, Model};
use djangors_orm::queryset::QuerySet;
use djangors_orm::FromRow;
use hyper::StatusCode;

/// Default page size for REST ViewSet list pagination.
/// Matches the admin's per-page convention (100).
pub const REST_PER_PAGE: i64 = 100;

/// Serializes any [`Model`]'s `field_values()` into a `serde_json::Value` object.
/// Relation fields serialize as their raw related id integer/null.
pub fn serialize<M: Model>(instance: &M) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, value) in instance.field_values() {
        let json_val = match value {
            Value::I64(n) => serde_json::Value::Number(n.into()),
            Value::F64(f) => serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::Text(s) => serde_json::Value::String(s),
            Value::Bool(b) => serde_json::Value::Bool(b),
            Value::DateTime(dt) => serde_json::Value::String(dt.to_rfc3339()),
            Value::Null => serde_json::Value::Null,
        };
        map.insert(name.to_string(), json_val);
    }
    serde_json::Value::Object(map)
}

/// Deserializes a JSON object into field values suitable for `QuerySet::insert_raw` / `update`,
/// using per-`FieldKind` parsing conventions. Collects ALL validation errors at once into a
/// `HashMap<String, String>` (mapping field names to error messages).
pub fn deserialize<M: Model>(
    json: &serde_json::Value,
) -> Result<Vec<(&'static str, Value)>, HashMap<String, String>> {
    let obj = match json.as_object() {
        Some(o) => o,
        None => {
            let mut errs = HashMap::new();
            errs.insert(
                "non_field_errors".to_string(),
                "Expected JSON object".to_string(),
            );
            return Err(errs);
        }
    };

    let meta = M::meta();
    let mut errors = HashMap::new();
    let mut values = Vec::new();

    for field in meta.fields {
        if field.auto {
            continue;
        }

        let val_opt = obj.get(field.name);

        if field.kind == FieldKind::Boolean {
            match val_opt {
                None | Some(serde_json::Value::Null) => {
                    if field.nullable {
                        values.push((field.name, Value::Null));
                    } else {
                        values.push((field.name, Value::Bool(false)));
                    }
                }
                Some(serde_json::Value::Bool(b)) => {
                    values.push((field.name, Value::Bool(*b)));
                }
                Some(serde_json::Value::String(s)) => match s.as_str() {
                    "true" | "1" => values.push((field.name, Value::Bool(true))),
                    "false" | "0" => values.push((field.name, Value::Bool(false))),
                    _ => {
                        errors.insert(
                            field.name.to_string(),
                            format!("Field '{}' must be a valid boolean.", field.name),
                        );
                    }
                },
                _ => {
                    errors.insert(
                        field.name.to_string(),
                        format!("Field '{}' must be a valid boolean.", field.name),
                    );
                }
            }
            continue;
        }

        match val_opt {
            None | Some(serde_json::Value::Null) => {
                if field.nullable {
                    values.push((field.name, Value::Null));
                } else {
                    errors.insert(
                        field.name.to_string(),
                        format!("Field '{}' is required.", field.name),
                    );
                }
            }
            Some(v) => match field.kind {
                FieldKind::Char
                | FieldKind::Text
                | FieldKind::Email
                | FieldKind::Url
                | FieldKind::Slug
                | FieldKind::Ip
                | FieldKind::Binary
                | FieldKind::Json => {
                    if let Some(s) = v.as_str() {
                        values.push((field.name, Value::Text(s.to_string())));
                    } else {
                        errors.insert(
                            field.name.to_string(),
                            format!("Field '{}' must be a string.", field.name),
                        );
                    }
                }
                FieldKind::Integer | FieldKind::BigInt => {
                    if let Some(n) = v.as_i64() {
                        values.push((field.name, Value::I64(n)));
                    } else if let Some(s) = v.as_str() {
                        match s.parse::<i64>() {
                            Ok(n) => values.push((field.name, Value::I64(n))),
                            Err(_) => {
                                errors.insert(
                                    field.name.to_string(),
                                    format!("Field '{}' must be a valid integer.", field.name),
                                );
                            }
                        }
                    } else {
                        errors.insert(
                            field.name.to_string(),
                            format!("Field '{}' must be a valid integer.", field.name),
                        );
                    }
                }
                FieldKind::Float => {
                    if let Some(n) = v.as_f64() {
                        values.push((field.name, Value::F64(n)));
                    } else if let Some(s) = v.as_str() {
                        match s.parse::<f64>() {
                            Ok(n) => values.push((field.name, Value::F64(n))),
                            Err(_) => {
                                errors.insert(
                                    field.name.to_string(),
                                    format!("Field '{}' must be a valid float.", field.name),
                                );
                            }
                        }
                    } else {
                        errors.insert(
                            field.name.to_string(),
                            format!("Field '{}' must be a valid float.", field.name),
                        );
                    }
                }
                FieldKind::DateTime => {
                    if let Some(s) = v.as_str() {
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                            values.push((
                                field.name,
                                Value::DateTime(dt.with_timezone(&chrono::Utc)),
                            ));
                        } else if let Ok(naive) =
                            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                        {
                            let dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                                naive,
                                chrono::Utc,
                            );
                            values.push((field.name, Value::DateTime(dt)));
                        } else {
                            errors.insert(
                                field.name.to_string(),
                                format!(
                                    "Field '{}' must be in YYYY-MM-DD HH:MM:SS format.",
                                    field.name
                                ),
                            );
                        }
                    } else {
                        errors.insert(
                            field.name.to_string(),
                            format!(
                                "Field '{}' must be in YYYY-MM-DD HH:MM:SS format.",
                                field.name
                            ),
                        );
                    }
                }
                FieldKind::Decimal { .. }
                | FieldKind::Date
                | FieldKind::Time
                | FieldKind::Duration
                | FieldKind::Uuid => {
                    errors.insert(
                        field.name.to_string(),
                        format!(
                            "Unsupported FieldKind {:?} for field '{}'.",
                            field.kind, field.name
                        ),
                    );
                }
                FieldKind::Boolean => unreachable!(),
            },
        }
    }

    for rel in meta.relations {
        let val_opt = obj.get(rel.field_name);
        match val_opt {
            None | Some(serde_json::Value::Null) => {
                values.push((rel.field_name, Value::Null));
            }
            Some(v) => {
                if let Some(n) = v.as_i64() {
                    values.push((rel.field_name, Value::I64(n)));
                } else if let Some(s) = v.as_str() {
                    match s.parse::<i64>() {
                        Ok(n) => values.push((rel.field_name, Value::I64(n))),
                        Err(_) => {
                            errors.insert(
                                rel.field_name.to_string(),
                                format!("Field '{}' must be a valid integer ID.", rel.field_name),
                            );
                        }
                    }
                } else {
                    errors.insert(
                        rel.field_name.to_string(),
                        format!("Field '{}' must be a valid integer ID.", rel.field_name),
                    );
                }
            }
        }
    }

    if !errors.is_empty() {
        Err(errors)
    } else {
        Ok(values)
    }
}

/// Generic ViewSet controller for model `M`.
///
/// Implements standard REST CRUD handlers:
/// - `list` (GET): Paginated list of records
/// - `retrieve` (GET /{pk}): Single record details
/// - `create` (POST): Create a new record
/// - `update` (PUT / PATCH /{pk}): Update an existing record
/// - `destroy` (DELETE /{pk}): Remove a record
pub struct ViewSet<M: Model + FromRow> {
    _marker: PhantomData<M>,
}

impl<M> ViewSet<M>
where
    M: Model + FromRow + Send + Sync + 'static,
{
    /// `GET /` — returns paginated list of records.
    pub async fn list(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
        let db = req
            .state::<djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

        let page: i64 = req
            .raw_query()
            .and_then(parse_page_param)
            .unwrap_or(1)
            .max(1);

        let total_items = QuerySet::<M>::new()
            .count(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let paginator = Paginator::new(total_items, REST_PER_PAGE);
        let total_pages = paginator.total_pages();
        let offset = paginator.offset(page);

        let items = QuerySet::<M>::new()
            .limit(REST_PER_PAGE)
            .offset(offset)
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let results: Vec<serde_json::Value> = items.iter().map(serialize).collect();

        let body = serde_json::json!({
            "count": total_items,
            "page": page,
            "total_pages": total_pages,
            "results": results,
        });

        Response::json(StatusCode::OK, &body)
    }

    /// `GET /{pk}` — returns a single record by primary key, or 404.
    pub async fn retrieve(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let db = req
            .state::<djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

        let pk_str = params.get("pk").ok_or_else(|| {
            DjangorsError::BadRequest("Missing primary key parameter".to_string())
        })?;
        let pk: i64 = pk_str
            .parse()
            .map_err(|_| DjangorsError::BadRequest("Invalid primary key".to_string()))?;

        let meta = M::meta();
        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;

        let unresolved_cmp = UnresolvedExpr::And(vec![UnresolvedCompare {
            field: pk_field.name,
            value: Value::I64(pk),
        }]);

        let row_opt = QuerySet::<M>::new()
            .filter(unresolved_cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await;

        match row_opt {
            Ok(item) => Response::json(StatusCode::OK, &serialize(&item)),
            Err(djangors_orm::error::OrmError::NotFound { .. }) => Err(DjangorsError::NotFound),
            Err(e) => Err(DjangorsError::Internal(e.to_string())),
        }
    }

    /// `POST /` — creates a new record from JSON body (201 Created or 422 Unprocessable Entity).
    pub async fn create(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
        let db = req
            .state::<djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

        let body_bytes = req.body_bytes().await;
        let json_val: serde_json::Value = serde_json::from_slice(body_bytes)
            .map_err(|e| DjangorsError::BadRequest(format!("Failed to parse JSON body: {e}")))?;

        let field_values = match deserialize::<M>(&json_val) {
            Ok(vals) => vals,
            Err(errors) => {
                let err_body = serde_json::json!({
                    "errors": errors
                });
                return Response::json(StatusCode::UNPROCESSABLE_ENTITY, &err_body);
            }
        };

        let pk = QuerySet::<M>::insert_raw(db, field_values)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let meta = M::meta();
        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;

        let unresolved_cmp = UnresolvedExpr::And(vec![UnresolvedCompare {
            field: pk_field.name,
            value: Value::I64(pk),
        }]);

        let created_item = QuerySet::<M>::new()
            .filter(unresolved_cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Response::json(StatusCode::CREATED, &serialize(&created_item))
    }

    /// `PUT /{pk}` / `PATCH /{pk}` — updates an existing record (200 OK, 422 Unprocessable Entity, or 404).
    pub async fn update(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let db = req
            .state::<djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

        let pk_str = params.get("pk").ok_or_else(|| {
            DjangorsError::BadRequest("Missing primary key parameter".to_string())
        })?;
        let pk: i64 = pk_str
            .parse()
            .map_err(|_| DjangorsError::BadRequest("Invalid primary key".to_string()))?;

        let body_bytes = req.body_bytes().await;
        let json_val: serde_json::Value = serde_json::from_slice(body_bytes)
            .map_err(|e| DjangorsError::BadRequest(format!("Failed to parse JSON body: {e}")))?;

        let field_values = match deserialize::<M>(&json_val) {
            Ok(vals) => vals,
            Err(errors) => {
                let err_body = serde_json::json!({
                    "errors": errors
                });
                return Response::json(StatusCode::UNPROCESSABLE_ENTITY, &err_body);
            }
        };

        let meta = M::meta();
        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;

        let unresolved_cmp = UnresolvedExpr::And(vec![UnresolvedCompare {
            field: pk_field.name,
            value: Value::I64(pk),
        }]);

        let qs = QuerySet::<M>::new()
            .filter(unresolved_cmp.clone())
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let sets: Vec<(&'static str, SetExpr)> = field_values
            .into_iter()
            .map(|(col, val)| (col, SetExpr::Literal(val)))
            .collect();

        let updated_rows = qs
            .update(db, sets)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        if updated_rows == 0 {
            return Err(DjangorsError::NotFound);
        }

        let updated_item = QuerySet::<M>::new()
            .filter(unresolved_cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Response::json(StatusCode::OK, &serialize(&updated_item))
    }

    /// `DELETE /{pk}` — deletes a record by primary key (204 No Content or 404).
    pub async fn destroy(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let db = req
            .state::<djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

        let pk_str = params.get("pk").ok_or_else(|| {
            DjangorsError::BadRequest("Missing primary key parameter".to_string())
        })?;
        let pk: i64 = pk_str
            .parse()
            .map_err(|_| DjangorsError::BadRequest("Invalid primary key".to_string()))?;

        let deleted_count = QuerySet::<M>::delete_by_pk(db, pk)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        if deleted_count == 0 {
            return Err(DjangorsError::NotFound);
        }

        Ok(Response::bytes(
            StatusCode::NO_CONTENT,
            "text/plain",
            Vec::new(),
        ))
    }
}

fn parse_page_param(query: &str) -> Option<i64> {
    for pair in query.split('&') {
        if let Some((key, val)) = pair.split_once('=') {
            if key == "page" {
                return val.parse::<i64>().ok();
            }
        }
    }
    None
}

/// Mounts standard REST routes for model `M` onto `router` at `base_path`.
///
/// Route layout:
/// - `GET {base_path}` -> list
/// - `POST {base_path}` -> create
/// - `GET {base_path}/{pk:i64}` -> retrieve
/// - `PUT {base_path}/{pk:i64}` -> update
/// - `PATCH {base_path}/{pk:i64}` -> update
/// - `DELETE {base_path}/{pk:i64}` -> destroy
///
/// # Security Warning
///
/// **WARNING**: This v1 implementation has zero access control. Routes mounted with
/// `viewset_routes` are publicly accessible. Do NOT mount on production routes or
/// routes serving sensitive data until Phase 8.2 (Authentication & Permissions) lands.
pub fn viewset_routes<M>(router: Router, base_path: &str) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
{
    let clean_base = base_path.trim_end_matches('/');
    let detail_path = format!("{clean_base}/{{pk:i64}}");
    let list_create_path = if clean_base.is_empty() {
        "/"
    } else {
        clean_base
    };

    router
        .get(list_create_path, ViewSet::<M>::list)
        .post(list_create_path, ViewSet::<M>::create)
        .get(&detail_path, ViewSet::<M>::retrieve)
        .put(&detail_path, ViewSet::<M>::update)
        .route(
            &detail_path,
            hyper::http::Method::PATCH,
            ViewSet::<M>::update,
        )
        .delete(&detail_path, ViewSet::<M>::destroy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use djangors_macros::Model as DeriveModel;
    use hyper::http::{header::CONTENT_TYPE, HeaderMap, Method, Uri};
    use std::sync::Mutex;

    static DB_MUTEX: Mutex<()> = Mutex::new(());

    #[derive(DeriveModel, Debug, Clone, sqlx::FromRow)]
    #[djangors(app = "rest_test", table_name = "rest_test_category")]
    pub struct TestCategory {
        #[djangors(primary_key, auto)]
        pub id: i64,
        pub name: String,
    }

    #[derive(DeriveModel, Debug, Clone, sqlx::FromRow)]
    #[djangors(app = "rest_test", table_name = "rest_test_article")]
    pub struct TestArticle {
        #[djangors(primary_key, auto)]
        pub id: i64,
        pub title: String,
        pub view_count: i64,
        pub is_published: bool,
        pub published_at: chrono::DateTime<chrono::Utc>,
        #[djangors(foreign_key(on_delete = "cascade"))]
        pub category: djangors_orm::ForeignKey<TestCategory>,
    }

    #[test]
    fn test_serialization_round_trip() {
        let now = chrono::Utc::now();
        let article = TestArticle {
            id: 1,
            title: "Rust REST Framework".to_string(),
            view_count: 42,
            is_published: true,
            published_at: now,
            category: djangors_orm::ForeignKey::new(10),
        };

        let json = serialize(&article);
        assert_eq!(json["id"], 1);
        assert_eq!(json["title"], "Rust REST Framework");
        assert_eq!(json["view_count"], 42);
        assert_eq!(json["is_published"], true);
        assert_eq!(json["published_at"], now.to_rfc3339());
        assert_eq!(json["category"], 10);

        let deserialized_values = deserialize::<TestArticle>(&json).unwrap();
        assert_eq!(
            deserialized_values,
            vec![
                ("title", Value::Text("Rust REST Framework".to_string())),
                ("view_count", Value::I64(42)),
                ("is_published", Value::Bool(true)),
                ("published_at", Value::DateTime(now)),
                ("category", Value::I64(10)),
            ]
        );
    }

    #[test]
    fn test_deserialization_multiple_errors() {
        let bad_json = serde_json::json!({
            "title": 12345, // invalid, must be string
            "view_count": "not_an_int", // invalid, must be integer
            "is_published": "maybe", // invalid, must be boolean
            "published_at": "invalid_date_string", // invalid datetime
            "category": "not_an_id" // invalid relation ID
        });

        let errs = deserialize::<TestArticle>(&bad_json).unwrap_err();
        assert!(errs.contains_key("title"));
        assert!(errs.contains_key("view_count"));
        assert!(errs.contains_key("is_published"));
        assert!(errs.contains_key("published_at"));
        assert!(errs.contains_key("category"));
        assert!(errs.get("title").unwrap().contains("must be a string"));
        assert!(errs.get("view_count").unwrap().contains("valid integer"));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_viewset_crud_end_to_end() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS rest_test_article")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS rest_test_category")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE rest_test_category (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(100) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE rest_test_article (
                id BIGSERIAL PRIMARY KEY,
                title VARCHAR(200) NOT NULL,
                view_count BIGINT NOT NULL,
                is_published BOOLEAN NOT NULL,
                published_at TIMESTAMPTZ NOT NULL,
                category BIGINT NOT NULL REFERENCES rest_test_category(id) ON DELETE CASCADE
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let cat = TestCategory {
            id: 0,
            name: "Tech".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let router =
            viewset_routes::<TestArticle>(Router::new(), "/api/articles").with_state(db.clone());

        // 1. Create Article via POST (Validation Error test)
        let invalid_create_body = serde_json::json!({
            "view_count": "invalid",
            "category": cat.id
        });
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
        let req = Request::new(
            Method::POST,
            Uri::from_static("/api/articles"),
            headers.clone(),
            Bytes::from(serde_json::to_vec(&invalid_create_body).unwrap()),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body_str = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body_str.contains("errors"));
        assert!(body_str.contains("Field 'title' is required."));

        // 2. Create Article via POST (Success)
        let valid_create_body = serde_json::json!({
            "title": "First Article",
            "view_count": 100,
            "is_published": true,
            "published_at": "2026-07-22T14:00:00Z",
            "category": cat.id
        });
        let req = Request::new(
            Method::POST,
            Uri::from_static("/api/articles"),
            headers.clone(),
            Bytes::from(serde_json::to_vec(&valid_create_body).unwrap()),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let created_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(created_json["id"], 1);
        assert_eq!(created_json["title"], "First Article");

        // 3. List Articles via GET
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let list_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(list_json["count"], 1);
        assert_eq!(list_json["page"], 1);
        assert_eq!(list_json["total_pages"], 1);
        assert_eq!(list_json["results"].as_array().unwrap().len(), 1);

        // 4. Retrieve Article via GET /{pk}
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles/1"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let retrieve_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(retrieve_json["id"], 1);

        // Retrieve 404 test
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles/999"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await;
        assert!(matches!(res, Err(DjangorsError::NotFound)));

        // 5. Update Article via PUT /{pk}
        let update_body = serde_json::json!({
            "title": "Updated Article Title",
            "view_count": 200,
            "is_published": false,
            "published_at": "2026-07-22T15:00:00Z",
            "category": cat.id
        });
        let req = Request::new(
            Method::PUT,
            Uri::from_static("/api/articles/1"),
            headers.clone(),
            Bytes::from(serde_json::to_vec(&update_body).unwrap()),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let updated_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(updated_json["title"], "Updated Article Title");
        assert_eq!(updated_json["view_count"], 200);

        // 6. Destroy Article via DELETE /{pk}
        let req = Request::new(
            Method::DELETE,
            Uri::from_static("/api/articles/1"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        // Confirm deleted
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles/1"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await;
        assert!(matches!(res, Err(DjangorsError::NotFound)));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_viewset_pagination() {
        let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        let _ = sqlx::query("DROP TABLE IF EXISTS rest_test_article")
            .execute(db.pool())
            .await;
        let _ = sqlx::query("DROP TABLE IF EXISTS rest_test_category")
            .execute(db.pool())
            .await;

        sqlx::query(
            "CREATE TABLE rest_test_category (
                id BIGSERIAL PRIMARY KEY,
                name VARCHAR(100) NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE rest_test_article (
                id BIGSERIAL PRIMARY KEY,
                title VARCHAR(200) NOT NULL,
                view_count BIGINT NOT NULL,
                is_published BOOLEAN NOT NULL,
                published_at TIMESTAMPTZ NOT NULL,
                category BIGINT NOT NULL REFERENCES rest_test_category(id) ON DELETE CASCADE
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let cat = TestCategory {
            id: 0,
            name: "Tech".to_string(),
        }
        .save(&db)
        .await
        .unwrap();

        let now = chrono::Utc::now();
        // Insert 105 articles to span 2 pages (REST_PER_PAGE = 100)
        for i in 1..=105 {
            sqlx::query(
                "INSERT INTO rest_test_article (title, view_count, is_published, published_at, category) VALUES ($1, $2, $3, $4, $5)"
            )
            .bind(format!("Article {i}"))
            .bind(i as i64)
            .bind(true)
            .bind(now)
            .bind(cat.id)
            .execute(db.pool())
            .await
            .unwrap();
        }

        let router =
            viewset_routes::<TestArticle>(Router::new(), "/api/articles").with_state(db.clone());

        // Page 1 (default page = 1)
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles?page=1"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let page1_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(page1_json["count"], 105);
        assert_eq!(page1_json["page"], 1);
        assert_eq!(page1_json["total_pages"], 2);
        assert_eq!(page1_json["results"].as_array().unwrap().len(), 100);

        // Page 2
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles?page=2"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let page2_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(page2_json["count"], 105);
        assert_eq!(page2_json["page"], 2);
        assert_eq!(page2_json["total_pages"], 2);
        assert_eq!(page2_json["results"].as_array().unwrap().len(), 5);
    }
}
