use crate::error::StaticFilesError;
use crate::serve::StaticFiles;
use crate::storage::{LocalDiskStorage, Storage};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// A static file manifest mapping original relative paths to content-hashed paths.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
    /// Map of original relative path to hashed relative path.
    pub mapping: std::collections::HashMap<String, String>,
}

fn walk_dir(dir: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    let target_dir = dir.join(current);
    if target_dir.is_dir() {
        for entry in fs::read_dir(target_dir)? {
            let entry = entry?;
            let path = entry.path();
            let rel = path.strip_prefix(dir).unwrap_or(&path).to_path_buf();
            if path.is_dir() {
                walk_dir(dir, &rel, files)?;
            } else if path.is_file() {
                files.push(rel);
            }
        }
    }
    Ok(())
}

fn get_hashed_relative_path(rel_path: &str, hash: &str) -> String {
    let path = Path::new(rel_path);
    let parent = path.parent().unwrap_or(Path::new(""));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(rel_path);

    let hashed_file_name =
        if let Some(ext) = Path::new(file_name).extension().and_then(|e| e.to_str()) {
            if let Some(stem) = Path::new(file_name).file_stem().and_then(|s| s.to_str()) {
                format!("{}.{}.{}", stem, hash, ext)
            } else {
                format!("{}.{}", file_name, hash)
            }
        } else {
            format!("{}.{}", file_name, hash)
        };

    if parent.as_os_str().is_empty() {
        hashed_file_name
    } else {
        parent
            .join(hashed_file_name)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

impl StaticFiles {
    /// Collects static files through an injected storage backend.
    pub async fn collect_to(&self, storage: &dyn Storage) -> Result<Manifest, StaticFilesError> {
        let mut collected_files: std::collections::HashMap<String, PathBuf> =
            std::collections::HashMap::new();

        for dir in &self.source_dirs {
            let mut files = Vec::new();
            if dir.exists() && dir.is_dir() {
                walk_dir(dir, Path::new(""), &mut files)?;
            }
            for rel in files {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                collected_files
                    .entry(rel_str)
                    .or_insert_with(|| dir.join(&rel));
            }
        }

        let mut mapping = std::collections::HashMap::new();
        for (rel_path, abs_path) in collected_files {
            let content = fs::read(&abs_path)?;
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&content);
            let hash_hex = format!("{:x}", hasher.finalize());
            let hash_prefix = if hash_hex.len() >= 8 {
                &hash_hex[..8]
            } else {
                &hash_hex
            };
            let hashed_rel_path = get_hashed_relative_path(&rel_path, hash_prefix);
            storage.save(&hashed_rel_path, content).await?;
            mapping.insert(rel_path, hashed_rel_path);
        }

        let manifest = Manifest { mapping };
        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        storage
            .save("manifest.json", manifest_json.into_bytes())
            .await?;
        Ok(manifest)
    }

    /// Collects static files from source directories into `output_dir` with content hashes and writes `manifest.json`.
    pub fn collect(&self, output_dir: &Path) -> Result<Manifest, StaticFilesError> {
        let storage = LocalDiskStorage::new(output_dir, "");
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| StaticFilesError::Io(std::io::Error::other(e.to_string())))?
                    .block_on(self.collect_to(&storage))
            });
            handle.join().map_err(|_| {
                StaticFilesError::Io(std::io::Error::other("collection thread panicked"))
            })?
        })
    }
}
