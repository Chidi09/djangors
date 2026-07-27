use djangors_core::{DjangorsError, PathParams, Request, Response};
use hyper::StatusCode;
use std::fs;
use std::path::{Path, PathBuf};

/// Serves static files from configured source directories.
pub struct StaticFiles {
    pub(crate) source_dirs: Vec<PathBuf>,
}

impl StaticFiles {
    /// Creates a new `StaticFiles` instance with the given search directories.
    pub fn new(source_dirs: Vec<PathBuf>) -> Self {
        StaticFiles { source_dirs }
    }

    /// Serve a single file, resolved against `source_dirs` in order.
    pub async fn serve(&self, req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let rel_path = if let Some(path_param) = params.get("path") {
            path_param
        } else {
            let raw_path = req.path();
            if let Some(stripped) = raw_path.strip_prefix("/static/") {
                stripped
            } else {
                raw_path.trim_start_matches('/')
            }
        };

        let resolved = self.resolve_path(rel_path)?;

        let content = fs::read(&resolved).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                DjangorsError::NotFound
            } else {
                DjangorsError::Internal(e.to_string())
            }
        })?;

        let ext = resolved.extension().and_then(|e| e.to_str()).unwrap_or("");

        let content_type = match ext.to_lowercase().as_str() {
            "css" => "text/css; charset=utf-8",
            "js" => "application/javascript; charset=utf-8",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "svg" => "image/svg+xml",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "json" => "application/json; charset=utf-8",
            _ => "application/octet-stream",
        };

        Ok(Response::bytes(StatusCode::OK, content_type, content))
    }

    fn resolve_path(&self, rel_path: &str) -> Result<PathBuf, DjangorsError> {
        if rel_path.is_empty() {
            return Err(DjangorsError::NotFound);
        }

        let path_obj = Path::new(rel_path);

        for dir in &self.source_dirs {
            let target = dir.join(path_obj);

            if !target.exists() {
                continue;
            }

            let canonical_target = match fs::canonicalize(&target) {
                Ok(p) => p,
                Err(_) => continue,
            };

            let canonical_dir = match fs::canonicalize(dir) {
                Ok(p) => p,
                Err(_) => continue,
            };

            if canonical_target.starts_with(&canonical_dir) {
                return Ok(canonical_target);
            } else {
                return Err(DjangorsError::NotFound);
            }
        }

        Err(DjangorsError::NotFound)
    }

    /// Returns a Handler-compatible type capturing the configuration.
    pub fn handler(&self) -> StaticFilesHandler {
        StaticFilesHandler {
            source_dirs: self.source_dirs.clone(),
        }
    }
}

/// Router handler for serving static files.
#[derive(Clone)]
pub struct StaticFilesHandler {
    source_dirs: Vec<PathBuf>,
}

impl djangors_core::Handler for StaticFilesHandler {
    fn call(
        &self,
        req: Request,
        params: PathParams,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, DjangorsError>> + Send>>
    {
        let sf = StaticFiles::new(self.source_dirs.clone());
        Box::pin(async move { sf.serve(req, params).await })
    }
}
