//! Field-level validation errors, and the validator hook serializers run.
//!
//! DRF's serializers fail with a `{field: [messages]}` map that clients can
//! render next to the offending input. Before this module, `deserialize`
//! returned a bare `HashMap<String, String>` that ViewSets collapsed into a
//! single `DjangorsError::BadRequest` string, so a client could not tell which
//! field was wrong without parsing prose.
//!
//! [`ValidationErrors`] keeps the structure all the way to the wire: it renders
//! as a [`DjangorsError::Api`] whose `details` is the field map.

use std::collections::BTreeMap;

use djangors_core::error::DjangorsError;
use hyper::StatusCode;

/// Errors keyed by field name, plus errors that belong to the object as a whole.
///
/// Field order is stable (sorted) so responses and tests are deterministic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationErrors {
    fields: BTreeMap<String, Vec<String>>,
    non_field: Vec<String>,
}

/// The `code` carried by the [`DjangorsError::Api`] that [`ValidationErrors`]
/// renders into. Clients can branch on this instead of matching a status alone.
pub const VALIDATION_ERROR_CODE: &str = "validation_error";

impl ValidationErrors {
    /// Create an empty error set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a message against a named field.
    pub fn add(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.fields
            .entry(field.into())
            .or_default()
            .push(message.into());
    }

    /// Record a message that belongs to the object rather than one field —
    /// DRF's `non_field_errors`. Use for cross-field rules such as
    /// "end must be after start".
    pub fn add_non_field(&mut self, message: impl Into<String>) {
        self.non_field.push(message.into());
    }

    /// Merge another set into this one, preserving every message.
    pub fn merge(&mut self, other: ValidationErrors) {
        for (field, messages) in other.fields {
            self.fields.entry(field).or_default().extend(messages);
        }
        self.non_field.extend(other.non_field);
    }

    /// Whether any error has been recorded.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.non_field.is_empty()
    }

    /// Messages recorded for one field, if any.
    pub fn get(&self, field: &str) -> Option<&[String]> {
        self.fields.get(field).map(|v| v.as_slice())
    }

    /// Whether a message was recorded for the given field.
    pub fn contains_key(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }

    /// Every field that has at least one message.
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.fields.keys().map(|k| k.as_str())
    }

    /// Object-level messages.
    pub fn non_field_errors(&self) -> &[String] {
        &self.non_field
    }

    /// `Ok(())` when empty, otherwise `Err(self)`.
    ///
    /// Lets a validator accumulate every problem and surface them together,
    /// rather than failing on the first one.
    pub fn into_result(self) -> Result<(), ValidationErrors> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }

    /// The error map as JSON: `{"field": ["msg", ...]}`, with object-level
    /// messages under `non_field_errors`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (field, messages) in &self.fields {
            map.insert(field.clone(), serde_json::json!(messages));
        }
        if !self.non_field.is_empty() {
            map.insert(
                "non_field_errors".to_string(),
                serde_json::json!(self.non_field),
            );
        }
        serde_json::Value::Object(map)
    }
}

impl std::fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts: Vec<String> = self
            .fields
            .iter()
            .map(|(field, messages)| format!("{field}: {}", messages.join("; ")))
            .collect();
        parts.extend(self.non_field.iter().cloned());
        write!(f, "{}", parts.join(", "))
    }
}

impl std::error::Error for ValidationErrors {}

impl From<ValidationErrors> for DjangorsError {
    /// Renders as a `422 Unprocessable Entity` [`DjangorsError::Api`] carrying
    /// the whole field map in `details`, so the structure survives to the
    /// client instead of being flattened into a message string.
    fn from(errors: ValidationErrors) -> Self {
        let details = errors.to_json();
        DjangorsError::api(
            StatusCode::UNPROCESSABLE_ENTITY,
            VALIDATION_ERROR_CODE,
            "Validation failed",
        )
        .with_details(details)
    }
}

impl From<std::collections::HashMap<String, String>> for ValidationErrors {
    /// Adapts the flat map that [`crate::deserialize`] produces.
    fn from(map: std::collections::HashMap<String, String>) -> Self {
        let mut errors = ValidationErrors::new();
        for (field, message) in map {
            errors.add(field, message);
        }
        errors
    }
}

/// An object-level validation rule, run after individual fields have been
/// parsed and coerced.
///
/// This is the hook the review found missing: previously a ViewSet could reject
/// a badly *typed* field but had nowhere to express a business rule.
pub trait Validator<T>: Send + Sync + 'static {
    /// Inspect the candidate value and record any problems.
    fn validate(&self, value: &T, errors: &mut ValidationErrors);
}

impl<T, F> Validator<T> for F
where
    F: Fn(&T, &mut ValidationErrors) + Send + Sync + 'static,
{
    fn validate(&self, value: &T, errors: &mut ValidationErrors) {
        self(value, errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_and_renders_field_and_non_field_errors() {
        let mut errors = ValidationErrors::new();
        errors.add("title", "must not be blank");
        errors.add("title", "must be under 200 characters");
        errors.add("view_count", "must be positive");
        errors.add_non_field("end must be after start");

        assert!(!errors.is_empty());
        assert_eq!(errors.get("title").unwrap().len(), 2);
        assert_eq!(
            errors.to_json(),
            serde_json::json!({
                "title": ["must not be blank", "must be under 200 characters"],
                "view_count": ["must be positive"],
                "non_field_errors": ["end must be after start"],
            })
        );
    }

    #[test]
    fn empty_errors_convert_to_ok() {
        assert!(ValidationErrors::new().into_result().is_ok());
    }

    #[test]
    fn renders_as_422_api_error_carrying_the_field_map() {
        let mut errors = ValidationErrors::new();
        errors.add("title", "must not be blank");

        let err: DjangorsError = errors.into();
        assert_eq!(err.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(err.code(), VALIDATION_ERROR_CODE);
        assert_eq!(
            err.details().unwrap(),
            &serde_json::json!({"title": ["must not be blank"]})
        );
    }

    #[test]
    fn merge_preserves_messages_from_both_sides() {
        let mut a = ValidationErrors::new();
        a.add("title", "too short");
        let mut b = ValidationErrors::new();
        b.add("title", "banned word");
        b.add("slug", "already taken");

        a.merge(b);
        assert_eq!(a.get("title").unwrap().len(), 2);
        assert!(a.contains_key("slug"));
    }
}
