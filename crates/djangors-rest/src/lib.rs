//! REST framework core for Djangors: generic serialization, ViewSets, and router mounting.
//!
//! ViewSet routes require an authenticated user by default. Public routes must opt into
//! [`AllowAny`] explicitly through [`viewset_routes_with_permission`].

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use djangors_auth::AuthUser;
use djangors_core::error::DjangorsError;
use djangors_core::extract::FromRequest;
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
use rand::RngCore;

/// A database-backed token for authenticating API requests.
#[derive(djangors_macros::Model, Debug, Clone)]
#[djangors(app = "djangors_rest", table_name = "djangors_rest_authtoken")]
pub struct AuthToken {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub user: djangors_orm::ForeignKey<djangors_auth::User>,
    #[djangors(max_length = 64, unique)]
    pub key: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Generates a 256-bit, hexadecimal API token key.
pub fn generate_token_key() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The user authenticated by the `Authorization: Token <key>` scheme.
#[derive(Debug, Clone)]
pub struct TokenAuth(pub djangors_auth::User);

#[async_trait::async_trait]
impl FromRequest for TokenAuth {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let key = req
            .header("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Token "))
            .filter(|key| !key.is_empty() && !key.chars().any(char::is_whitespace))
            .ok_or_else(|| DjangorsError::Unauthorized("not authenticated".to_string()))?;

