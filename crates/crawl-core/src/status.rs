use crate::error::{Error, Result};
use crate::registry::sources;
use crate::vault::CrawlVault;
use serde::{Deserialize, Serialize};

/// Lifecycle state of a discovered document. Only `crawl run` drives
/// transitions; `crawl rm`/`crawl prune` only delete rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocStatus {
    /// Seen in the most recent crawl of its source; unchanged or first sighting.
    Present,
    /// Seen in the most recent crawl, but size/mtime/hash differ from last time.
    /// Transient — resolves back to `present` on the next clean crawl.
    Modified,
    /// Previously discovered, absent from the most recent crawl of its source.
    Gone,
    /// Found, but larger than `documents.size_cap_bytes`; recorded but not hashed.
    TooLarge,
    /// The crawler could not read or stat this item.
    Error,
}

impl DocStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Modified => "modified",
            Self::Gone => "gone",
            Self::TooLarge => "too_large",
            Self::Error => "error",
        }
    }
    pub fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "present" => Self::Present,
            "modified" => Self::Modified,
            "gone" => Self::Gone,
            "too_large" => Self::TooLarge,
            "error" => Self::Error,
            other => return Err(Error::other(format!("unknown document status: {other}"))),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusReport {
    pub vault: VaultBlock,
    pub summary: SummaryBlock,
    pub sources: Vec<SourceStatusBlock>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultBlock {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SummaryBlock {
    pub sources: u32,
    pub sources_enabled: u32,
    pub documents: u32,
    pub present: u32,
    pub modified: u32,
    pub gone: u32,
    pub too_large: u32,
    pub error: u32,
    /// Documents whose `last_run_id` equals the source's most-recent run and
    /// whose `first_run_id` is that same run — i.e. brand new last crawl.
    pub new_last_run: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceStatusBlock {
    pub name: String,
    pub kind: String,
    pub uri: String,
    pub strategy: String,
    pub enabled: bool,
    pub documents: u32,
    pub last_crawled: Option<i64>,
    pub last_status: Option<String>,
}

/// Build the vault-wide status report: per-source counts plus a roll-up.
pub fn compute(vault: &CrawlVault) -> Result<StatusReport> {
    let mut report = StatusReport::default();
    report.vault.path = vault.root.to_string_lossy().to_string();
    report.vault.name = vault.name();

    let by_status = crate::registry::count_by_status(&vault.conn)?;
    let count = |s: DocStatus| *by_status.get(&s).unwrap_or(&0) as u32;
    report.summary.present = count(DocStatus::Present);
    report.summary.modified = count(DocStatus::Modified);
    report.summary.gone = count(DocStatus::Gone);
    report.summary.too_large = count(DocStatus::TooLarge);
    report.summary.error = count(DocStatus::Error);
    report.summary.documents = by_status.values().sum::<i64>() as u32;

    // New since each source's most recent run: first sighting was that run.
    report.summary.new_last_run = vault
        .conn
        .query_row(
            "SELECT COUNT(*) FROM documents d JOIN sources s ON d.source_id = s.id
             WHERE s.last_run_id IS NOT NULL AND d.first_run_id = s.last_run_id",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as u32;

    let srcs = sources::list_sources(&vault.conn)?;
    report.summary.sources = srcs.len() as u32;
    report.summary.sources_enabled = srcs.iter().filter(|s| s.enabled).count() as u32;
    for s in &srcs {
        let docs: i64 = vault
            .conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE source_id = ?1",
                [s.id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        report.sources.push(SourceStatusBlock {
            name: s.name.clone(),
            kind: s.kind.as_str().to_string(),
            uri: s.uri.clone(),
            strategy: s.strategy.as_str().to_string(),
            enabled: s.enabled,
            documents: docs as u32,
            last_crawled: s.last_crawled,
            last_status: s.last_status.clone(),
        });
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trip() {
        for s in [
            DocStatus::Present,
            DocStatus::Modified,
            DocStatus::Gone,
            DocStatus::TooLarge,
            DocStatus::Error,
        ] {
            assert_eq!(DocStatus::from_str(s.as_str()).unwrap(), s);
        }
    }
}
