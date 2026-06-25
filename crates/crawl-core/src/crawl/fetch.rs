//! The fetch (materialize) pass: write the actual bytes of discovered documents
//! into a local tree, **uniformly for every source kind** — copy for local/smb,
//! download for SharePoint. The output is always `<out>/<source>/<rel_path>`, so
//! downstream tools (`md`, `rag`) consume one shape regardless of origin.

use super::sharepoint;
use crate::error::{Error, Result};
use crate::registry::{self, sources, DocQuery, DocumentRow};
use crate::source::{Source, SourceKind};
use crate::status::DocStatus;
use crate::vault::CrawlVault;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FetchOptions {
    /// Root directory the materialized tree is written under.
    pub out_dir: PathBuf,
    /// Only fetch this source. `None` = all enabled-or-not sources.
    pub source: Option<String>,
    pub extension: Option<String>,
    /// Status filter. `None` = present + modified (the live documents).
    pub status: Option<DocStatus>,
    /// Re-download/copy even if an up-to-date local copy already exists.
    pub force: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FetchReport {
    pub out_dir: String,
    pub sources: Vec<SourceFetchReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFetchReport {
    pub source: String,
    pub kind: String,
    pub fetched: u32,
    pub skipped: u32, // already up to date
    pub errors: u32,
    pub bytes: u64,
}

pub fn run(vault: &CrawlVault, opts: &FetchOptions) -> Result<FetchReport> {
    let mut all = sources::list_sources(&vault.conn)?;
    if let Some(name) = &opts.source {
        all.retain(|s| &s.name == name);
        if all.is_empty() {
            return Err(Error::NoSuchSource(name.clone()));
        }
    }
    let mut report = FetchReport {
        out_dir: opts.out_dir.to_string_lossy().to_string(),
        sources: Vec::new(),
    };
    for src in &all {
        report.sources.push(fetch_source(vault, src, opts)?);
    }
    Ok(report)
}

fn fetch_source(
    vault: &CrawlVault,
    src: &Source,
    opts: &FetchOptions,
) -> Result<SourceFetchReport> {
    let mut rep = SourceFetchReport {
        source: src.name.clone(),
        kind: src.kind.as_str().to_string(),
        fetched: 0,
        skipped: 0,
        errors: 0,
        bytes: 0,
    };

    let q = DocQuery {
        status: opts.status,
        source_id: Some(src.id),
        extension: opts.extension.clone(),
        name_like: None,
        limit: None,
    };
    let mut docs = registry::query_documents(&vault.conn, &q)?;
    if opts.status.is_none() {
        docs.retain(|d| matches!(d.status, DocStatus::Present | DocStatus::Modified));
    }

    let safe_src = sanitize(&src.name);
    for doc in &docs {
        let rel = doc.rel_path.clone().unwrap_or_else(|| doc.name.clone());
        let dest = opts.out_dir.join(&safe_src).join(&rel);
        if !opts.force && up_to_date(&dest, doc) {
            rep.skipped += 1;
            continue;
        }
        let bytes = match src.kind {
            // Local/SMB documents are already on disk; the uri is their path.
            SourceKind::Local | SourceKind::Smb => std::fs::read(&doc.uri).map_err(Error::Io),
            SourceKind::SharePoint => sharepoint::download_document(src, &vault.state_dir, doc),
        };
        match bytes {
            Ok(b) => {
                if let Some(parent) = dest.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        eprintln!("warning: {}: {e}", dest.display());
                        rep.errors += 1;
                        continue;
                    }
                }
                match std::fs::write(&dest, &b) {
                    Ok(()) => {
                        rep.fetched += 1;
                        rep.bytes += b.len() as u64;
                    }
                    Err(e) => {
                        eprintln!("warning: {}: {e}", dest.display());
                        rep.errors += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("warning: could not fetch '{}': {e}", doc.name);
                rep.errors += 1;
            }
        }
    }
    Ok(rep)
}

/// An on-disk copy is current if it exists and its size matches the registry.
fn up_to_date(dest: &Path, doc: &DocumentRow) -> bool {
    match std::fs::metadata(dest) {
        Ok(m) => doc.size.map(|s| s as u64 == m.len()).unwrap_or(false),
        Err(_) => false,
    }
}

/// Make a source name safe to use as a single path segment.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect()
}
