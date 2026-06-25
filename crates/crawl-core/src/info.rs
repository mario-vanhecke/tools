use crate::error::Result;
use crate::vault::CrawlVault;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoReport {
    pub path: String,
    pub vault_id: String,
    pub name: String,
    pub created_at: i64,
    pub schema_version: u32,
    pub tool_version: String,
    pub counts: CountsBlock,
    pub size_bytes: u64,
    pub last_crawled_at: Option<i64>,
    pub checks: Option<Vec<Check>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountsBlock {
    pub sources: u32,
    pub documents: u32,
    pub runs: u32,
    pub present: u32,
    pub gone: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: Option<String>,
}

pub fn compute(vault: &CrawlVault, run_checks: bool) -> Result<InfoReport> {
    let (vault_id, created_at, tool_version) = vault.meta()?;
    let schema_version = vault.schema_version()?;

    let counts = CountsBlock {
        sources: scalar(vault, "SELECT COUNT(*) FROM sources")?,
        documents: scalar(vault, "SELECT COUNT(*) FROM documents")?,
        runs: scalar(vault, "SELECT COUNT(*) FROM runs")?,
        present: scalar(
            vault,
            "SELECT COUNT(*) FROM documents WHERE status IN ('present','modified')",
        )?,
        gone: scalar(vault, "SELECT COUNT(*) FROM documents WHERE status='gone'")?,
    };

    let last_crawled_at: Option<i64> = vault
        .conn
        .query_row("SELECT MAX(last_crawled) FROM sources", [], |r| r.get(0))
        .unwrap_or(None);

    let size_bytes = std::fs::metadata(&vault.db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let checks = if run_checks {
        Some(run_consistency_checks(vault)?)
    } else {
        None
    };

    Ok(InfoReport {
        path: vault.root.to_string_lossy().to_string(),
        vault_id,
        name: vault.name(),
        created_at,
        schema_version,
        tool_version,
        counts,
        size_bytes,
        last_crawled_at,
        checks,
    })
}

fn run_consistency_checks(vault: &CrawlVault) -> Result<Vec<Check>> {
    let mut checks = Vec::new();

    // Every document belongs to an existing source (FK should guarantee this).
    let orphans: i64 = vault.conn.query_row(
        "SELECT COUNT(*) FROM documents d LEFT JOIN sources s ON d.source_id = s.id
         WHERE s.id IS NULL",
        [],
        |r| r.get(0),
    )?;
    checks.push(Check {
        name: "no_orphan_documents".into(),
        ok: orphans == 0,
        detail: (orphans != 0).then(|| format!("{orphans} documents reference a missing source")),
    });

    // Every document status is a value the tool understands.
    let bad_status: i64 = vault.conn.query_row(
        "SELECT COUNT(*) FROM documents
         WHERE status NOT IN ('present','modified','gone','too_large','error')",
        [],
        |r| r.get(0),
    )?;
    checks.push(Check {
        name: "valid_statuses".into(),
        ok: bad_status == 0,
        detail: (bad_status != 0).then(|| format!("{bad_status} documents have an unknown status")),
    });

    // No two documents share a (source, uri) — enforced by a UNIQUE index.
    let dupes: i64 = vault.conn.query_row(
        "SELECT COUNT(*) FROM (SELECT source_id, uri FROM documents
         GROUP BY source_id, uri HAVING COUNT(*) > 1)",
        [],
        |r| r.get(0),
    )?;
    checks.push(Check {
        name: "unique_uri_per_source".into(),
        ok: dupes == 0,
        detail: (dupes != 0).then(|| format!("{dupes} duplicate (source, uri) pairs")),
    });

    Ok(checks)
}

fn scalar(vault: &CrawlVault, sql: &str) -> Result<u32> {
    let n: i64 = vault.conn.query_row(sql, [], |r| r.get(0))?;
    Ok(n as u32)
}
