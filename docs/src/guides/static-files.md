# Static Files

Managing CSS, JavaScript, images, and other assets is handled by the `djangors-staticfiles` subsystem. This provides dev-time asset serving, a pluggable storage model, and a production `collectstatic` asset pipeline that generates content-hashed assets and a tracking manifest.

---

## Dev-Time Serving

During local development, Djangors can serve static assets directly from your source directories.

```rust,illustrative
use djangors::staticfiles::StaticFiles;
use std::path::PathBuf;

// Set up source directories
let static_dirs = vec![
    PathBuf::from("src/static"),
    PathBuf::from("static_assets"),
];

let static_files = StaticFiles::new(static_dirs);

// Mount the static files handler in the router
let app = djangors::Router::new()
    .mount("/static/<path:path>", static_files.handler());
```

When a request arrives at `/static/style.css`, the `StaticFiles` handler:
1. Loops through your configured source directories in the order they were defined.
2. Checks if the requested file exists in that directory.
3. Serves the file with the appropriate `Content-Type` header (such as `text/css; charset=utf-8` or `application/javascript; charset=utf-8`).

---

## The `Storage` Trait

Djangors abstracts file read and write operations behind the `Storage` trait, allowing you to use different asset destinations in development versus production:

```rust,illustrative
use async_trait::async_trait;
use djangors::staticfiles::StaticFilesError;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn save(&self, path: &str, content: Vec<u8>) -> Result<String, StaticFilesError>;
    async fn open(&self, path: &str) -> Result<Vec<u8>, StaticFilesError>;
    async fn exists(&self, path: &str) -> Result<bool, StaticFilesError>;
    async fn delete(&self, path: &str) -> Result<(), StaticFilesError>;
    fn url(&self, path: &str) -> String;
}
```

### Supported Implementations
* **`LocalDiskStorage`**: Saves and reads files from a directory on the local filesystem. To protect against directory traversal attacks, it canonicalizes paths and verifies they reside within the root.
* **`S3Storage`**: Interacts with AWS S3 or any S3-compatible service (such as MinIO, Cloudflare R2, or DigitalOcean Spaces) using the `Client` API.

---

## Production Collection (`collectstatic`)

In production, serving assets through the Rust application process is inefficient. Instead, you run the collection pipeline to bundle and write your assets to a production storage destination (such as local disk served by Nginx, or an S3 bucket with a CDN).

To collect static files, use the `collect` method (for local disk storage) or `collect_to` (for any `Storage` backend):

```rust,illustrative
// Collects all static files into the "staticfiles/" directory
static_files.collect(Path::new("staticfiles/"))?;
```

You can also trigger this via the command-line utility:
```bash
dj collectstatic
```

### Content Hashing and `manifest.json`
When running the collection process:
1. Djangors crawls all source directories and reads every asset.
2. It computes the SHA-256 hash of each file's contents.
3. It appends the first 8 characters of the hash to the file's filename (e.g. `logo.css` becomes `logo.8bf74a2d.css`), ensuring browsers cache assets forever.
4. It saves the hashed files via the configured storage backend.
5. It generates a `manifest.json` file mapping the original paths to the hashed paths:

```json
{
  "mapping": {
    "css/style.css": "css/style.f8a7e3d1.css",
    "images/logo.png": "images/logo.a8c6e28f.png"
  }
}
```

6. The `manifest.json` is saved in the root of the storage directory, allowing template helpers to resolve path names to their production content-hashed variants.
