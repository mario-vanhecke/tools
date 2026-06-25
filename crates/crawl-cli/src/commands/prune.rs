use crate::cli::PruneCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use crawl_core::registry::{prune, PruneOptions};
use crawl_core::DocStatus;
use std::path::Path;

pub fn run(cmd: PruneCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let opts = PruneOptions {
        status: match cmd.status.as_deref() {
            Some(s) => Some(DocStatus::from_str(s)?),
            None => None,
        },
        dry_run: cmd.dry_run,
    };
    let report = prune(&vault.conn, &opts)?;
    if json {
        emit_json(&report)?;
    } else {
        println!(
            "{} {} document(s) in status '{}'.",
            if report.dry_run {
                "Would prune"
            } else {
                "Pruned"
            },
            report.pruned,
            report.status
        );
    }
    Ok(0)
}
