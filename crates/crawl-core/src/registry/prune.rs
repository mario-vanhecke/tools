use crate::error::Result;
use crate::status::DocStatus;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct PruneOptions {
    /// Which status to prune. Defaults to `gone` when unset.
    pub status: Option<DocStatus>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PruneReport {
    pub status: String,
    pub pruned: u32,
    pub dry_run: bool,
}

/// Delete document rows in a terminal status (default: `gone`). Sources and
/// their `present`/`modified` documents are untouched.
pub fn prune(conn: &Connection, opts: &PruneOptions) -> Result<PruneReport> {
    let status = opts.status.unwrap_or(DocStatus::Gone);
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM documents WHERE status = ?1",
        params![status.as_str()],
        |r| r.get(0),
    )?;
    if !opts.dry_run {
        conn.execute(
            "DELETE FROM documents WHERE status = ?1",
            params![status.as_str()],
        )?;
    }
    Ok(PruneReport {
        status: status.as_str().to_string(),
        pruned: count as u32,
        dry_run: opts.dry_run,
    })
}
