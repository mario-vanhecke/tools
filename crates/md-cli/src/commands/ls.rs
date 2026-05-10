use crate::cli::LsCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use md_core::registry;
use md_core::status::FileStatus;
use serde_json::json;
use std::path::Path;

pub fn run(cmd: LsCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let status = match cmd.status.as_deref() {
        Some(s) => Some(FileStatus::from_str(s)?),
        None => None,
    };
    let rows = registry::list_filtered(&vault.conn, status)?;
    if json {
        emit_json(&json!({"files": rows}))?;
    } else {
        for row in &rows {
            let out = row.output_path.as_deref().unwrap_or("-");
            println!("{:<12} {}  →  {}", row.status.as_str(), row.input_path, out);
        }
    }
    Ok(0)
}
