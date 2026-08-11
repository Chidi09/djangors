#![deny(missing_docs)]
//! Forms and validation for the Djangors web framework.
//!
//! Provides the core traits, fields, widgets, renderers, formsets, and error structures.

/// Form error structures and per-field error maps.
pub mod error;
/// Form field types and field validation traits.
pub mod fields;
/// FormSets for managing multiple form instances in a single request.
pub mod formsets;
/// Form rendering engines (`as_div`, `as_table`, `as_p`) and `BoundField`.
pub mod renderers;
/// Form widgets and HTML rendering.
pub mod widgets;

pub use djangors_macros::Form;
pub use error::{FieldError, FormErrors};
pub use fields::{BooleanField, CharField, ChoiceField, EmailField, FormField, IntegerField};
pub use formsets::*;
pub use renderers::*;
pub use widgets::*;

#[cfg(test)]
mod tests;
