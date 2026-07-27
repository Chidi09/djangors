use bytes::Bytes;
use djangors_core::{DjangorsError, PathParams, Request, Router};
use djangors_staticfiles::{LocalDiskStorage, Manifest, S3Storage, StaticFiles, Storage};

async fn round_trip(storage: &impl Storage, path: &str) {
    let bytes = b"storage abstraction".to_vec();
    storage.save(path, bytes.clone()).await.unwrap();
    assert!(storage.exists(path).await.unwrap());
    assert_eq!(storage.open(path).await.unwrap(), bytes);
    storage.delete(path).await.unwrap();
    assert!(!storage.exists(path).await.unwrap());
}

#[tokio::test]
async fn storage_backends_share_the_same_trait_contract() {
    let root = tempfile::tempdir().unwrap();
    let local = LocalDiskStorage::new(root.path(), "");
    round_trip(&local, "trait-local.txt").await;

    let s3 = S3Storage::new(
        "djangors-test",
        "us-east-1",
        Some("http://localhost:19000"),
        "test",
        "testtest",
        "http://localhost:19000/djangors-test",
    )
    .unwrap();
    if s3.create_bucket().await.is_err() {
        return;
    }
    round_trip(&s3, "trait-s3.txt").await;
}

#[tokio::test]
async fn s3_storage_round_trips_against_minio() {
    let s3 = S3Storage::new(
        "djangors-test",
        "us-east-1",
        Some("http://localhost:19000"),
        "test",
        "testtest",
        "http://localhost:19000/djangors-test",
    )
    .unwrap();
    if s3.create_bucket().await.is_err() {
        return;
    }
    let content = b"real minio wire round trip".to_vec();
    s3.save("integration/object.bin", content.clone())
        .await
        .unwrap();
    assert!(s3.exists("integration/object.bin").await.unwrap());
    assert_eq!(s3.open("integration/object.bin").await.unwrap(), content);
    s3.delete("integration/object.bin").await.unwrap();
    assert!(!s3.exists("integration/object.bin").await.unwrap());
}
use hyper::http::{HeaderMap, Method, Uri};
use hyper::StatusCode;
use std::fs;
use tempfile::TempDir;

fn make_request(method: Method, path: &str) -> Request {
    let uri = Uri::try_from(path).expect("valid URI");
    Request::new(method, uri, HeaderMap::new(), Bytes::new())
}

#[tokio::test]
async fn test_dev_serving_basic() {
    let tmp = TempDir::new().unwrap();
    let css_file = tmp.path().join("style.css");
    fs::write(&css_file, b"body { color: red; }").unwrap();

    let js_file = tmp.path().join("script.js");
    fs::write(&js_file, b"console.log('hello');").unwrap();

    let sf = StaticFiles::new(vec![tmp.path().to_path_buf()]);

    // Test CSS serving
    let req = make_request(Method::GET, "/static/style.css");
    let resp = sf.serve(req, PathParams::default()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/css; charset=utf-8"
    );
    assert_eq!(resp.body(), b"body { color: red; }");

    // Test JS serving
    let req2 = make_request(Method::GET, "/static/script.js");
    let resp2 = sf.serve(req2, PathParams::default()).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    assert_eq!(
        resp2.headers().get("content-type").unwrap(),
        "application/javascript; charset=utf-8"
    );
    assert_eq!(resp2.body(), b"console.log('hello');");
}

#[tokio::test]
async fn test_path_traversal_protection() {
    let root = TempDir::new().unwrap();
    let source_dir = root.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();

    let secret_file = root.path().join("secret.txt");
    fs::write(&secret_file, b"classified information").unwrap();

    let sf = StaticFiles::new(vec![source_dir.clone()]);

    let req = make_request(Method::GET, "/static/../secret.txt");
    let res = sf.serve(req, PathParams::default()).await;

    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), DjangorsError::NotFound));
}

#[tokio::test]
async fn test_nonexistent_file() {
    let tmp = TempDir::new().unwrap();
    let sf = StaticFiles::new(vec![tmp.path().to_path_buf()]);

    let req = make_request(Method::GET, "/static/ghost.txt");
    let res = sf.serve(req, PathParams::default()).await;
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), DjangorsError::NotFound));
}

