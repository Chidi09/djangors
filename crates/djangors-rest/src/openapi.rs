//! OpenAPI 3.1 schema generation from model metadata.

use std::collections::HashMap;

use djangors_orm::meta::{FieldKind, Model};

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
            FieldKind::Text | FieldKind::Slug | FieldKind::FileField => {
                serde_json::json!({ "type": "string" })
            }
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
