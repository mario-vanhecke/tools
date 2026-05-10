pub mod pipeline;
pub mod reconcile;

use crate::error::{Error, Result};
use crate::registry::{self, OutputRow};
use crate::status::FileStatus;
use crate::vault::MdVault;
use extract_core::ExtractorRegistry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vault_core::FileExt;

#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// Re-convert clean rows even if the source hasn't changed.
    pub force: bool,
    /// Retry rows that ended up `failed` last time.
    pub retry_failed: bool,
    /// Re-convert rows currently in `conflict`, discarding any hand edits to the output.
    pub overwrite: bool,
    /// Treat the existing output as authoritative; clear the conflict
    /// (without re-converting) by recording a new last_out_hash.
    pub keep_existing: bool,
    /// Restrict to specific input paths.
    pub paths: Option<Vec<PathBuf>>,
    pub no_wait: bool,
    pub wait_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertReport {
    pub started_at: i64,
    pub completed_at: i64,
    pub duration_ms: i64,
    pub summary: ConvertSummary,
    pub results: Vec<FileResult>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConvertSummary {
    pub converted: u32,
    pub skipped: u32,
    pub failed: u32,
    pub missing: u32,
    pub unsupported: u32,
    pub excluded: u32,
    pub too_large: u32,
    pub needs_ocr: u32,
    pub conflict: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResult {
    pub input_path: String,
    pub output_path: Option<String>,
    pub outcome: Outcome,
    pub status_detail: Option<String>,
    pub status_note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Converted,
    Skipped,
    Failed,
    Unsupported,
    Excluded,
    TooLarge,
    NeedsOcr,
    Missing,
    Conflict,
}

impl Outcome {
    pub fn tally(&self, s: &mut ConvertSummary) {
        match self {
            Outcome::Converted => s.converted += 1,
            Outcome::Skipped => s.skipped += 1,
            Outcome::Failed => s.failed += 1,
            Outcome::Missing => s.missing += 1,
            Outcome::Unsupported => s.unsupported += 1,
            Outcome::Excluded => s.excluded += 1,
            Outcome::TooLarge => s.too_large += 1,
            Outcome::NeedsOcr => s.needs_ocr += 1,
            Outcome::Conflict => s.conflict += 1,
        }
    }
}

/// Map a per-file processing error to a Failed FileResult plus a DB write so
/// subsequent runs skip it. Mirrors rag-core's `per_file_failure`.
fn per_file_failure(vault: &MdVault, row: &OutputRow, err: &Error) -> Result<FileResult> {
    let detail = "internal_error";
    let message = err.to_string();
    tracing::warn!("{}: {}", row.input_path, message);
    pipeline::write_status_only(
        vault,
        row,
        FileStatus::Failed,
        Some(detail),
        Some(&message),
        false,
    )?;
    Ok(FileResult {
        input_path: row.input_path.clone(),
        output_path: row.output_path.clone(),
        outcome: Outcome::Failed,
        status_detail: Some(detail.to_string()),
        status_note: Some(message),
    })
}

pub fn run_convert(
    vault: &mut MdVault,
    extractors: &ExtractorRegistry,
    opts: &ConvertOptions,
    progress: Option<&dyn Fn(usize, usize, &str)>,
) -> Result<ConvertReport> {
    // Acquire vault-level file lock.
    let lock_file = vault_core::acquire_lock(
        &vault.convert_lock_path(),
        &vault_core::LockOptions {
            no_wait: opts.no_wait,
            wait_seconds: opts.wait_seconds,
        },
    )?;

    let started_at = chrono::Utc::now().timestamp_millis();

    // Snapshot rows.
    let target_paths: Option<Vec<String>> = match &opts.paths {
        Some(ps) => Some(
            ps.iter()
                .map(|p| vault.relativize(p).map(|r| r.to_string_lossy().to_string()))
                .collect::<Result<Vec<_>>>()?,
        ),
        None => None,
    };

    let mut rows = registry::list_all(&vault.conn)?;
    if let Some(paths) = target_paths {
        let set: std::collections::HashSet<&str> = paths.iter().map(|s| s.as_str()).collect();
        rows.retain(|r| set.contains(r.input_path.as_str()));
    }

    let total = rows.len();
    let mut results: Vec<FileResult> = Vec::with_capacity(total);
    let mut summary = ConvertSummary::default();

    // Sequential. Conversion is mostly I/O-bound (extractors + a single
    // small write per file) so we don't bother with the worker-pool pattern
    // rag's index uses. If conversion ever becomes a bottleneck we can copy
    // rag's parallel pipeline.
    for (i, row) in rows.iter().enumerate() {
        if let Some(p) = progress {
            p(i, total, &row.input_path);
        }
        let res = match reconcile::process_one(vault, row, extractors, opts) {
            Ok(r) => r,
            Err(e) => per_file_failure(vault, row, &e)?,
        };
        res.outcome.tally(&mut summary);
        results.push(res);
    }

    if let Some(p) = progress {
        p(total, total, "");
    }

    let _ = FileExt::unlock(&lock_file);
    results.sort_by(|a, b| a.input_path.cmp(&b.input_path));

    let completed_at = chrono::Utc::now().timestamp_millis();
    Ok(ConvertReport {
        started_at,
        completed_at,
        duration_ms: completed_at - started_at,
        summary,
        results,
    })
}
