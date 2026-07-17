use crate::error::StaticFilesError;
use crate::serve::StaticFiles;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
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
    pub fn collect(&self, output_dir: &Path) -> Result<Manifest, StaticFilesError> {
        fs::create_dir_all(output_dir)?;

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
            let result = hasher.finalize();
            let hash_hex = format!("{:x}", result);
            let hash_prefix = if hash_hex.len() >= 8 {
                &hash_hex[..8]
            } else {
                &hash_hex
            };

            let hashed_rel_path = get_hashed_relative_path(&rel_path, hash_prefix);

            let dest_path = output_dir.join(&hashed_rel_path);
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&dest_path, &content)?;

            mapping.insert(rel_path, hashed_rel_path);
        }

        let manifest = Manifest { mapping };
        let manifest_path = output_dir.join("manifest.json");
        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        fs::write(manifest_path, manifest_json)?;

        Ok(manifest)
    }
}