#[tokio::test]
async fn test_directory_precedence() {
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();

    fs::write(tmp1.path().join("file.txt"), b"first wins").unwrap();
    fs::write(tmp2.path().join("file.txt"), b"second wins").unwrap();

    let sf = StaticFiles::new(vec![tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);

    let req = make_request(Method::GET, "/static/file.txt");
    let resp = sf.serve(req, PathParams::default()).await.unwrap();
    assert_eq!(resp.body(), b"first wins");

    let sf2 = StaticFiles::new(vec![tmp2.path().to_path_buf(), tmp1.path().to_path_buf()]);

    let req2 = make_request(Method::GET, "/static/file.txt");
    let resp2 = sf2.serve(req2, PathParams::default()).await.unwrap();
    assert_eq!(resp2.body(), b"second wins");
}

#[tokio::test]
async fn test_router_integration() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("style.css"), b"css").unwrap();

    let sf = StaticFiles::new(vec![tmp.path().to_path_buf()]);
    let handler = sf.handler();

    let router = Router::new()
        .get("/static/{path}", handler.clone())
        .get("/static/{d1}/{path}", handler);

    let req = make_request(Method::GET, "/static/style.css");
    let resp = router.handle(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.body(), b"css");
}

#[tokio::test]
async fn test_collectstatic_behavior() {
    let src1 = TempDir::new().unwrap();
    let src2 = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();

    fs::write(src1.path().join("a.css"), b"content_a").unwrap();
    fs::write(src2.path().join("b.js"), b"content_b").unwrap();
    fs::write(src1.path().join("c.txt"), b"first").unwrap();
    fs::write(src2.path().join("c.txt"), b"second").unwrap();

    let sf = StaticFiles::new(vec![src1.path().to_path_buf(), src2.path().to_path_buf()]);
    let manifest = sf.collect(dest.path()).unwrap();

    assert!(manifest.mapping.contains_key("a.css"));
    assert!(manifest.mapping.contains_key("b.js"));
    assert!(manifest.mapping.contains_key("c.txt"));

    let hashed_a = manifest.mapping.get("a.css").unwrap();
    let hashed_b = manifest.mapping.get("b.js").unwrap();
    let hashed_c = manifest.mapping.get("c.txt").unwrap();

    assert_eq!(fs::read(dest.path().join(hashed_a)).unwrap(), b"content_a");
    assert_eq!(fs::read(dest.path().join(hashed_b)).unwrap(), b"content_b");
    assert_eq!(fs::read(dest.path().join(hashed_c)).unwrap(), b"first");

    let manifest_json = fs::read_to_string(dest.path().join("manifest.json")).unwrap();
    let decoded: Manifest = serde_json::from_str(&manifest_json).unwrap();
    assert_eq!(decoded.mapping.get("a.css"), Some(hashed_a));

    let manifest2 = sf.collect(dest.path()).unwrap();
    assert_eq!(manifest2.mapping.get("a.css"), Some(hashed_a));

    fs::write(src1.path().join("a.css"), b"different_content").unwrap();
    let manifest3 = sf.collect(dest.path()).unwrap();
    let hashed_a_new = manifest3.mapping.get("a.css").unwrap();
    assert_ne!(hashed_a, hashed_a_new);
}

#[tokio::test]
async fn test_local_storage_rejects_traversal() {
    let root = TempDir::new().unwrap();
    let outside = root.path().join("secret.txt");
    fs::write(&outside, b"secret").unwrap();
    let storage = LocalDiskStorage::new(root.path().join("source"), "");
    fs::create_dir_all(root.path().join("source")).unwrap();

    assert!(!storage.exists("../secret.txt").await.unwrap());
    assert!(storage.open("../secret.txt").await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn test_local_storage_rejects_escaping_symlink() {
    use std::os::unix::fs::symlink;
    let root = TempDir::new().unwrap();
    let source = root.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(root.path().join("secret.txt"), b"secret").unwrap();
    symlink(root.path().join("secret.txt"), source.join("link.txt")).unwrap();
    let storage = LocalDiskStorage::new(source, "");
    assert!(!storage.exists("link.txt").await.unwrap());
    assert!(storage.open("link.txt").await.is_err());
}
