#![deny(missing_docs)]
//! Server-rendered generic class-based views for Djangors.
//!
//! This crate provides reusable, generic views that mimic Django's class-based views:
//! - [`ListView`]: Displays multiple records of a model.
//! - [`DetailView`]: Displays a single detailed model instance.
//! - [`CreateView`]: Handles rendering and submission of forms to create a new model instance.
//! - [`UpdateView`]: Handles editing and updating an existing model instance.
//! - [`DeleteView`]: Renders a deletion confirmation page and deletes the object upon POST.
//!
//! Each view takes a [`ViewSetConfig`] specifying the engine, template name, and redirect targets.

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

// Converts a model instance's fields and their values into a JSON object.
// Used to construct template context where fields are exposed under their string names.
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
                // Only reachable through a projection such as
                // `QuerySet::values`, never from a plain model field, but
                // render it as an array so a template sees a real list.
                Value::List(items) => serde_json::Value::Array(
                    items
                        .into_iter()
                        .map(|item| match item {
                            Value::I64(v) => v.into(),
                            Value::F64(v) => serde_json::json!(v),
                            Value::Text(v) => v.into(),
                            Value::Bool(v) => v.into(),
                            Value::DateTime(v) => v.to_rfc3339().into(),
                            _ => serde_json::Value::Null,
                        })
                        .collect(),
                ),
            },
        );
    }
    serde_json::Value::Object(map)
}

// Formats a `FormErrors` instance into a JSON object suitable for template rendering.
// Field-specific errors are mapped to their field names, and non-field errors are mapped to `__all__`.
fn errors_context(errors: &djangors_orm::djangors_forms::FormErrors) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, error) in &errors.fields {
        map.insert(name.clone(), error.0.clone().into());
    }
    map.insert("__all__".into(), errors.non_field.clone().into());
    serde_json::Value::Object(map)
}

// Helper to look up the primary key field name of the model and extract its `i64` value.
// Fails with an internal error if the primary key field cannot be found or is not an integer.
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

// Queries the database to retrieve a single model instance matching the `pk` parameter from path parameters.
async fn find<M: Model + FromRow>(req: &Request, params: &PathParams) -> Result<M, DjangorsError> {
    let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
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

// Renders the specified template with the given context and returns a 200 OK HTML response.
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
        let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
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
        let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
        let pk = pk_value(&object)?;
        djangors_orm::QuerySet::<M>::delete_by_pk(db, pk)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        Ok(Response::redirect(config.success_url))
    }
}

#[cfg(test)]
mod tests;
