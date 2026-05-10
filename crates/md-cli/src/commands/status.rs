use crate::cli::StatusCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use md_core::status::{compute, FileStatus, StatusOptions};
use std::path::Path;

pub fn run(cmd: StatusCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let opts = StatusOptions {
        filter: match cmd.filter.as_deref() {
            Some(s) => Some(FileStatus::from_str(s)?),
            None => None,
        },
        no_stat: cmd.no_stat,
    };
    let report = compute(&vault, &opts)?;

    if json {
        emit_json(&report)?;
    } else {
        println!("Vault:      {} ({})", report.vault.path, report.vault.name);
        println!("Output dir: {}", report.vault.output_dir);
        println!();
        let s = &report.summary;
        println!(
            "Registered: {:<6} Converted: {:<6} Pending: {}",
            s.registered, s.converted, s.pending
        );
        println!(
            "Reconvert: {:<6} OutputMod: {:<6} Conflict: {}",
            s.reconvert, s.output_modified, s.conflict
        );
        println!(
            "Failed: {:<10} NeedsOcr: {:<5} Unsupported: {}",
            s.failed, s.needs_ocr, s.unsupported
        );
        println!(
            "TooLarge: {:<8} Missing: {:<6} Excluded: {}",
            s.too_large, s.missing, s.excluded
        );
    }

    Ok(0)
}
