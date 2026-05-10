use crate::error::Result;
use crate::vault::MdVault;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoReport {
    pub path: String,
    pub vault_id: String,
    pub name: String,
    pub created_at: i64,
    pub schema_version: u32,
    pub tool_version: String,
    pub output_dir: String,
    pub annotate: bool,
    pub counts: CountsBlock,
    pub size_bytes: u64,
    pub last_converted_at: Option<i64>,
    pub last_added_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountsBlock {
    pub registered: u32,
    pub converted: u32,
    pub pending: u32,
    pub failed: u32,
    pub conflict: u32,
}

pub fn compute(vault: &MdVault) -> Result<InfoReport> {
    let (vault_id, created_at, tool_version) = vault.meta()?;
    let schema_version = vault.schema_version()?;
    let counts = CountsBlock {
        registered: scalar(vault, "SELECT COUNT(*) FROM outputs")?,
        converted: scalar(
            vault,
            "SELECT COUNT(*) FROM outputs WHERE status='converted'",
        )?,
        pending: scalar(vault, "SELECT COUNT(*) FROM outputs WHERE status='pending'")?,
        failed: scalar(vault, "SELECT COUNT(*) FROM outputs WHERE status='failed'")?,
        conflict: scalar(
            vault,
            "SELECT COUNT(*) FROM outputs WHERE status='conflict'",
        )?,
    };
    let last_converted_at: Option<i64> = vault
        .conn
        .query_row("SELECT MAX(last_converted) FROM outputs", [], |r| r.get(0))
        .unwrap_or(None);
    let last_added_at: Option<i64> = vault
        .conn
        .query_row("SELECT MAX(added_at) FROM outputs", [], |r| r.get(0))
        .unwrap_or(None);

    let size_bytes = std::fs::metadata(&vault.db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let name = if vault.config.vault_name.is_empty() {
        vault
            .root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        vault.config.vault_name.clone()
    };

    Ok(InfoReport {
        path: vault.root.to_string_lossy().to_string(),
        vault_id,
        name,
        created_at,
        schema_version,
        tool_version,
        output_dir: vault.output_dir_abs().to_string_lossy().to_string(),
        annotate: vault.config.output.annotate,
        counts,
        size_bytes,
        last_converted_at,
        last_added_at,
    })
}

fn scalar(vault: &MdVault, sql: &str) -> Result<u32> {
    let n: i64 = vault.conn.query_row(sql, [], |r| r.get(0))?;
    Ok(n as u32)
}