        let db = req
            .state::<djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
        let token = AuthToken::objects()
            .filter(djangors_orm::q!(key = key))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| match e {
                djangors_orm::OrmError::NotFound { .. } => {
                    DjangorsError::Unauthorized("invalid token".to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;

        let user = djangors_auth::User::objects()
            .filter(djangors_orm::q!(id = token.user.id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| match e {
                djangors_orm::OrmError::NotFound { .. } => {
                    DjangorsError::Unauthorized("user not found".to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;

        if !user.is_active() {
            return Err(DjangorsError::Unauthorized("account inactive".to_string()));
        }
        Ok(TokenAuth(user))
    }
}

#[cfg(feature = "jwt")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct JwtClaims {
    user_id: i64,
    exp: u64,
}

#[cfg(feature = "jwt")]
/// Encodes a user id as an HS256 JWT signed with `secret`.
pub fn encode_jwt(user_id: i64, secret: &str) -> String {
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &JwtClaims {
            user_id,
            exp: jsonwebtoken::get_current_timestamp() + 60 * 60,
        },
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("JWT claims are serializable")
}

#[cfg(feature = "jwt")]
/// Decodes and validates an HS256 JWT, returning its user id claim.
pub fn decode_jwt(token: &str, secret: &str) -> Result<i64, jsonwebtoken::errors::Error> {
    let data = jsonwebtoken::decode::<JwtClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256),
    )?;
    Ok(data.claims.user_id)
}

#[cfg(feature = "jwt")]
/// The user authenticated by the `Authorization: Bearer <jwt>` scheme.
#[derive(Debug, Clone)]
pub struct JwtAuth(pub djangors_auth::User);

#[cfg(feature = "jwt")]
#[async_trait::async_trait]
impl FromRequest for JwtAuth {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let token = req
            .header("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .filter(|token| !token.is_empty() && !token.chars().any(char::is_whitespace))
            .ok_or_else(|| DjangorsError::Unauthorized("not authenticated".to_string()))?;
        let user_id = decode_jwt(token, settings_secret(req)?)
            .map_err(|_| DjangorsError::Unauthorized("invalid token".to_string()))?;
        let db = req
            .state::<djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
        let user = djangors_auth::User::objects()
            .filter(djangors_orm::q!(id = user_id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| match e {
                djangors_orm::OrmError::NotFound { .. } => {
                    DjangorsError::Unauthorized("user not found".to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;
        if !user.is_active() {
            return Err(DjangorsError::Unauthorized("account inactive".to_string()));
        }
        Ok(JwtAuth(user))
    }
}

#[cfg(feature = "jwt")]
fn settings_secret(req: &Request) -> Result<&str, DjangorsError> {
    req.state::<djangors_core::DjangorsSettings>()
        .map(|settings| settings.secret_key.as_str())
        .ok_or_else(|| DjangorsError::Internal("settings state absent".to_string()))
}

/// A policy deciding whether a request may reach a ViewSet handler.
#[async_trait::async_trait]
pub trait Permission: Send + Sync + 'static {
    async fn has_permission(&self, req: &Request) -> bool;
}

/// Explicitly permits unauthenticated requests.
pub struct AllowAny;

#[async_trait::async_trait]
impl Permission for AllowAny {
    async fn has_permission(&self, _req: &Request) -> bool {
        true
    }
}

/// Requires a valid session or API token (and, when enabled, JWT).
pub struct IsAuthenticated;

#[async_trait::async_trait]
impl Permission for IsAuthenticated {
    async fn has_permission(&self, req: &Request) -> bool {
        if djangors_auth::Auth::<djangors_auth::User>::from_request(req)
            .await
            .is_ok()
            || TokenAuth::from_request(req).await.is_ok()
        {
            return true;
        }
        #[cfg(feature = "jwt")]
        {
            return JwtAuth::from_request(req).await.is_ok();
        }
        #[cfg(not(feature = "jwt"))]
        false
    }
}

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

/// Configuration options for a ViewSet endpoint (filtering and ordering allowlists).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewSetConfig {
    /// Allowlist of field names that can be filtered via `?field=value` query params.
    pub filterable_fields: &'static [&'static str],
    /// Allowlist of field names that can be ordered via `?ordering=field` / `?ordering=-field` query params.
    pub orderable_fields: &'static [&'static str],
}

fn parse_filter_value<M: Model>(field_name: &str, raw_val: &str) -> Option<Value> {
    let meta = M::meta();
    if let Some(field) = meta.fields.iter().find(|f| f.name == field_name) {
        match field.kind {
            FieldKind::Integer | FieldKind::BigInt => raw_val.parse::<i64>().ok().map(Value::I64),
            FieldKind::Float => raw_val.parse::<f64>().ok().map(Value::F64),
            FieldKind::Boolean => match raw_val {
                "true" | "1" => Some(Value::Bool(true)),
                "false" | "0" => Some(Value::Bool(false)),
                _ => None,
            },
            FieldKind::DateTime => {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw_val) {
                    Some(Value::DateTime(dt.with_timezone(&chrono::Utc)))
                } else if let Ok(naive) =
                    chrono::NaiveDateTime::parse_from_str(raw_val, "%Y-%m-%d %H:%M:%S")
                {
                    let dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                        naive,
                        chrono::Utc,
                    );
                    Some(Value::DateTime(dt))
                } else {
                    None
                }
            }
            FieldKind::Char
            | FieldKind::Text
            | FieldKind::Email
            | FieldKind::Url
            | FieldKind::Slug
            | FieldKind::Ip
            | FieldKind::Binary
            | FieldKind::Json
            | FieldKind::Decimal { .. }
            | FieldKind::Date
            | FieldKind::Time
            | FieldKind::Duration
            | FieldKind::Uuid => Some(Value::Text(raw_val.to_string())),
        }
    } else if meta.relations.iter().any(|r| r.field_name == field_name) {
        raw_val.parse::<i64>().ok().map(Value::I64)
    } else {
        None
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
    /// `GET /` — returns paginated list of records using default configuration.
    pub async fn list(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        Self::list_with_config(req, params, &ViewSetConfig::default()).await
    }

    /// `GET /` — returns paginated list of records with custom filtering and ordering allowlists.
    pub async fn list_with_config(
        req: Request,
        _params: PathParams,
        config: &ViewSetConfig,
    ) -> Result<Response, DjangorsError> {
        let db = req
            .state::<djangors_db::Database>()
            .ok_or_else(|| DjangorsError::Internal("Database connection not found".to_string()))?;

        let page: i64 = req
            .raw_query()
            .and_then(parse_page_param)
            .unwrap_or(1)
            .max(1);

        let mut qs = QuerySet::<M>::new();

        // 1. Parse ?field=value query params for allowlisted filterable_fields
        for &field in config.filterable_fields {
            if let Some(val_str) = req.query(field) {
                if let Some(value) = parse_filter_value::<M>(field, val_str) {
                    let cmp = UnresolvedExpr::And(vec![UnresolvedCompare { field, value }]);
                    qs = qs
                        .filter(cmp)
                        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                }
            }
        }

        // 2. Parse ?ordering=field / ?ordering=-field query params for allowlisted orderable_fields
        if let Some(ordering_param) = req.query("ordering") {
            for part in ordering_param.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let clean_field = part.strip_prefix('-').unwrap_or(part);
                if config.orderable_fields.contains(&clean_field) {
                    qs = qs
                        .order_by(part)
                        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                }
            }
        }

        let total_items = qs
            .count(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let paginator = Paginator::new(total_items, REST_PER_PAGE);
        let total_pages = paginator.total_pages();
        let offset = paginator.offset(page);

        let items = qs
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
pub fn viewset_routes<M>(router: Router, base_path: &str) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
{
    viewset_routes_with_config_and_permission::<M, IsAuthenticated>(
        router,
        base_path,
        ViewSetConfig::default(),
        IsAuthenticated,
    )
}

/// Mounts standard REST routes with an explicit permission policy.
///
/// [`viewset_routes`] uses [`IsAuthenticated`] by default. Pass [`AllowAny`] here only for
/// endpoints that are intentionally public.
pub fn viewset_routes_with_permission<M, P>(
    router: Router,
    base_path: &str,
    permission: P,
) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
    P: Permission,
{
    viewset_routes_with_config_and_permission::<M, P>(
        router,
        base_path,
        ViewSetConfig::default(),
        permission,
    )
}

/// Mounts standard REST routes with custom filtering and ordering configuration.
pub fn viewset_routes_with_config<M>(
    router: Router,
    base_path: &str,
    config: ViewSetConfig,
) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
{
    viewset_routes_with_config_and_permission::<M, IsAuthenticated>(
        router,
        base_path,
        config,
        IsAuthenticated,
    )
}

/// Mounts standard REST routes with custom configuration and an explicit permission policy.
pub fn viewset_routes_with_config_and_permission<M, P>(
    router: Router,
    base_path: &str,
    config: ViewSetConfig,
    permission: P,
) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
    P: Permission,
{
    let clean_base = base_path.trim_end_matches('/');
    let detail_path = format!("{clean_base}/{{pk:i64}}");
    let list_create_path = if clean_base.is_empty() {
        "/"
    } else {
        clean_base
    };
    let permission = Arc::new(permission);
    let config = Arc::new(config);

    let list_permission = permission.clone();
    let list_config = config.clone();
    let list_handler = move |req: Request, params: PathParams| {
        let perm = list_permission.clone();
        let cfg = list_config.clone();
        async move {
            if !perm.has_permission(&req).await {
                return Err(DjangorsError::Unauthorized("not authenticated".to_string()));
            }
            ViewSet::<M>::list_with_config(req, params, &cfg).await
        }
    };

    router
        .get(list_create_path, list_handler)
        .post(
            list_create_path,
            guarded(permission.clone(), ViewSet::<M>::create),
        )
        .get(
            &detail_path,
            guarded(permission.clone(), ViewSet::<M>::retrieve),
        )
        .put(
            &detail_path,
            guarded(permission.clone(), ViewSet::<M>::update),
        )
        .route(
            &detail_path,
            hyper::http::Method::PATCH,
            guarded(permission.clone(), ViewSet::<M>::update),
        )
        .delete(&detail_path, guarded(permission, ViewSet::<M>::destroy))
}

/// Generates an OpenAPI 3.1 JSON Schema component schema for model `M`.
pub fn openapi_schema_for<M: Model>() -> serde_json::Value {
    let meta = M::meta();
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();

    for field in meta.fields {
        let mut field_schema = match field.kind {
            FieldKind::Char => {
                let mut s = serde_json::json!({ "type": "string" });
                if let Some(max_len) = field.max_length {
                    s["maxLength"] = serde_json::json!(max_len);
                }
                s
            }
            FieldKind::Text | FieldKind::Slug => serde_json::json!({ "type": "string" }),
            FieldKind::Email => serde_json::json!({ "type": "string", "format": "email" }),
            FieldKind::Url => serde_json::json!({ "type": "string", "format": "uri" }),
            FieldKind::Ip => serde_json::json!({ "type": "string", "format": "ipv4" }),
            FieldKind::Binary => serde_json::json!({ "type": "string", "format": "binary" }),
            FieldKind::Uuid => serde_json::json!({ "type": "string", "format": "uuid" }),
            FieldKind::Date => serde_json::json!({ "type": "string", "format": "date" }),
            FieldKind::DateTime => serde_json::json!({ "type": "string", "format": "date-time" }),
            FieldKind::Time => serde_json::json!({ "type": "string", "format": "time" }),
            FieldKind::Duration => serde_json::json!({ "type": "string" }),
            FieldKind::Json => serde_json::json!({ "type": "object" }),
            FieldKind::Integer => serde_json::json!({ "type": "integer" }),
            FieldKind::BigInt => serde_json::json!({ "type": "integer", "format": "int64" }),
            FieldKind::Float => serde_json::json!({ "type": "number", "format": "float" }),
            FieldKind::Decimal {
                max_digits,
                decimal_places,
            } => {
                // Fixed-precision decimal fields serialize as JSON strings (matching Stripe/financial API conventions)
                // because fixed-precision decimals cannot be safely represented as IEEE-754 JSON numbers without precision loss.
                serde_json::json!({
                    "type": "string",
                    "description": format!("Decimal number with max {} digits and {} decimal places", max_digits, decimal_places)
                })
            }
            FieldKind::Boolean => serde_json::json!({ "type": "boolean" }),
        };

        if field.nullable {
            field_schema["nullable"] = serde_json::json!(true);
        } else if !field.auto {
            required.push(field.name);
        }

        if let Some(verbose) = field.verbose_name {
            if field_schema.get("description").is_none() {
                field_schema["description"] = serde_json::json!(verbose);
            }
        } else if let Some(help) = field.help_text {
            if field_schema.get("description").is_none() {
                field_schema["description"] = serde_json::json!(help);
            }
        }

        props.insert(field.name.to_string(), field_schema);
    }

    for rel in meta.relations {
        let rel_schema = serde_json::json!({
            "type": "integer",
            "format": "int64",
            "description": format!("Foreign key ID for relation {}", rel.field_name)
        });
        props.insert(rel.field_name.to_string(), rel_schema);
    }

    let mut schema = serde_json::json!({
        "type": "object",
        "title": meta.struct_name,
        "properties": props
    });

    if !required.is_empty() {
        schema["required"] = serde_json::json!(required);
    }

    schema
}

/// Builder for constructing OpenAPI 3.1 specifications from registered Djangors models.
#[derive(Debug, Clone, Default)]
pub struct OpenApiBuilder {
    title: String,
    version: String,
    schemas: HashMap<String, serde_json::Value>,
    paths: serde_json::Map<String, serde_json::Value>,
}

impl OpenApiBuilder {
    /// Creates a new `OpenApiBuilder` with specified title and API version.
    pub fn new(title: &str, version: &str) -> Self {
        Self {
            title: title.to_string(),
            version: version.to_string(),
            schemas: HashMap::new(),
            paths: serde_json::Map::new(),
        }
    }

    /// Registers model `M` mounted at `base_path` into the OpenAPI specification,
    /// generating its component schema and standard CRUD path operations.
    pub fn register<M: Model>(&mut self, base_path: &str) -> &mut Self {
        let meta = M::meta();
        let schema_name = meta.struct_name;

        if !self.schemas.contains_key(schema_name) {
            self.schemas
                .insert(schema_name.to_string(), openapi_schema_for::<M>());
        }

        let clean_base = base_path.trim_end_matches('/');
        let list_create_path = if clean_base.is_empty() {
            "/".to_string()
        } else {
            clean_base.to_string()
        };
        let detail_path = if list_create_path == "/" {
            "/{pk}".to_string()
        } else {
            format!("{list_create_path}/{{pk}}")
        };

        let schema_ref = format!("#/components/schemas/{schema_name}");

        let list_op = serde_json::json!({
            "summary": format!("List {} items", schema_name),
            "operationId": format!("list_{}", schema_name.to_lowercase()),
            "responses": {
                "200": {
                    "description": "Paginated list of items",
                    "content": {
                        "application/json": {
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "count": { "type": "integer" },
                                    "page": { "type": "integer" },
                                    "total_pages": { "type": "integer" },
                                    "results": {
                                        "type": "array",
                                        "items": { "$ref": schema_ref }
                                    }
                                },
                                "required": ["count", "page", "total_pages", "results"]
                            }
                        }
                    }
                }
            }
        });

        let create_op = serde_json::json!({
            "summary": format!("Create a {}", schema_name),
            "operationId": format!("create_{}", schema_name.to_lowercase()),
            "requestBody": {
                "required": true,
                "content": {
                    "application/json": {
                        "schema": { "$ref": schema_ref }
                    }
                }
            },
            "responses": {
                "201": {
                    "description": "Created item",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": schema_ref }
                        }
                    }
                },
                "422": { "description": "Validation error" }
            }
        });

        let mut list_create_item = self
            .paths
            .get(&list_create_path)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        list_create_item["get"] = list_op;
        list_create_item["post"] = create_op;
        self.paths.insert(list_create_path, list_create_item);

        let pk_param = serde_json::json!([{
            "name": "pk",
            "in": "path",
            "required": true,
            "schema": {
                "type": "integer",
                "format": "int64"
            },
            "description": "Primary key"
        }]);

        let retrieve_op = serde_json::json!({
            "summary": format!("Retrieve a {}", schema_name),
            "operationId": format!("retrieve_{}", schema_name.to_lowercase()),
            "parameters": pk_param,
            "responses": {
                "200": {
                    "description": "Item details",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": schema_ref }
                        }
                    }
                },
                "404": { "description": "Item not found" }
            }
        });

        let update_put_op = serde_json::json!({
            "summary": format!("Update a {}", schema_name),
            "operationId": format!("update_{}", schema_name.to_lowercase()),
            "parameters": pk_param,
            "requestBody": {
                "required": true,
                "content": {
                    "application/json": {
                        "schema": { "$ref": schema_ref }
                    }
                }
            },
            "responses": {
                "200": {
                    "description": "Updated item",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": schema_ref }
                        }
                    }
                },
                "404": { "description": "Item not found" },
                "422": { "description": "Validation error" }
            }
        });

        let update_patch_op = serde_json::json!({
            "summary": format!("Partial update a {}", schema_name),
            "operationId": format!("partial_update_{}", schema_name.to_lowercase()),
            "parameters": pk_param,
            "requestBody": {
                "required": true,
                "content": {
                    "application/json": {
                        "schema": { "$ref": schema_ref }
                    }
                }
            },
            "responses": {
                "200": {
                    "description": "Updated item",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": schema_ref }
                        }
                    }
                },
                "404": { "description": "Item not found" },
                "422": { "description": "Validation error" }
            }
        });

        let destroy_op = serde_json::json!({
            "summary": format!("Delete a {}", schema_name),
            "operationId": format!("destroy_{}", schema_name.to_lowercase()),
            "parameters": pk_param,
            "responses": {
                "204": { "description": "No content" },
                "404": { "description": "Item not found" }
            }
        });

        let mut detail_item = self
            .paths
            .get(&detail_path)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        detail_item["get"] = retrieve_op;
        detail_item["put"] = update_put_op;
        detail_item["patch"] = update_patch_op;
        detail_item["delete"] = destroy_op;
        self.paths.insert(detail_path, detail_item);

        self
    }

    /// Builds and returns the complete OpenAPI 3.1 specification JSON Value.
    pub fn build(&self) -> serde_json::Value {
        serde_json::json!({
            "openapi": "3.1.0",
            "info": {
                "title": self.title,
                "version": self.version
            },
            "paths": self.paths,
            "components": {
                "schemas": self.schemas
            }
        })
    }
}

