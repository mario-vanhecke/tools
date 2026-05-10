use crate::error::Result;
use crate::status::FileStatus;
use crate::vault::MdVault;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct PruneOptions {
    pub status: Option<FileStatus>,
    pub all_non_converted: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PruneReport {
    pub removed: u32,
    pub by_status: BTreeMap<String, u32>,
    pub files: Vec<PrunedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrunedFile {
    pub path: String,
    pub status_at_prune: String,
}

pub fn prune(vault: &MdVault, opts: &PruneOptions) -> Result<PruneReport> {
    let (clause, args): (&'static str, Vec<String>) = if opts.all_non_converted {
        ("status != 'converted'", vec![])
    } else if let Some(s) = opts.status {
        ("status = ?1", vec![s.as_str().to_string()])
    } else {
        ("status = 'missing'", vec![])
    };

    let select_sql =
        format!("SELECT input_path, status FROM outputs WHERE {clause} ORDER BY input_path");
    let mut stmt = vault.conn.prepare(&select_sql)?;
    let rows: Vec<(String, String)> = if args.is_empty() {
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?
    } else {
        stmt.query_map(params![args[0]], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?
    };

    let mut report = PruneReport::default();
    for (p, s) in &rows {
        report.removed += 1;
        *report.by_status.entry(s.clone()).or_insert(0) += 1;
        report.files.push(PrunedFile {
            path: p.clone(),
            status_at_prune: s.clone(),
        });
    }
    drop(stmt);

    if !opts.dry_run && report.removed > 0 {
        let delete_sql = format!("DELETE FROM outputs WHERE {clause}");
        let tx = vault.conn.unchecked_transaction()?;
        if args.is_empty() {
            tx.execute(&delete_sql, [])?;
        } else {
            tx.execute(&delete_sql, params![args[0]])?;
        }
        tx.commit()?;
    }
    Ok(report)
}
