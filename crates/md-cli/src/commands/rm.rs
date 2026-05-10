use crate::cli::RmCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use md_core::registry::{remove_paths, rm::remove_all};
use serde_json::json;
use std::path::Path;

pub fn run(cmd: RmCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    if cmd.all {
        if !cmd.yes {
            anyhow::bail!("md rm --all requires --yes (refusing to deregister every file without confirmation)");
        }
        let n = remove_all(&vault)?;
        if json {
            emit_json(&json!({"removed": n, "all": true}))?;
        } else {
            println!("Removed {} files (all).", n);
        }
        return Ok(0);
    }

    let mut rels: Vec<String> = Vec::with_capacity(cmd.paths.len());
    for p in &cmd.paths {
        let rel = vault.relativize(p)?;
        rels.push(rel.to_string_lossy().to_string());
    }
    let report = remove_paths(&vault, &rels)?;
    if json {
        emit_json(&report)?;
    } else {
        println!("Removed {} file(s).", report.removed);
        if report.not_found > 0 {
            println!("Not in registry: {}", report.not_found);
        }
    }
    Ok(0)
}
