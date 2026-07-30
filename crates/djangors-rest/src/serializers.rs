//! Model serialization for REST endpoints.

use std::collections::HashMap;

use djangors_orm::expr::Value;
use djangors_orm::meta::{FieldKind, Model};

use crate::*;

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
    deserialize_inner::<M>(json, false)
}

/// Parses only the fields the body actually supplied.
///
/// [`deserialize`] is whole-object: it walks every column, defaults the absent
/// ones, and reports the rest as required. That is right for a full write and
/// wrong for `PATCH`, where an omitted field means "leave it alone" — so
/// partial writes skip absent keys entirely instead of defaulting them.
///
/// An explicit `null` is still honoured (and still rejected on a non-nullable
/// column); only *absent* keys are skipped.
pub fn deserialize_partial<M: Model>(
    json: &serde_json::Value,
) -> Result<Vec<(&'static str, Value)>, HashMap<String, String>> {
    deserialize_inner::<M>(json, true)
}

fn deserialize_inner<M: Model>(
    json: &serde_json::Value,
    partial: bool,
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

        if partial && val_opt.is_none() {
            continue;
        }

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
                FieldKind::FileField => {
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
        if partial && val_opt.is_none() {
            continue;
        }
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

/// Controls which fields cross the wire, in each direction.
///
/// This is the read/write split the review found missing: previously
/// [`serialize`] emitted every column and [`deserialize`] accepted every
/// column, so a password hash or an internal flag could only be hidden by
/// post-processing the JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldSet {
    /// When non-empty, only these fields are emitted or accepted. Empty means
    /// "every model field", subject to the exclusions below.
    pub only: Vec<String>,
    /// Fields never emitted and never accepted, in either direction.
    pub exclude: Vec<String>,
    /// Fields emitted on read but silently dropped from writes — server-owned
    /// values such as `id` or `created_at`.
    pub read_only: Vec<String>,
    /// Fields accepted on write but never emitted — secrets such as `password`.
    pub write_only: Vec<String>,
}

impl FieldSet {
    /// An unrestricted field set: every model field, both directions.
    pub fn all() -> Self {
        Self::default()
    }

    /// Restrict to exactly these fields.
    pub fn only(fields: &[&str]) -> Self {
        Self {
            only: fields.iter().map(|f| f.to_string()).collect(),
            ..Self::default()
        }
    }

    /// Exclude these fields entirely.
    pub fn excluding(mut self, fields: &[&str]) -> Self {
        self.exclude = fields.iter().map(|f| f.to_string()).collect();
        self
    }

    /// Mark these fields read-only: emitted, never accepted.
    pub fn read_only(mut self, fields: &[&str]) -> Self {
        self.read_only = fields.iter().map(|f| f.to_string()).collect();
        self
    }

    /// Mark these fields write-only: accepted, never emitted.
    pub fn write_only(mut self, fields: &[&str]) -> Self {
        self.write_only = fields.iter().map(|f| f.to_string()).collect();
        self
    }

    fn in_scope(&self, field: &str) -> bool {
        if self.exclude.iter().any(|f| f == field) {
            return false;
        }
        self.only.is_empty() || self.only.iter().any(|f| f == field)
    }

    /// Whether the field appears in serialized output.
    pub fn is_readable(&self, field: &str) -> bool {
        self.in_scope(field) && !self.write_only.iter().any(|f| f == field)
    }

    /// Whether the field is accepted from client input.
    pub fn is_writable(&self, field: &str) -> bool {
        self.in_scope(field) && !self.read_only.iter().any(|f| f == field)
    }
}

/// The column values a serializer produces from a request body: pairs of model
/// field name and coerced value, ready for `insert_raw` / `update`.
pub type FieldValues = Vec<(&'static str, Value)>;

/// Converts between a model and its wire representation, with validation.
///
/// This is the seam DRF has and Djangors lacked. A ViewSet no longer hardcodes
/// `serialize`/`deserialize`; it asks a serializer, so a project can control
/// field exposure, coerce values, and enforce business rules in one place.
pub trait Serializer<M: Model>: Send + Sync + 'static {
    /// Render an instance for the response body.
    fn to_representation(&self, instance: &M) -> serde_json::Value;

    /// Parse and validate a request body into column values ready for
    /// `insert_raw` / `update`.
    ///
    /// Implementations should accumulate every problem into
    /// [`ValidationErrors`] rather than returning on the first one.
    fn to_internal_value(
        &self,
        data: &serde_json::Value,
        partial: bool,
    ) -> Result<Vec<(&'static str, Value)>, ValidationErrors>;

    /// Object-level rules, run after fields parse successfully.
    ///
    /// The default accepts everything. Override to express cross-field
    /// constraints; record problems on `errors` instead of returning early so
    /// the client sees them all at once.
    fn validate(&self, _values: &[(&'static str, Value)], _errors: &mut ValidationErrors) {}

    /// Render a collection.
    fn to_representation_many(&self, instances: &[M]) -> Vec<serde_json::Value> {
        instances
            .iter()
            .map(|item| self.to_representation(item))
            .collect()
    }

    /// Parse, then apply [`Serializer::validate`]. ViewSets call this rather
    /// than `to_internal_value` directly, so object-level rules are never
    /// accidentally skipped.
    fn parse(
        &self,
        data: &serde_json::Value,
        partial: bool,
    ) -> Result<Vec<(&'static str, Value)>, ValidationErrors> {
        let values = self.to_internal_value(data, partial)?;
        let mut errors = ValidationErrors::new();
        self.validate(&values, &mut errors);
        if errors.is_empty() {
            Ok(values)
        } else {
            Err(errors)
        }
    }
}

/// The default [`Serializer`]: derives its behaviour from [`Model`] metadata,
/// narrowed by a [`FieldSet`].
///
/// Equivalent to DRF's `ModelSerializer`. Constructing one with
/// [`FieldSet::all`] reproduces the historical [`serialize`]/[`deserialize`]
/// behaviour exactly.
pub struct ModelSerializer<M: Model> {
    fields: FieldSet,
    validators: Vec<Box<dyn Validator<FieldValues>>>,
    _marker: std::marker::PhantomData<M>,
}

impl<M: Model> Default for ModelSerializer<M> {
    fn default() -> Self {
        Self::new(FieldSet::all())
    }
}

impl<M: Model> ModelSerializer<M> {
    /// Build a serializer restricted to `fields`.
    pub fn new(fields: FieldSet) -> Self {
        Self {
            fields,
            validators: Vec::new(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Attach an object-level rule. Rules run in registration order, and every
    /// rule runs even if an earlier one failed, so the client sees the full set.
    pub fn with_validator<V>(mut self, validator: V) -> Self
    where
        V: Validator<FieldValues>,
    {
        self.validators.push(Box::new(validator));
        self
    }

    /// The field set in force.
    pub fn field_set(&self) -> &FieldSet {
        &self.fields
    }
}

impl<M: Model + Send + Sync + 'static> Serializer<M> for ModelSerializer<M> {
    fn to_representation(&self, instance: &M) -> serde_json::Value {
        let full = serialize(instance);
        let serde_json::Value::Object(map) = full else {
            return serde_json::Value::Object(serde_json::Map::new());
        };
        serde_json::Value::Object(
            map.into_iter()
                .filter(|(name, _)| self.fields.is_readable(name))
                .collect(),
        )
    }

    fn to_internal_value(
        &self,
        data: &serde_json::Value,
        partial: bool,
    ) -> Result<Vec<(&'static str, Value)>, ValidationErrors> {
        // Reject writes to read-only/excluded fields loudly rather than
        // silently dropping them: a client that thinks it set `id` should be
        // told it did not.
        let mut errors = ValidationErrors::new();
        if let serde_json::Value::Object(map) = data {
            for name in map.keys() {
                let known = M::meta().fields.iter().any(|f| f.name == name)
                    || M::meta().relations.iter().any(|r| r.field_name == name);
                if known && !self.fields.is_writable(name) {
                    errors.add(name.clone(), "field is read-only");
                }
            }
        }

        // A partial (PATCH) body legitimately omits fields; a full (PUT/POST)
        // body must supply every writable, non-auto column.
        //
        // This reads the raw body rather than `deserialize`'s output on
        // purpose. `deserialize` defaults a missing non-nullable boolean to
        // `false` — correct for an HTML checkbox, wrong for a JSON API, where
        // `POST {}` would silently set the flag instead of reporting it
        // missing. Checking the body directly closes that hole, and runs
        // independently of whether `deserialize` also failed, so the client
        // sees every problem at once.
        if !partial {
            let supplied = |name: &str| data.get(name).is_some_and(|value| !value.is_null());
            for field in M::meta().fields {
                if field.auto
                    || field.primary_key
                    || field.nullable
                    || !self.fields.is_writable(field.name)
                {
                    continue;
                }
                if !supplied(field.name) {
                    errors.add(field.name, "this field is required");
                }
            }
            for relation in M::meta().relations {
                if !self.fields.is_writable(relation.field_name) {
                    continue;
                }
                if !supplied(relation.field_name) {
                    errors.add(relation.field_name, "this field is required");
                }
            }
        }

        let parsed = if partial {
            deserialize_partial::<M>(data)
        } else {
            deserialize::<M>(data)
        };

        let mut values = match parsed {
            Ok(values) => values,
            Err(parse_errors) => {
                // Merge rather than replace: a body can be both missing one
                // field and malformed in another, and the client should see
                // both at once.
                errors.merge(ValidationErrors::from(parse_errors));
                return Err(errors);
            }
        };

        errors.into_result()?;

        values.retain(|(name, _)| self.fields.is_writable(name));
        Ok(values)
    }

    fn validate(&self, values: &[(&'static str, Value)], errors: &mut ValidationErrors) {
        let owned = values.to_vec();
        for validator in &self.validators {
            validator.validate(&owned, errors);
        }
    }
}
