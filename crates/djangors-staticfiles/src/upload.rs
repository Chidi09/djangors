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
mod tests {
    use super::*;
    use bytes::Bytes;
    use djangors_core::extract::UploadedFile;
    use tempfile::TempDir;

    fn make_test_file() -> UploadedFile {
        UploadedFile {
            field_name: "document".into(),
            file_name: "hello.txt".into(),
            content_type: Some("text/plain".into()),
            bytes: Bytes::from_static(b"Hello, world!"),
        }
    }

    #[tokio::test]
    async fn test_save_upload_writes_and_can_be_read_back() {
        let tmp = TempDir::new().unwrap();
        let storage = crate::storage::LocalDiskStorage::new(tmp.path(), "/uploads/");

        let file = make_test_file();
        let stored_path = save_upload(&file, &storage, "testdir").await.unwrap();

        // The returned path should be usable to open the bytes back
        let read_back = storage.open(&stored_path).await.unwrap();
        assert_eq!(read_back, b"Hello, world!");
    }

    #[tokio::test]
    async fn test_storage_path_differs_from_client_filename() {
        let tmp = TempDir::new().unwrap();
        let storage = crate::storage::LocalDiskStorage::new(tmp.path(), "/uploads/");

        let file = make_test_file();
        let stored_path = save_upload(&file, &storage, "").await.unwrap();

        // The stored path must NOT equal the raw client-supplied filename
        assert_ne!(
            stored_path, "hello.txt",
            "stored path must not be the raw client filename verbatim"
        );

        // Verify it contains the original stem for readability
        assert!(
            stored_path.contains("hello"),
            "stored path should contain the original file stem: {stored_path}"
        );
        // Verify it contains a hash component
        assert!(
            stored_path.contains('_'),
            "stored path should contain a separator: {stored_path}"
        );
    }

    #[tokio::test]
    async fn test_generate_storage_path_collision_avoidance() {
        let path1 = generate_storage_path("photo.jpg", b"content A");
        let path2 = generate_storage_path("photo.jpg", b"content B");

        // Different content → different paths
        assert_ne!(path1, path2, "different content must yield different paths");

        // Both preserve the extension
        assert!(path1.ends_with(".jpg"), "{path1} should end with .jpg");
        assert!(path2.ends_with(".jpg"), "{path2} should end with .jpg");
    }
}
