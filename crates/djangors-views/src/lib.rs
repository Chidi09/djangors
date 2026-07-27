#![deny(missing_docs)]
//! Server-rendered generic class-based views for Djangors.

use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response};
use djangors_orm::expr::{SetExpr, UnresolvedCompare, UnresolvedExpr, Value};
use djangors_orm::{FromRow, Model, ModelForm};
use djangors_template::TemplateEngine;
use hyper::{Method, StatusCode};
use std::collections::HashMap;

/// Configuration shared by generic view handlers.
pub struct ViewSetConfig<'a> {
    /// Template engine used to render responses.
    pub engine: &'a TemplateEngine,
    /// Template used by the view.
    pub template_name: &'a str,
    /// Redirect destination after a successful write.
    pub success_url: &'a str,
}

fn model_context<M: Model>(model: &M) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, value) in model.field_values() {
        map.insert(
            name.to_string(),
            match value {
                Value::I64(v) => v.into(),
                Value::F64(v) => serde_json::json!(v),
                Value::Text(v) => v.into(),
                Value::Bool(v) => v.into(),
                Value::DateTime(v) => v.to_rfc3339().into(),
                Value::Null => serde_json::Value::Null,
            },
        );
    }
    serde_json::Value::Object(map)
}

fn errors_context(errors: &djangors_orm::djangors_forms::FormErrors) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, error) in &errors.fields {
        map.insert(name.clone(), error.0.clone().into());
    }
    map.insert("__all__".into(), errors.non_field.clone().into());
    serde_json::Value::Object(map)
}

fn pk_value<M: Model>(model: &M) -> Result<i64, DjangorsError> {
    let name = M::meta()
        .fields
        .iter()
        .find(|f| f.primary_key)
        .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
        .name;
    model
        .field_values()
        .into_iter()
        .find(|(field, _)| *field == name)
        .and_then(|(_, value)| match value {
            Value::I64(v) => Some(v),
            _ => None,
        })
        .ok_or_else(|| DjangorsError::Internal("Primary key is not an integer".into()))
}

async fn find<M: Model + FromRow>(req: &Request, params: &PathParams) -> Result<M, DjangorsError> {
    let db = req
        .state::<djangors_orm::djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database connection not found".into()))?;
    let pk: i64 = params.get_as("pk")?;
    let field = M::meta()
        .fields
        .iter()
        .find(|f| f.primary_key)
        .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
        .name;
    M::objects()
        .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
            field,
            value: Value::I64(pk),
        }]))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|e| match e {
            djangors_orm::OrmError::NotFound { .. } => DjangorsError::NotFound,
            _ => DjangorsError::Internal(e.to_string()),
        })
}

fn render(
    engine: &TemplateEngine,
    name: &str,
    context: serde_json::Value,
) -> Result<Response, DjangorsError> {
    engine
        .render(name, context)
        .map(|body| Response::html(StatusCode::OK, body))
        .map_err(|e| DjangorsError::Internal(e.to_string()))
}

/// Generic list view rendering all rows.
pub struct ListView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow> ListView<M> {
    /// Handle a list request.
    pub async fn list(
        req: Request,
        _params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let db = req
            .state::<djangors_orm::djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("Database connection not found".into()))?;
        let rows = M::objects()
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        let context =
            serde_json::json!({"object_list": rows.iter().map(model_context).collect::<Vec<_>>()});
        render(config.engine, config.template_name, context)
    }
}

/// Generic detail view rendering one row.
pub struct DetailView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow> DetailView<M> {
    /// Handle a detail request.
    pub async fn detail(
        req: Request,
        params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        render(
            config.engine,
            config.template_name,
            serde_json::json!({"object": model_context(&find::<M>(&req, &params).await?)}),
        )
    }
}

/// Generic create view.
pub struct CreateView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow + ModelForm> CreateView<M> {
    /// Handle GET and POST create requests.
    pub async fn create(
        req: Request,
        _params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        if req.method() == Method::GET {
            return render(
                config.engine,
                config.template_name,
                serde_json::json!({"fields": M::field_names(), "errors": {}}),
            );
        }
        let Form(data) = Form::<HashMap<String, String>>::from_request(&req).await?;
        match M::validate_form(&data) {
            Ok(cleaned) => {
                let db = req
                    .state::<djangors_orm::djangors_db::Database>()
                    .ok_or_else(|| {
                        DjangorsError::Internal("Database connection not found".into())
                    })?;
                djangors_orm::QuerySet::<M>::insert_raw(
                    db,
                    M::from_cleaned_form(cleaned).field_values(),
                )
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                Ok(Response::redirect(config.success_url))
            }
            Err(errors) => render(
                config.engine,
                config.template_name,
                serde_json::json!({"fields": M::field_names(), "form": data, "errors": errors_context(&errors)}),
            ),
        }
    }
}

