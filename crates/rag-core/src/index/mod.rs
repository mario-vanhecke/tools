pub mod pipeline;
pub mod reconcile;

use crate::embed::Embedder;
use crate::error::{Error, Result};
use crate::extract::ExtractorRegistry;
use crate::vault::Vault;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    pub force: bool,
    pub retry_failed: bool,
    pub paths: Option<Vec<PathBuf>>,
    pub no_wait: bool,
    pub wait_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexReport {
    pub started_at: i64,
    pub completed_at: i64,
    pub duration_ms: i64,
    pub summary: IndexSummary,
    pub results: Vec<FileResult>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexSummary {
    pub indexed: u32,
    pub skipped: u32,
    pub failed: u32,
    pub missing: u32,
    pub unsupported: u32,
    pub excluded: u32,
    pub too_large: u32,
    pub needs_ocr: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResult {
    pub path: String,
    pub outcome: Outcome,
    pub chunks_added: u32,
    pub chunks_replaced: u32,
    pub status_detail: Option<String>,
    pub status_note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Indexed,
    Skipped,
    Failed,
    Unsupported,
    Excluded,
    TooLarge,
    NeedsOcr,
    Missing,
}

impl Outcome {
    pub fn tally(&self, s: &mut IndexSummary) {
        match self {
            Outcome::Indexed => s.indexed += 1,
            Outcome::Skipped => s.skipped += 1,
            Outcome::Failed => s.failed += 1,
            Outcome::Missing => s.missing += 1,
            Outcome::Unsupported => s.unsupported += 1,
            Outcome::Excluded => s.excluded += 1,
            Outcome::TooLarge => s.too_large += 1,
            Outcome::NeedsOcr => s.needs_ocr += 1,
        }
    }
}

pub fn run_index(
    vault: &mut Vault,
    embedder: &dyn Embedder,
    extractors: &ExtractorRegistry,
    opts: &IndexOptions,
    progress: Option<&dyn Fn(usize, usize, &str)>,
) -> Result<IndexReport> {
    if embedder.dimension() != 1024 {
        return Err(Error::Invariant(format!(
            "embedder dimension is {} but schema expects 1024",
            embedder.dimension()
        )));
    }

    // Acquire vault-level file lock
    let lock_path = vault.index_lock_path();
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;

    if opts.no_wait {
        lock_file
            .try_lock_exclusive()
            .map_err(|_| Error::LockContention)?;
    } else {
        // We don't have a true bounded blocking lock from fs2; emulate with
        // try_lock_exclusive in a short loop bounded by `wait_seconds`.
        let deadline =
            std::time::Instant::now() + Duration::from_secs(opts.wait_seconds.unwrap_or(60));
        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => break,
                Err(_) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(Error::LockContention);
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }

    let started_at = chrono::Utc::now().timestamp_millis();

    // Snapshot the rows we'll process.
    let target_paths = match &opts.paths {
        Some(ps) => Some(
            ps.iter()
                .map(|p| vault.relativize(p).map(|r| r.to_string_lossy().to_string()))
                .collect::<Result<Vec<_>>>()?,
        ),
        None => None,
    };

    let mut rows = crate::registry::list_all(&vault.conn)?;
    if let Some(paths) = target_paths {
        let set: std::collections::HashSet<&str> = paths.iter().map(|s| s.as_str()).collect();
        rows.retain(|r| set.contains(r.path.as_str()));
    }

    let total = rows.len();
    let mut results: Vec<FileResult> = Vec::with_capacity(total);
    let mut summary = IndexSummary::default();

    let concurrency = vault.config.indexing.extract_concurrency.max(1) as usize;

    if concurrency == 1 || total <= 1 {
        // Sequential path — keeps simple semantics for small vaults and tests.
        for (i, row) in rows.iter().enumerate() {
            if let Some(p) = progress {
                p(i, total, &row.path);
            }
            let res = reconcile::process_one(vault, row, extractors, embedder, opts)?;
            res.outcome.tally(&mut summary);
            results.push(res);
        }
    } else {
        run_index_parallel(
            vault,
            embedder,
            extractors,
            opts,
            &rows,
            concurrency,
            progress,
            &mut summary,
            &mut results,
        )?;
        // Restore deterministic order so the JSON output is stable.
        results.sort_by(|a, b| a.path.cmp(&b.path));
    }

    if let Some(p) = progress {
        p(total, total, "");
    }

    let _ = FileExt::unlock(&lock_file);

    let completed_at = chrono::Utc::now().timestamp_millis();
    Ok(IndexReport {
        started_at,
        completed_at,
        duration_ms: completed_at - started_at,
        summary,
        results,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_index_parallel(
    vault: &Vault,
    embedder: &dyn Embedder,
    extractors: &ExtractorRegistry,
    opts: &IndexOptions,
    rows: &[crate::registry::FileRow],
    concurrency: usize,
    progress: Option<&dyn Fn(usize, usize, &str)>,
    summary: &mut IndexSummary,
    results: &mut Vec<FileResult>,
) -> Result<()> {
    use crossbeam_channel::{bounded, Receiver, Sender};

    struct Done {
        task: reconcile::ExtractTask,
        extracted: reconcile::Extracted,
    }

    let total = rows.len();
    let chunking = vault.config.chunking.clone();

    std::thread::scope(|s| -> Result<()> {
        let (work_tx, work_rx): (
            Sender<reconcile::ExtractTask>,
            Receiver<reconcile::ExtractTask>,
        ) = bounded(concurrency * 2);
        let (done_tx, done_rx): (Sender<Done>, Receiver<Done>) = bounded(concurrency * 2);

        // Extract workers — borrow extractors and chunking via thread::scope.
        for _ in 0..concurrency {
            let work_rx = work_rx.clone();
            let done_tx = done_tx.clone();
            let chunking = &chunking;
            s.spawn(move || {
                while let Ok(task) = work_rx.recv() {
                    let extracted = reconcile::do_extract(extractors, chunking, &task);
                    if done_tx.send(Done { task, extracted }).is_err() {
                        break;
                    }
                }
            });
        }
        // Drop the extras held by the main scope so workers exit when we drop
        // our own work_tx.
        drop(work_rx);
        drop(done_tx);

        let mut row_iter = rows.iter().enumerate();
        let mut in_flight: usize = 0;
        let mut processed: usize = 0;

        loop {
            // Dispatch as much work as the channel will accept.
            while in_flight < concurrency * 2 {
                let Some((_, row)) = row_iter.next() else {
                    break;
                };
                match reconcile::precheck(vault, row, opts)? {
                    reconcile::Precheck::Resolved(result) => {
                        if let Some(p) = progress {
                            p(processed, total, &result.path);
                        }
                        result.outcome.tally(summary);
                        results.push(result);
                        processed += 1;
                    }
                    reconcile::Precheck::NeedsExtraction(task) => {
                        // If sending fails, all workers died; that's a panic
                        // condition.
                        work_tx
                            .send(task)
                            .map_err(|e| Error::other(format!("dispatch: {e}")))?;
                        in_flight += 1;
                    }
                }
            }

            if in_flight == 0 {
                break;
            }

            // Block on the next worker result.
            let Done { task, extracted } = match done_rx.recv() {
                Ok(d) => d,
                Err(_) => {
                    return Err(Error::other(
                        "extract worker pool died with in-flight tasks",
                    ));
                }
            };
            in_flight -= 1;

            if let Some(p) = progress {
                p(processed, total, &task.row.path);
            }
            let result = reconcile::finalize(vault, embedder, task, extracted)?;
            result.outcome.tally(summary);
            results.push(result);
            processed += 1;
        }

        drop(work_tx); // signal workers to exit
        Ok(())
    })?;

    Ok(())
}
