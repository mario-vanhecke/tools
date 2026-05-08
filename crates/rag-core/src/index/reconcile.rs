use super::pipeline::{write_indexed_content, write_status_only};
use super::{FileResult, IndexOptions, Outcome};
use crate::chunk::{sha256_hex, Chunk, ChunkInput};
use crate::config::ChunkingConfig;
use crate::embed::Embedder;
use crate::error::Result;
use crate::extract::{ExtractionResult, ExtractorRegistry};
use crate::registry::{FileRow, FileStatus};
use crate::vault::Vault;
use std::path::{Path, PathBuf};

/// Outcome of the pre-extraction decision tree.
pub enum Precheck {
    /// Row was fully resolved (status was updated and/or no work is needed).
    Resolved(FileResult),
    /// Row needs extraction + chunking. The main thread dispatches `task` to
    /// a worker; the worker calls `do_extract` and the result is fed back to
    /// `finalize` (which runs the embedder + DB write on the main thread).
    NeedsExtraction(ExtractTask),
}

#[derive(Clone, Debug)]
pub struct ExtractTask {
    pub row: FileRow,
    pub abs_path: PathBuf,
    pub ext: String,
    pub mtime_ms: Option<i64>,
    pub size: i64,
    pub title: Option<String>,
}

#[derive(Debug)]
pub enum Extracted {
    Chunks(Vec<Chunk>),
    NeedsOcr,
    Failed { detail: String, message: String },
    NoExtractor,
}

/// Phase 1 of the per-row pipeline: stat, classify, and either resolve inline
/// (status update only) or hand back an `ExtractTask` for the worker pool.
pub fn precheck(vault: &Vault, row: &FileRow, opts: &IndexOptions) -> Result<Precheck> {
    let abs = vault.absolutize(&row.path);
    let meta = std::fs::metadata(&abs);

    let meta = match meta {
        Ok(m) => m,
        Err(_) => {
            write_status_only(
                vault,
                row,
                FileStatus::Missing,
                None,
                None,
                row.status != FileStatus::Missing,
            )?;
            return Ok(Precheck::Resolved(FileResult {
                path: row.path.clone(),
                outcome: Outcome::Missing,
                chunks_added: 0,
                chunks_replaced: 0,
                status_detail: None,
                status_note: None,
            }));
        }
    };
    if !meta.is_file() {
        write_status_only(
            vault,
            row,
            FileStatus::Failed,
            Some("path_not_a_file"),
            Some("registered path is not a regular file"),
            true,
        )?;
        return Ok(Precheck::Resolved(FileResult {
            path: row.path.clone(),
            outcome: Outcome::Failed,
            chunks_added: 0,
            chunks_replaced: 0,
            status_detail: Some("path_not_a_file".to_string()),
            status_note: Some("registered path is not a regular file".to_string()),
        }));
    }

    let ext = Path::new(&row.path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if !vault
        .config
        .files
        .supported_extensions
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&ext))
    {
        write_status_only(
            vault,
            row,
            FileStatus::Unsupported,
            Some("extension_not_supported"),
            None,
            row.status == FileStatus::Indexed,
        )?;
        return Ok(Precheck::Resolved(FileResult {
            path: row.path.clone(),
            outcome: Outcome::Unsupported,
            chunks_added: 0,
            chunks_replaced: 0,
            status_detail: Some("extension_not_supported".to_string()),
            status_note: None,
        }));
    }

    if vault
        .config
        .files
        .excluded_extensions
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&ext))
    {
        write_status_only(
            vault,
            row,
            FileStatus::Excluded,
            Some("extension_excluded_by_config"),
            None,
            row.status == FileStatus::Indexed,
        )?;
        return Ok(Precheck::Resolved(FileResult {
            path: row.path.clone(),
            outcome: Outcome::Excluded,
            chunks_added: 0,
            chunks_replaced: 0,
            status_detail: Some("extension_excluded_by_config".to_string()),
            status_note: None,
        }));
    }

    if meta.len() > vault.config.files.size_cap_bytes {
        write_status_only(
            vault,
            row,
            FileStatus::TooLarge,
            Some("size_exceeds_cap"),
            None,
            row.status == FileStatus::Indexed,
        )?;
        return Ok(Precheck::Resolved(FileResult {
            path: row.path.clone(),
            outcome: Outcome::TooLarge,
            chunks_added: 0,
            chunks_replaced: 0,
            status_detail: Some("size_exceeds_cap".to_string()),
            status_note: None,
        }));
    }

    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    let size = meta.len() as i64;

    if row.status == FileStatus::Indexed
        && !opts.force
        && row.last_mtime == mtime_ms
        && row.last_size == Some(size)
    {
        return Ok(Precheck::Resolved(FileResult {
            path: row.path.clone(),
            outcome: Outcome::Skipped,
            chunks_added: 0,
            chunks_replaced: 0,
            status_detail: None,
            status_note: None,
        }));
    }

    if row.status == FileStatus::Failed && !opts.retry_failed && !opts.force {
        return Ok(Precheck::Resolved(FileResult {
            path: row.path.clone(),
            outcome: Outcome::Skipped,
            chunks_added: 0,
            chunks_replaced: 0,
            status_detail: row.status_detail.clone(),
            status_note: row.status_note.clone(),
        }));
    }

    let title = Path::new(&row.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());

    Ok(Precheck::NeedsExtraction(ExtractTask {
        row: row.clone(),
        abs_path: abs,
        ext,
        mtime_ms,
        size,
        title,
    }))
}