/// Generic update view.
pub struct UpdateView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow + ModelForm> UpdateView<M> {
    /// Handle GET and POST update requests.
    pub async fn update(
        req: Request,
        params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let mut object = find::<M>(&req, &params).await?;
        if req.method() == Method::GET {
            return render(
                config.engine,
                config.template_name,
                serde_json::json!({"object": model_context(&object), "fields": M::field_names(), "errors": {}}),
            );
        }
        let Form(data) = Form::<HashMap<String, String>>::from_request(&req).await?;
        match M::validate_form(&data) {
            Ok(cleaned) => {
                let db = req
                    .state::<djangors_orm::djangors_db::Database>()
                    .ok_or_else(|| {
                        DjangorsError::Internal("Database connection not found".into())
                    })?;
                object.apply_cleaned_form(cleaned);
                let pk = pk_value(&object)?;
                let pk_field = M::meta()
                    .fields
                    .iter()
                    .find(|f| f.primary_key)
                    .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
                    .name;
                let sets = object
                    .field_values()
                    .into_iter()
                    .filter(|(name, _)| *name != pk_field)
                    .map(|(name, value)| (name, SetExpr::Literal(value)))
                    .collect();
                M::objects()
                    .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
                        field: pk_field,
                        value: Value::I64(pk),
                    }]))
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?
                    .update(db, sets)
                    .await
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                Ok(Response::redirect(config.success_url))
            }
            Err(errors) => render(
                config.engine,
                config.template_name,
                serde_json::json!({"object": model_context(&object), "form": data, "errors": errors_context(&errors)}),
            ),
        }
    }
}

/// Generic delete view.
pub struct DeleteView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow> DeleteView<M> {
    /// Handle GET confirmation and POST deletion requests.
    pub async fn delete(
        req: Request,
        params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let object = find::<M>(&req, &params).await?;
        if req.method() == Method::GET {
            return render(
                config.engine,
                config.template_name,
                serde_json::json!({"object": model_context(&object)}),
            );
        }
        let db = req
            .state::<djangors_orm::djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("Database connection not found".into()))?;
        let pk = pk_value(&object)?;
        djangors_orm::QuerySet::<M>::delete_by_pk(db, pk)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        Ok(Response::redirect(config.success_url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes as HttpBytes;
    use djangors_core::state::AppState;
    use djangors_macros::Model;
    use hyper::{HeaderMap, Method, Uri};

    #[derive(Model, Debug, Clone)]
    #[djangors(app = "test_app", table_name = "test_cbv_model")]
    pub struct CbvTestModel {
        #[djangors(primary_key, auto)]
        pub id: i64,
        #[djangors(max_length = 40)]
        pub name: String,
        pub age: i64,
    }

    // All tests below share the one real `djangors_test` database and a fixed table name
    // (the model's `table_name` is fixed at macro-expansion time, so per-test unique naming
    // isn't an option the way it is for hand-written SQL). `cargo test` runs `#[tokio::test]`
    // functions concurrently by default, so without this lock two tests racing on
    // `DROP TABLE IF EXISTS` + `CREATE TABLE` produce a "relation already exists" error -
    // confirmed directly (5/5 tests failed with Postgres error 42P07 before this lock was
    // added). Every test must acquire this guard before calling `setup()`.
    static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn setup() -> (djangors_db::Database, tempfile::TempDir, TemplateEngine) {
        let db_url = "postgres://postgres:postgres@localhost/djangors_test";
        let config = djangors_db::config::DatabaseConfig::new(db_url);
        let db = djangors_db::Database::connect(&config).await.unwrap();

        sqlx::query("DROP TABLE IF EXISTS test_cbv_model")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE test_cbv_model (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                age BIGINT NOT NULL
            )",
        )
        .execute(db.pool())
        .await
        .unwrap();

        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("list.html"),
            "{% for o in object_list %}{{ o.name }}:{{ o.age }};{% endfor %}",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("detail.html"),
            "{{ object.name }}:{{ object.age }}",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("form.html"),
            "{% if errors %}ERRORS:{% for k in errors %}{{ k }}={{ errors[k] }};{% endfor %}{% else %}OK{% endif %}",
        )
        .unwrap();
        std::fs::write(dir.path().join("confirm.html"), "confirm {{ object.name }}").unwrap();
        let engine = TemplateEngine::new(vec![dir.path().to_path_buf()]).unwrap();

        (db, dir, engine)
    }

    fn make_request(method: Method, body: &str, db: djangors_db::Database) -> Request {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        Request::new(
            method,
            Uri::from_static("/"),
            headers,
            HttpBytes::from(body.to_string()),
        )
        .with_state(AppState::new().insert(db))
    }

    fn path_with_pk(pk: i64) -> PathParams {
        let mut params = PathParams::new();
        params.insert("pk", &pk.to_string());
        params
    }

