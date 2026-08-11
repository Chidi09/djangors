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
