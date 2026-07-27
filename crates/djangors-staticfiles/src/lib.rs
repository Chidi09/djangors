#![deny(missing_docs)]
//! Static file collection, hashing, and serving for Djangors.

/// Static file collection and manifest creation.
pub mod collect;
/// Static file processing and loading errors.
pub mod error;
/// Static file serving handler logic.
pub mod serve;

pub use collect::Manifest;
pub use error::StaticFilesError;
pub use serve::{StaticFiles, StaticFilesHandler};