fn guarded<P, F, Fut>(permission: Arc<P>, handler: F) -> impl djangors_core::Handler
where
    P: Permission,
    F: Fn(Request, PathParams) -> Fut + Copy + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Response, DjangorsError>> + Send + 'static,
{
    move |req: Request, params: PathParams| {
        let permission = permission.clone();
        async move {
            if !permission.has_permission(&req).await {
                return Err(DjangorsError::Unauthorized("not authenticated".to_string()));
            }
            handler(req, params).await
        }
    }
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

    #[test]
    fn token_keys_are_high_entropy_and_fit_the_model() {
        let first = generate_token_key();
        let second = generate_token_key();
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn viewset_routes_require_authentication_by_default() {
        let router = viewset_routes::<TestArticle>(Router::new(), "/api/articles");
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles"),
            HeaderMap::new(),
            Bytes::new(),
        );
        assert!(matches!(
            router.handle(req).await,
            Err(DjangorsError::Unauthorized(_))
        ));
    }

    #[tokio::test]
    async fn malformed_token_header_is_unauthorized() {
        let req = Request::new(
            Method::GET,
            Uri::from_static("/"),
            HeaderMap::new(),
            Bytes::new(),
        );
        assert!(matches!(
            TokenAuth::from_request(&req).await,
            Err(DjangorsError::Unauthorized(_))
        ));
    }

    #[cfg(feature = "jwt")]
    #[test]
    fn jwt_round_trip_and_tamper_rejection() {
        let token = encode_jwt(42, "test-secret");
        assert_eq!(decode_jwt(&token, "test-secret").unwrap(), 42);
        assert!(decode_jwt(&format!("{token}tampered"), "test-secret").is_err());
        assert!(decode_jwt(&token, "wrong-secret").is_err());
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

        let router = viewset_routes_with_permission::<TestArticle, _>(
            Router::new(),
            "/api/articles",
            AllowAny,
        )
        .with_state(db.clone());

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

        let router = viewset_routes_with_permission::<TestArticle, _>(
            Router::new(),
            "/api/articles",
            AllowAny,
        )
        .with_state(db.clone());

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

    #[derive(DeriveModel, Debug, Clone, sqlx::FromRow)]
    #[djangors(app = "rest_test", table_name = "rest_test_full_types")]
    pub struct TestFullTypes {
        #[djangors(primary_key, auto)]
        pub id: i64,
        #[djangors(max_length = 50)]
        pub title: String,
        pub description: String,
        pub view_count: i64,
        pub is_active: bool,
        pub created_at: chrono::DateTime<chrono::Utc>,
        pub price: String, // DecimalField equivalent test
        #[djangors(foreign_key(on_delete = "cascade"))]
        pub category: djangors_orm::ForeignKey<TestCategory>,
    }

    #[test]
    fn test_openapi_schema_generation() {
        let schema = openapi_schema_for::<TestArticle>();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["title"], "TestArticle");
        assert!(schema["properties"]["id"].is_object());
        assert_eq!(schema["properties"]["title"]["type"], "string");
        assert_eq!(schema["properties"]["view_count"]["type"], "integer");
        assert_eq!(schema["properties"]["is_published"]["type"], "boolean");
        assert_eq!(schema["properties"]["published_at"]["type"], "string");
        assert_eq!(schema["properties"]["published_at"]["format"], "date-time");
        assert_eq!(schema["properties"]["category"]["type"], "integer");
        assert_eq!(schema["properties"]["category"]["format"], "int64");

        let full_schema = openapi_schema_for::<TestFullTypes>();
        assert_eq!(full_schema["properties"]["title"]["maxLength"], 50);
    }

    #[test]
    fn test_openapi_builder_build() {
        let mut builder = OpenApiBuilder::new("Test API", "1.2.3");
        builder.register::<TestArticle>("/api/articles");

        let doc = builder.build();
        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["title"], "Test API");
        assert_eq!(doc["info"]["version"], "1.2.3");

        let paths = &doc["paths"];
        assert!(paths.get("/api/articles").is_some());
        assert!(paths["/api/articles"]["get"].is_object());
        assert!(paths["/api/articles"]["post"].is_object());

        assert!(paths.get("/api/articles/{pk}").is_some());
        assert!(paths["/api/articles/{pk}"]["get"].is_object());
        assert!(paths["/api/articles/{pk}"]["put"].is_object());
        assert!(paths["/api/articles/{pk}"]["patch"].is_object());
        assert!(paths["/api/articles/{pk}"]["delete"].is_object());

        let schemas = &doc["components"]["schemas"];
        assert!(schemas.get("TestArticle").is_some());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_viewset_filtering_and_ordering_end_to_end() {
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

        // Seed 3 articles:
        // A: title="Alpha", view_count=30, is_published=true
        // B: title="Beta", view_count=10, is_published=true
        // C: title="Gamma", view_count=20, is_published=false
        sqlx::query(
            "INSERT INTO rest_test_article (title, view_count, is_published, published_at, category) VALUES ($1, $2, $3, $4, $5)"
        )
        .bind("Alpha")
        .bind(30_i64)
        .bind(true)
        .bind(now)
        .bind(cat.id)
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO rest_test_article (title, view_count, is_published, published_at, category) VALUES ($1, $2, $3, $4, $5)"
        )
        .bind("Beta")
        .bind(10_i64)
        .bind(true)
        .bind(now)
        .bind(cat.id)
        .execute(db.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO rest_test_article (title, view_count, is_published, published_at, category) VALUES ($1, $2, $3, $4, $5)"
        )
        .bind("Gamma")
        .bind(20_i64)
        .bind(false)
        .bind(now)
        .bind(cat.id)
        .execute(db.pool())
        .await
        .unwrap();

        let viewset_config = ViewSetConfig {
            filterable_fields: &["is_published", "title"],
            orderable_fields: &["view_count", "title"],
        };

        let router = viewset_routes_with_config_and_permission::<TestArticle, _>(
            Router::new(),
            "/api/articles",
            viewset_config,
            AllowAny,
        )
        .with_state(db.clone());

        // 1. Filter by allowlisted boolean field `is_published=true` (should return 2 items: Alpha, Beta)
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles?is_published=true"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(json["count"], 2);

        // 2. Filter by allowlisted field `is_published=false` (should return 1 item: Gamma)
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles?is_published=false"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(json["count"], 1);
        assert_eq!(json["results"][0]["title"], "Gamma");

        // 3. Query param NOT in allowlist (`view_count=30`) must be ignored (returns all 3 items)
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles?view_count=30"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(json["count"], 3);

        // 4. Order ascending by allowlisted field `ordering=view_count` (Beta 10, Gamma 20, Alpha 30)
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles?ordering=view_count"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        let items = json["results"].as_array().unwrap();
        assert_eq!(items[0]["title"], "Beta");
        assert_eq!(items[1]["title"], "Gamma");
        assert_eq!(items[2]["title"], "Alpha");

        // 5. Order descending by allowlisted field `ordering=-view_count` (Alpha 30, Gamma 20, Beta 10)
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles?ordering=-view_count"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        let items = json["results"].as_array().unwrap();
        assert_eq!(items[0]["title"], "Alpha");
        assert_eq!(items[1]["title"], "Gamma");
        assert_eq!(items[2]["title"], "Beta");

        // 6. Non-allowlisted order parameter (`ordering=-published_at`) must be ignored
        let req = Request::new(
            Method::GET,
            Uri::from_static("/api/articles?ordering=-published_at"),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(djangors_core::state::AppState::new().insert(db.clone()));
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(json["count"], 3);
    }
}
