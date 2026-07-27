#![deny(missing_docs)]
//! Static file collection, hashing, and serving for Djangors.

/// Static file collection and manifest creation.
pub mod collect;
/// Static file processing and loading errors.
pub mod error;
/// Static file serving handler logic.
pub mod serve;
/// Pluggable static-file storage backends.
pub mod storage;
/// Helpers for writing uploaded files through a storage backend.
pub mod upload;

pub use collect::Manifest;
pub use error::StaticFilesError;
pub use serve::{StaticFiles, StaticFilesHandler};
pub use storage::{LocalDiskStorage, S3Storage, Storage};
