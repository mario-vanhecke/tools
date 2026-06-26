//! `distill build` — single-pass: config → SQLite+sqlite-vec index.
//!
//! Reference-only and incremental: documents unchanged since the last build
//! (by mtime+size, then content hash) are skipped without re-embedding; nothing
//! is copied or converted to disk (temp files during extraction are transient).

use crate::extract::{self, Converters};
use crate::sources::{self, SourceDoc};
use anyhow::{Context, Result};
use kb_core::chunk::chunk;
use kb_core::index::DocumentInput;
use kb_core::{Config, Embedder, Index};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct SourceReport {
    pub name: String,
    pub kind: String,
    pub indexed: usize,
    pub skipped: usize,
    pub unsupported: usize,
    pub pruned: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct BuildReport {
    pub sources: Vec<SourceReport>,
    pub documents: i64,
    pub chunks: i64,
    pub output: String,
}

pub fn build(cfg: &Config, force: bool) -> Result<BuildReport> {
    // Clear any orphaned temp files from a previous crashed run.
    extract::sweep_temp_dir();
    let conv = Converters::detect();
    let embedder = Embedder::from_config(&cfg.embedding).context("embedding config")?;
    let mut index = Index::open_or_create(&cfg.output.path, &cfg.embedding)
        .with_context(|| format!("opening index {}", cfg.output.path))?;

    let mut report = BuildReport {
        output: cfg.output.path.clone(),
        ..Default::default()
    };

    for src in &cfg.sources {
        let mut sr = SourceReport {
            name: src.name(),
            kind: src.kind().to_string(),
            ..Default::default()
        };

        let docs = match sources::enumerate(src) {
            Ok(d) => d,
            Err(e) => {
                sr.errors.push(e.to_string());
                report.sources.push(sr);
                continue;
            }
        };

        let mut seen = HashSet::new();
        for doc in docs {
            seen.insert(doc.locator.clone());
            match index_one(
                &mut index,
                &embedder,
                &conv,
                cfg,
                &sr.name,
                src.kind(),
                doc,
                force,
            ) {
                Ok(Outcome::Indexed) => sr.indexed += 1,
                Ok(Outcome::Skipped) => sr.skipped += 1,
                Ok(Outcome::Unsupported) => sr.unsupported += 1,
                Err(e) => sr.errors.push(e.to_string()),
            }
        }

        // Retire documents that vanished at origin.
        sr.pruned = index.prune_source(&sr.name, &seen).unwrap_or(0);
        report.sources.push(sr);
    }

    let stats = index.stats()?;
    report.documents = stats.documents;
    report.chunks = stats.chunks;
    Ok(report)
}

enum Outcome {
    Indexed,
    Skipped,
    Unsupported,
}

#[allow(clippy::too_many_arguments)]
fn index_one(
    index: &mut Index,
    embedder: &Embedder,
    conv: &Converters,
    cfg: &Config,
    source: &str,
    kind: &str,
    doc: SourceDoc,
    force: bool,
) -> Result<Outcome> {
    let prior = index.document_state(&doc.locator)?;

    // Cheap skip: same mtime + size as last time.
    if !force {
        if let Some(st) = &prior {
            let same_mtime = st.modified_at.is_some() && st.modified_at == doc.modified_at;
            let same_size = st.size == doc.size.map(|s| s as i64);
            if same_mtime && same_size {
                return Ok(Outcome::Skipped);
            }
        }
    }

    let bytes = (doc.read)()?;
    let hash = sha256_hex(&bytes);

    // Content-hash skip (handles mtime churn with identical bytes).
    if !force {
        if let Some(st) = &prior {
            if st.content_hash == hash {
                return Ok(Outcome::Skipped);
            }
        }
    }

    let text = match extract::extract(&doc.ext, &bytes, conv)? {
        Some(t) => t,
        None => return Ok(Outcome::Unsupported),
    };

    let chunks = chunk(&text, &cfg.chunk);
    if chunks.is_empty() {
        return Ok(Outcome::Unsupported);
    }
    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let embeddings = embedder
        .embed_batch(&texts)
        .with_context(|| format!("embedding {}", doc.locator))?;
    let pairs: Vec<(kb_core::chunk::Chunk, Vec<f32>)> =
        chunks.into_iter().zip(embeddings).collect();

    let input = DocumentInput {
        source: source.to_string(),
        kind: kind.to_string(),
        locator: doc.locator.clone(),
        title: doc.title.clone(),
        content_hash: hash,
        modified_at: doc.modified_at.clone(),
        size: doc.size,
    };
    index.add_document(&input, &pairs)?;
    Ok(Outcome::Indexed)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
