use crate::cli::RmCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use crawl_core::registry::remove_uris;
use std::path::Path;

pub fn run(cmd: RmCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let report = remove_uris(&vault.conn, &cmd.uris)?;
    if json {
        emit_json(&report)?;
    } else {
        println!("Removed {} document(s).", report.removed);
        if report.not_found > 0 {
            println!("Not found: {}", report.not_found);
        }
    }
    Ok(0)
}