/// Phase 2 (worker thread): extract + chunk. No DB or vault access; pure data.
pub fn do_extract(
    extractors: &ExtractorRegistry,
    chunking: &ChunkingConfig,
    task: &ExtractTask,
) -> Extracted {
    let extractor = match extractors.for_extension(&task.ext) {
        Some(e) => e.clone(),
        None => return Extracted::NoExtractor,
    };
    match extractor.extract(&task.abs_path) {
        ExtractionResult::NeedsOcr => Extracted::NeedsOcr,
        ExtractionResult::Failed { detail, message } => Extracted::Failed { detail, message },
        ExtractionResult::Ok(extracted) => {
            let chunks = crate::chunk::chunk(
                &ChunkInput {
                    markdown: &extracted.markdown,
                    page_boundaries: extracted.page_boundaries.as_deref(),
                    document_title: task.title.as_deref(),
                },
                chunking,
            );
            if chunks.is_empty() {
                Extracted::Failed {
                    detail: "no_chunks_produced".to_string(),
                    message: "extraction returned content but chunker produced no chunks"
                        .to_string(),
                }
            } else {
                Extracted::Chunks(chunks)
            }
        }
    }
}

/// Phase 3 (main thread): given the worker's extraction result, embed and
/// write transactionally. The consistency invariant is upheld here.
pub fn finalize(
    vault: &Vault,
    embedder: &dyn Embedder,
    task: ExtractTask,
    extracted: Extracted,
) -> Result<FileResult> {
    match extracted {
        Extracted::NoExtractor => {
            let detail = "no_extractor_available";
            let message = format!("no extractor registered for .{}", task.ext);
            write_status_only(
                vault,
                &task.row,
                FileStatus::Failed,
                Some(detail),
                Some(&message),
                task.row.status == FileStatus::Indexed,
            )?;
            Ok(FileResult {
                path: task.row.path,
                outcome: Outcome::Failed,
                chunks_added: 0,
                chunks_replaced: 0,
                status_detail: Some(detail.to_string()),
                status_note: Some(message),
            })
        }
        Extracted::NeedsOcr => {
            write_status_only(
                vault,
                &task.row,
                FileStatus::NeedsOcr,
                Some("no_extractable_text"),
                None,
                task.row.status == FileStatus::Indexed,
            )?;
            Ok(FileResult {
                path: task.row.path,
                outcome: Outcome::NeedsOcr,
                chunks_added: 0,
                chunks_replaced: 0,
                status_detail: Some("no_extractable_text".to_string()),
                status_note: None,
            })
        }
        Extracted::Failed { detail, message } => {
            write_status_only(
                vault,
                &task.row,
                FileStatus::Failed,
                Some(&detail),
                Some(&message),
                task.row.status == FileStatus::Indexed,
            )?;
            Ok(FileResult {
                path: task.row.path,
                outcome: Outcome::Failed,
                chunks_added: 0,
                chunks_replaced: 0,
                status_detail: Some(detail),
                status_note: Some(message),
            })
        }
        Extracted::Chunks(chunks) => {
            let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
            let embeddings = match embedder.embed_batch(&texts) {
                Ok(e) => e,
                Err(e) => {
                    write_status_only(
                        vault,
                        &task.row,
                        FileStatus::Failed,
                        Some("embedding_error"),
                        Some(&e.to_string()),
                        task.row.status == FileStatus::Indexed,
                    )?;
                    return Ok(FileResult {
                        path: task.row.path,
                        outcome: Outcome::Failed,
                        chunks_added: 0,
                        chunks_replaced: 0,
                        status_detail: Some("embedding_error".to_string()),
                        status_note: Some(e.to_string()),
                    });
                }
            };

            let bytes = std::fs::read(&task.abs_path)?;
            let content_hash = sha256_hex(&bytes);

            let chunks_added = chunks.len() as u32;
            let chunks_replaced = if task.row.status == FileStatus::Indexed {
                count_existing_chunks(vault, task.row.id)?
            } else {
                0
            };

            write_indexed_content(
                vault,
                &task.row,
                &chunks,
                &embeddings,
                task.mtime_ms,
                task.size,
                &content_hash,
            )?;

            Ok(FileResult {
                path: task.row.path,
                outcome: Outcome::Indexed,
                chunks_added,
                chunks_replaced,
                status_detail: None,
                status_note: None,
            })
        }
    }
}

/// Sequential per-row pipeline (used when extract_concurrency = 1 and by
/// existing callers). Wraps precheck + do_extract + finalize.
pub fn process_one(
    vault: &Vault,
    row: &FileRow,
    extractors: &ExtractorRegistry,
    embedder: &dyn Embedder,
    opts: &IndexOptions,
) -> Result<FileResult> {
    match precheck(vault, row, opts)? {
        Precheck::Resolved(result) => Ok(result),
        Precheck::NeedsExtraction(task) => {
            let extracted = do_extract(extractors, &vault.config.chunking, &task);
            finalize(vault, embedder, task, extracted)
        }
    }
}

fn count_existing_chunks(vault: &Vault, file_id: i64) -> Result<u32> {
    let n: i64 = vault.conn.query_row(
        "SELECT COUNT(*) FROM chunks WHERE file_id = ?1",
        rusqlite::params![file_id],
        |r| r.get(0),
    )?;
    Ok(n as u32)
}
