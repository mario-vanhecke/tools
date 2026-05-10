use crate::cli::PruneCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use md_core::registry::{prune as do_prune, PruneOptions};
use md_core::status::FileStatus;
use std::path::Path;

pub fn run(cmd: PruneCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let status = match cmd.status.as_deref() {
        Some(s) => Some(FileStatus::from_str(s)?),
        None => None,
    };
    let opts = PruneOptions {
        status,
        all_non_converted: cmd.all_non_converted,
        dry_run: cmd.dry_run,
    };
    let report = do_prune(&vault, &opts)?;
    if json {
        emit_json(&report)?;
    } else {
        println!(
            "{} {} row(s).",
            if cmd.dry_run {
                "Would remove"
            } else {
                "Removed"
            },
            report.removed
        );
        for (status, n) in &report.by_status {
            println!("  {:<14} {}", status, n);
        }
    }
    Ok(0)
}