    #[tokio::test]
    async fn test_list_view_renders_real_rows() {
        let _guard = TEST_DB_LOCK.lock().await;
        let (db, _dir, engine) = setup().await;
        CbvTestModel {
            id: 0,
            name: "Alice".into(),
            age: 30,
        }
        .save(&db)
        .await
        .unwrap();
        CbvTestModel {
            id: 0,
            name: "Bob".into(),
            age: 25,
        }
        .save(&db)
        .await
        .unwrap();

        let config = ViewSetConfig {
            engine: &engine,
            template_name: "list.html",
            success_url: "/",
        };
        let req = make_request(Method::GET, "", db);
        let res = ListView::<CbvTestModel>::list(req, PathParams::new(), &config)
            .await
            .unwrap();
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Alice:30"), "body was: {body}");
        assert!(body.contains("Bob:25"), "body was: {body}");

        sqlx::query("DROP TABLE test_cbv_model")
            .execute(
                djangors_db::Database::connect(&djangors_db::config::DatabaseConfig::new(
                    "postgres://postgres:postgres@localhost/djangors_test",
                ))
                .await
                .unwrap()
                .pool(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_detail_view_renders_the_matching_row() {
        let _guard = TEST_DB_LOCK.lock().await;
        let (db, _dir, engine) = setup().await;
        let saved = CbvTestModel {
            id: 0,
            name: "Carol".into(),
            age: 40,
        }
        .save(&db)
        .await
        .unwrap();

        let config = ViewSetConfig {
            engine: &engine,
            template_name: "detail.html",
            success_url: "/",
        };
        let req = make_request(Method::GET, "", db.clone());
        let res = DetailView::<CbvTestModel>::detail(req, path_with_pk(saved.id), &config)
            .await
            .unwrap();
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert_eq!(body, "Carol:40");

        sqlx::query("DROP TABLE test_cbv_model")
            .execute(db.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_create_view_post_inserts_a_real_row_and_redirects() {
        let _guard = TEST_DB_LOCK.lock().await;
        let (db, _dir, engine) = setup().await;
        let config = ViewSetConfig {
            engine: &engine,
            template_name: "form.html",
            success_url: "/done",
        };

        // Valid data actually inserts a row and redirects.
        let req = make_request(Method::POST, "name=Dave&age=22", db.clone());
        let res = CreateView::<CbvTestModel>::create(req, PathParams::new(), &config)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(res.headers().get(hyper::header::LOCATION).unwrap(), "/done");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_cbv_model WHERE name = $1")
            .bind("Dave")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 1, "the valid submission must actually insert a row");

        // Invalid data (missing required `name`) does not insert a row and re-renders with errors.
        let req = make_request(Method::POST, "age=22", db.clone());
        let res = CreateView::<CbvTestModel>::create(req, PathParams::new(), &config)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("ERRORS"), "body was: {body}");
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_cbv_model")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(total, 1, "invalid submission must not insert any row");

        sqlx::query("DROP TABLE test_cbv_model")
            .execute(db.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_update_view_post_changes_the_existing_row_and_redirects() {
        let _guard = TEST_DB_LOCK.lock().await;
        let (db, _dir, engine) = setup().await;
        let saved = CbvTestModel {
            id: 0,
            name: "Eve".into(),
            age: 50,
        }
        .save(&db)
        .await
        .unwrap();
        let original_id = saved.id;

        let config = ViewSetConfig {
            engine: &engine,
            template_name: "form.html",
            success_url: "/updated",
        };
        let req = make_request(Method::POST, "name=EveUpdated&age=51", db.clone());
        let res = UpdateView::<CbvTestModel>::update(req, path_with_pk(original_id), &config)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(
            res.headers().get(hyper::header::LOCATION).unwrap(),
            "/updated"
        );

        let row: (String, i64) =
            sqlx::query_as("SELECT name, age FROM test_cbv_model WHERE id = $1")
                .bind(original_id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(row.0, "EveUpdated");
        assert_eq!(row.1, 51);

        sqlx::query("DROP TABLE test_cbv_model")
            .execute(db.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_delete_view_post_removes_the_row() {
        let _guard = TEST_DB_LOCK.lock().await;
        let (db, _dir, engine) = setup().await;
        let saved = CbvTestModel {
            id: 0,
            name: "Frank".into(),
            age: 60,
        }
        .save(&db)
        .await
        .unwrap();

        let config = ViewSetConfig {
            engine: &engine,
            template_name: "confirm.html",
            success_url: "/gone",
        };

        // GET renders a confirmation page without deleting anything.
        let req = make_request(Method::GET, "", db.clone());
        let res = DeleteView::<CbvTestModel>::delete(req, path_with_pk(saved.id), &config)
            .await
            .unwrap();
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        assert!(body.contains("Frank"), "body was: {body}");
        let still_there: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM test_cbv_model WHERE id = $1")
                .bind(saved.id)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(still_there, 1, "GET must not delete anything");

        // POST actually deletes the row.
        let req = make_request(Method::POST, "", db.clone());
        let res = DeleteView::<CbvTestModel>::delete(req, path_with_pk(saved.id), &config)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM test_cbv_model WHERE id = $1")
            .bind(saved.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(gone, 0, "POST must actually delete the row");

        sqlx::query("DROP TABLE test_cbv_model")
            .execute(db.pool())
            .await
            .unwrap();
    }
}
