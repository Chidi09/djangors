//! Template engine integration for Djangors.
//!
//! Provides `TemplateEngine` and `render` shortcut helper for rendering templates.

pub mod engine;
pub mod error;
pub mod filters;
pub mod render;

pub use engine::TemplateEngine;
pub use error::TemplateError;
pub use render::render;
