use crate::error::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RmReport {
    pub removed: u32,
    pub not_found: u32,
    pub uris: Vec<RmEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RmEntry {
    pub uri: String,
    pub removed: bool,
}

/// Deregister documents by canonical URI (across all sources).
pub fn remove_uris(conn: &Connection, uris: &[String]) -> Result<RmReport> {
    let tx = conn.unchecked_transaction()?;
    let mut report = RmReport::default();
    for uri in uris {
        let n = tx.execute("DELETE FROM documents WHERE uri = ?1", params![uri])?;
        if n > 0 {
            report.removed += 1;
            report.uris.push(RmEntry {
                uri: uri.clone(),
                removed: true,
            });
        } else {
            report.not_found += 1;
            report.uris.push(RmEntry {
                uri: uri.clone(),
                removed: false,
            });
        }
    }
    tx.commit()?;
    Ok(report)
}
