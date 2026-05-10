//! Look up the source file for a given output `.md`. DB-first; falls back to
//! parsing the HTML-comment annotation embedded in the file when the path
//! is outside any vault we know about.

use crate::annotation::Annotation;
use crate::error::Result;
use crate::registry;
use crate::vault::MdVault;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhenceResult {
    pub source: String,
    pub source_hash: Option<String>,
    pub extractor: Option<String>,
    pub converted_at_ms: Option<i64>,
    /// "db" if we resolved it via the vault's DB, "annotation" if via the
    /// HTML-comment marker on the file itself.
    pub via: &'static str,
    /// The vault root, when resolved via DB.
    pub vault_root: Option<PathBuf>,
}

/// Try the DB first (if `vault` is provided and the path falls under
/// `output_dir`), then fall back to parsing the file's annotation.
pub fn whence(vault: Option<&MdVault>, path: &Path) -> Result<Option<WhenceResult>> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let abs = abs.canonicalize().unwrap_or(abs);

    // DB path: if `path` is under the vault's output_dir, look up by the
    // output-dir-relative path.
    if let Some(v) = vault {
        let out_root = v
            .output_dir_abs()
            .canonicalize()
            .unwrap_or_else(|_| v.output_dir_abs());
        if let Ok(rel) = abs.strip_prefix(&out_root) {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if let Some(row) = registry::find_by_output_path(&v.conn, &rel_str)? {
                return Ok(Some(WhenceResult {
                    source: row.input_path,
                    source_hash: row.last_src_hash,
                    extractor: row.extractor,
                    converted_at_ms: row.last_converted,
                    via: "db",
                    vault_root: Some(v.root.clone()),
                }));
            }
        }
    }

    // Fallback: read the file and parse the annotation.
    if !abs.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&abs)?;
    let text = String::from_utf8_lossy(&bytes);
    let Some(ann) = Annotation::parse(&text) else {
        return Ok(None);
    };
    Ok(Some(WhenceResult {
        source: ann.source,
        source_hash: if ann.source_hash.is_empty() {
            None
        } else {
            Some(ann.source_hash)
        },
        extractor: if ann.extractor.is_empty() {
            None
        } else {
            Some(ann.extractor)
        },
        converted_at_ms: if ann.converted_at_ms == 0 {
            None
        } else {
            Some(ann.converted_at_ms)
        },
        via: "annotation",
        vault_root: None,
    }))
}
