//! Local directories and mounted SMB / network shares. Both are just paths on
//! the filesystem (a UNC path or mount point for SMB), walked recursively.

use super::{is_supported, SourceDoc};
use anyhow::{Context, Result};
use kb_core::locator;
use std::path::Path;
use walkdir::WalkDir;

pub fn enumerate(root: &str) -> Result<Vec<SourceDoc>> {
    let root_path = Path::new(root);
    if !root_path.exists() {
        anyhow::bail!("source path does not exist: {root}");
    }

    let mut docs = Vec::new();
    for entry in WalkDir::new(root_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !is_supported(&ext) {
            continue;
        }

        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let meta = entry.metadata().ok();
        let size = meta.as_ref().map(|m| m.len());
        let modified_at = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_string();

        let read_path = abs.clone();
        docs.push(SourceDoc {
            locator: locator::file_url(&abs),
            title,
            ext,
            modified_at,
            size,
            read: Box::new(move || {
                std::fs::read(&read_path)
                    .with_context(|| format!("reading {}", read_path.display()))
            }),
        });
    }
    Ok(docs)
}
