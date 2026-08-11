//! Helpers for writing uploaded files through a [`Storage`] backend.
//!
//! Provides [`save_upload`] which takes a parsed file part from
//! [`djangors_core::extract::UploadedFile`], a [`Storage`] backend, and
//! a destination directory, then saves the file under a collision-avoiding
//! name and returns the stored path.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::StaticFilesError;
use crate::storage::Storage;

/// Generates a collision-avoiding storage path from a client-supplied filename
/// and the file content.
///
/// The returned path incorporates the original filename (for human readability)
/// and a hex digest of the content (for uniqueness / deduplication), ensuring
/// the stored path never equals the raw client-supplied filename.
fn generate_storage_path(file_name: &str, bytes: &[u8]) -> String {
    let input = Path::new(file_name);
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("upload");

    let safe_stem: String = stem
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect();
    let safe_stem = if safe_stem.is_empty() {
        "upload".to_string()
    } else {
        safe_stem
    };

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hash = format!("{:x}", hasher.finalize());
    let short_hash = &hash[..12];

    format!("{}_{}{}", safe_stem, short_hash, ext)
}

/// Writes an uploaded file through a [`Storage`] backend.
///
/// Takes a parsed file part, a storage backend, and an optional destination
/// directory prefix. The file is saved under a collision-avoiding name
/// (see [`generate_storage_path`]) and the stored path string is returned.
///
/// This is the value a `FileField` model field should be set to.
pub async fn save_upload(
    file: &djangors_core::extract::UploadedFile,
    storage: &dyn Storage,
    dest_dir: &str,
) -> Result<String, StaticFilesError> {
    let stored_name = generate_storage_path(&file.file_name, &file.bytes);
    let full_path = if dest_dir.is_empty() {
        stored_name
    } else {
        format!("{}/{}", dest_dir.trim_end_matches('/'), stored_name)
    };
    storage.save(&full_path, file.bytes.to_vec()).await
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
