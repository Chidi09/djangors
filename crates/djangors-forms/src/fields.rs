use crate::error::FieldError;

/// Trait implemented by all form fields to parse and validate input data.
pub trait FormField {
    /// The resulting typed value after parsing and validation.
    type Value;

    /// Parse and validate a raw string form value (as it would arrive
    /// from an HTTP POST body — always Option<&str>, since a field might
    /// be entirely absent from the submitted data) into a typed value,
    /// or a field-level error.
    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError>;
}

/// A text field that validates string input.
///
/// # Examples
///
/// ```
/// # use djangors_forms::{CharField, FormField};
/// let field = CharField { max_length: Some(5), required: true };
///
/// // Required and present
/// assert_eq!(field.clean(Some("hello")), Ok("hello".to_string()));
///
/// // Required but empty or absent leads to an error
/// assert!(field.clean(None).is_err());
/// assert!(field.clean(Some("")).is_err());
///
/// // Max length constraint
/// assert!(field.clean(Some("too_long")).is_err());
///
/// // Whitespace is preserved (no automatic trimming)
/// let not_required = CharField { max_length: None, required: false };
/// assert_eq!(not_required.clean(Some("  spaces  ")), Ok("  spaces  ".to_string()));
/// ```
pub struct CharField {
    /// The maximum allowed length in characters.
    pub max_length: Option<usize>,
    /// Whether the field is required to have a non-empty value.
    pub required: bool,
}

impl FormField for CharField {
    type Value = String;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        match raw {
            None | Some("") => {
                if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(String::new())
                }
            }
            Some(s) => {
                if let Some(limit) = self.max_length {
                    if s.chars().count() > limit {
                        return Err(FieldError(vec![format!(
                            "Ensure this value has at most {} characters.",
                            limit
                        )]));
                    }
                }
                Ok(s.to_string())
            }
        }
    }
}

/// An integer field that validates signed 64-bit integer values.
///
/// # Examples
///
/// ```
/// # use djangors_forms::{IntegerField, FormField};
/// let field = IntegerField { min: Some(1), max: Some(10), required: true };
///
/// // Valid integer in range
/// assert_eq!(field.clean(Some("5")), Ok(Some(5)));
///
/// // Valid integer at boundaries (boundary-inclusive)
/// assert_eq!(field.clean(Some("1")), Ok(Some(1)));
/// assert_eq!(field.clean(Some("10")), Ok(Some(10)));
///
/// // Below minimum or above maximum
/// assert!(field.clean(Some("0")).is_err());
/// assert!(field.clean(Some("11")).is_err());
///
/// // Invalid integer format
/// assert!(field.clean(Some("not-a-number")).is_err());
///
/// // Absent/optional handling
/// let optional = IntegerField { min: None, max: None, required: false };
/// assert_eq!(optional.clean(None), Ok(None));
/// assert_eq!(optional.clean(Some("")), Ok(None));
/// ```
pub struct IntegerField {
    /// The minimum allowed value (inclusive).
    pub min: Option<i64>,
    /// The maximum allowed value (inclusive).
    pub max: Option<i64>,
    /// Whether the field is required.
    pub required: bool,
}

impl FormField for IntegerField {
    type Value = Option<i64>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        match raw {
            None | Some("") => {
                if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                }
            }
            Some(s) => {
                let val = s
                    .parse::<i64>()
                    .map_err(|_| FieldError(vec!["Enter a whole number.".to_string()]))?;

                if let Some(minimum) = self.min {
                    if val < minimum {
                        return Err(FieldError(vec![format!(
                            "Ensure this value is greater than or equal to {}.",
                            minimum
                        )]));
                    }
                }

                if let Some(maximum) = self.max {
                    if val > maximum {
                        return Err(FieldError(vec![format!(
                            "Ensure this value is less than or equal to {}.",
                            maximum
                        )]));
                    }
                }

                Ok(Some(val))
            }
        }
    }
}

/// A boolean field that validates boolean states (typically checkbox inputs).
///
/// # Django-Specific Semantics
///
/// HTML checkboxes are omitted from HTTP POST submissions if unchecked.
/// Thus, `clean(None)` represents `false`.
///
/// If `required` is true, the field validates that the value is `true` (e.g. for "I agree to terms" checkboxes).
///
/// # Examples
///
/// ```
/// # use djangors_forms::{BooleanField, FormField};
/// let field = BooleanField { required: true };
///
/// // Truthy values are accepted
/// assert_eq!(field.clean(Some("on")), Ok(true));
/// assert_eq!(field.clean(Some("true")), Ok(true));
/// assert_eq!(field.clean(Some("1")), Ok(true));
///
/// // Falsy/absent values error if required: true
/// assert!(field.clean(None).is_err());
/// assert!(field.clean(Some("false")).is_err());
///
/// // If not required, false/absent value is accepted and returns false
/// let optional = BooleanField { required: false };
/// assert_eq!(optional.clean(None), Ok(false));
/// assert_eq!(optional.clean(Some("false")), Ok(false));
/// ```
pub struct BooleanField {
    /// Whether the field value must clean to `true`.
    pub required: bool,
}

impl FormField for BooleanField {
    type Value = bool;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let truthy = matches!(raw, Some("on" | "true" | "1"));

        if self.required && !truthy {
            Err(FieldError(vec!["This field is required.".to_string()]))
        } else {
            Ok(truthy)
        }
    }
}

/// An email field that validates a simple email format.
///
/// # Examples
///
/// ```
/// # use djangors_forms::{EmailField, FormField};
/// let field = EmailField { required: true };
///
/// // Valid emails
/// assert_eq!(field.clean(Some("user@example.com")), Ok("user@example.com".to_string()));
/// assert_eq!(field.clean(Some("a@b.c")), Ok("a@b.c".to_string()));
///
/// // Invalid email formats
/// assert!(field.clean(Some("no-at-sign")).is_err());
/// assert!(field.clean(Some("@domain.com")).is_err());
/// assert!(field.clean(Some("user@.com")).is_err());
/// assert!(field.clean(Some("user@domain.")).is_err());
/// assert!(field.clean(Some("user@domain..com")).is_err());
/// ```
pub struct EmailField {
    /// Whether the field is required.
    pub required: bool,
}

impl FormField for EmailField {
    type Value = String;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        match raw {
            None | Some("") => {
                if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(String::new())
                }
            }
            Some(s) => {
                let parts: Vec<&str> = s.split('@').collect();
                if parts.len() != 2 {
                    return Err(FieldError(vec!["Enter a valid email address.".to_string()]));
                }
                let local = parts[0];
                let domain = parts[1];

                if local.is_empty()
                    || domain.is_empty()
                    || !domain.contains('.')
                    || domain.starts_with('.')
                    || domain.ends_with('.')
                    || domain.contains("..")
                {
                    return Err(FieldError(vec!["Enter a valid email address.".to_string()]));
                }

                Ok(s.to_string())
            }
        }
    }
}
