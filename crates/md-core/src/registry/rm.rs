use crate::error::Result;
use crate::vault::MdVault;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RmReport {
    pub removed: u32,
    pub not_found: u32,
    pub files: Vec<RmFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RmFile {
    pub path: String,
    pub removed: bool,
    pub reason: Option<String>,
}

pub fn remove_paths(vault: &MdVault, paths: &[String]) -> Result<RmReport> {
    let tx = vault.conn.unchecked_transaction()?;
    let mut report = RmReport::default();
    for p in paths {
        let n = tx.execute("DELETE FROM outputs WHERE input_path = ?1", params![p])?;
        if n > 0 {
            report.removed += 1;
            report.files.push(RmFile {
                path: p.clone(),
                removed: true,
                reason: None,
            });
        } else {
            report.not_found += 1;
            report.files.push(RmFile {
                path: p.clone(),
                removed: false,
                reason: Some("not_in_registry".to_string()),
            });
        }
    }
    tx.commit()?;
    Ok(report)
}

pub fn remove_all(vault: &MdVault) -> Result<u32> {
    let tx = vault.conn.unchecked_transaction()?;
    let n = tx.execute("DELETE FROM outputs", [])? as u32;
    tx.commit()?;
    Ok(n)
}
