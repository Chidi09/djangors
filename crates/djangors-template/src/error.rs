use std::path::PathBuf;
use thiserror::Error;

/// Error type for djangors-template.
#[derive(Debug, Error)]
pub enum TemplateError {
    /// The template file was not found in any of the search directories.
    #[error("Template '{name}' not found. Searched in directories: {searched:?}")]
    NotFound {
        /// The name of the missing template.
        name: String,
        /// The directories searched.
        searched: Vec<PathBuf>,
    },

    /// An I/O error occurred while reading a template.
    #[error("I/O error reading template: {0}")]
    Io(#[from] std::io::Error),

    /// A template parsing or rendering error from MiniJinja.
    #[error("MiniJinja error: {0}")]
    MiniJinja(#[from] minijinja::Error),
}
