#![deny(missing_docs)]
//! Template engine integration for Djangors.
//!
//! Provides `TemplateEngine` and `render` shortcut helper for rendering templates.

/// MiniJinja template engine integration and directory loading logic.
pub mod engine;
/// Template loading and rendering error definitions.
pub mod error;
/// Template filters matching standard Django built-in filters.
pub mod filters;
/// Convenient HTTP response rendering shortcuts.
pub mod render;

pub use engine::TemplateEngine;
pub use error::TemplateError;
pub use render::render;
