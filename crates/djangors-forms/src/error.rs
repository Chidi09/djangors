use std::collections::HashMap;
use std::error::Error;
use std::fmt;

/// A single field's validation error(s). A field can have more than one
/// error at once (e.g. "required" AND "max_length exceeded" if somehow
/// both conditions applied, though in practice most fields will short-
/// circuit on the first failure — validate() implementations decide this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError(pub Vec<String>);

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.join(", "))
    }
}

impl Error for FieldError {}

/// All validation errors for a form: per-field errors, plus a `__all__`
/// slot for cross-field errors raised by a form-level `clean()` hook
/// (Django's non-field-error convention).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormErrors {
    /// Validation errors mapped by field name.
    pub fields: HashMap<String, FieldError>,
    /// Cross-field or form-level validation errors.
    pub non_field: Vec<String>,
}

impl FormErrors {
    /// Creates a new, empty `FormErrors`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks if there are any validation errors.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.non_field.is_empty()
    }

    /// Adds a validation error to a specific field.
    pub fn add_field_error(&mut self, field: &str, message: impl Into<String>) {
        self.fields
            .entry(field.to_string())
            .or_insert_with(|| FieldError(Vec::new()))
            .0
            .push(message.into());
    }

    /// Adds a validation error that applies to the form as a whole.
    pub fn add_non_field_error(&mut self, message: impl Into<String>) {
        self.non_field.push(message.into());
    }
}
