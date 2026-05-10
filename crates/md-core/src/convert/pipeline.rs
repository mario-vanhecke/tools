use crate::error::Result;
use crate::registry::OutputRow;
use crate::status::FileStatus;
use crate::vault::MdVault;
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let bytes = h.finalize();
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Status-only update for non-converted outcomes (failed, missing,
/// unsupported, ...). Single transaction.
pub fn write_status_only(
    vault: &MdVault,
    row: &OutputRow,
    status: FileStatus,
    detail: Option<&str>,
    note: Option<&str>,
    bump_attempts: bool,
) -> Result<()> {
    let tx = vault.conn.unchecked_transaction()?;
    let now = chrono::Utc::now().timestamp_millis();
    let attempts = if bump_attempts || matches!(status, FileStatus::Failed) {
        row.attempts + 1
    } else {
        row.attempts
    };
    tx.execute(
        "UPDATE outputs SET
            status = ?1,
            status_detail = ?2,
            status_note = ?3,
            attempts = ?4,
            last_attempt = ?5
         WHERE id = ?6",
        params![status.as_str(), detail, note, attempts, now, row.id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Successful-conversion write: persist the output file to disk, record the
/// hashes/output_path/extractor in the DB. Mirrors rag-core's
/// `write_indexed_content`.
///
/// The output file write happens BEFORE the DB transaction. If the process
/// dies between writing the file and committing the DB row, on the next run
/// we'll see a stale `pending` row whose target file already exists; the
/// reconcile pass detects this via hash check and re-converts (the source
/// hash drives the decision, not the output's existence).
#[allow(clippy::too_many_arguments)]
pub fn write_converted(
    vault: &MdVault,
    row: &OutputRow,
    output_rel_path: &str,
    output_abs_path: &Path,
    output_bytes: &[u8],
    src_mtime_ms: Option<i64>,
    src_size: i64,
    src_hash: &str,
    extractor: &str,
) -> Result<()> {
    if let Some(parent) = output_abs_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Atomic write: write to a temp file, rename. Avoids leaving a partial
    // file if we crash mid-write.
    let tmp_path = output_abs_path.with_extension("md.tmp");
    std::fs::write(&tmp_path, output_bytes)?;
    std::fs::rename(&tmp_path, output_abs_path)?;

    let out_hash = sha256_hex(output_bytes);
    let now = chrono::Utc::now().timestamp_millis();

    let tx = vault.conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE outputs SET
            status = 'converted',
            status_detail = NULL,
            status_note = NULL,
            output_path = ?1,
            last_src_mtime = ?2,
            last_src_size = ?3,
            last_src_hash = ?4,
            last_out_hash = ?5,
            last_converted = ?6,
            extractor = ?7,
            attempts = 0,
            last_attempt = ?6
         WHERE id = ?8",
        params![
            output_rel_path,
            src_mtime_ms,
            src_size,
            src_hash,
            out_hash,
            now,
            extractor,
            row.id,
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Mark a row as `conflict` (source changed AND output hand-edited) without
/// touching the user's output file. Optionally also write a `<output>.new`
/// side-by-side that contains the freshly converted content the user can
/// `diff` against.
pub fn write_conflict(
    vault: &MdVault,
    row: &OutputRow,
    new_output_abs_path: Option<&Path>,
    new_output_bytes: Option<&[u8]>,
    note: &str,
) -> Result<()> {
    if let (Some(p), Some(b)) = (new_output_abs_path, new_output_bytes) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, b)?;
    }
    let tx = vault.conn.unchecked_transaction()?;
    let now = chrono::Utc::now().timestamp_millis();
    tx.execute(
        "UPDATE outputs SET
            status = 'conflict',
            status_detail = ?1,
            status_note = ?2,
            last_attempt = ?3
         WHERE id = ?4",
        params!["output_modified_with_source_change", note, now, row.id],
    )?;
    tx.commit()?;
    Ok(())
}
