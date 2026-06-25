use crate::cli::InfoCmd;
use crate::commands::open_vault;
use crate::output::{emit_json, fmt_time};
use crawl_core::info::compute;
use std::path::Path;

pub fn run(cmd: InfoCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let report = compute(&vault, cmd.check)?;

    if json {
        emit_json(&report)?;
    } else {
        println!("Vault:          {} ({})", report.path, report.name);
        println!("vault_id:       {}", report.vault_id);
        println!("created:        {}", fmt_time(Some(report.created_at)));
        println!("schema_version: {}", report.schema_version);
        println!("tool_version:   {}", report.tool_version);
        println!("db size:        {} bytes", report.size_bytes);
        println!("last crawled:   {}", fmt_time(report.last_crawled_at));
        let c = &report.counts;
        println!(
            "sources: {}   documents: {}   runs: {}   present: {}   gone: {}",
            c.sources, c.documents, c.runs, c.present, c.gone
        );
        if let Some(checks) = &report.checks {
            println!("\nConsistency checks:");
            for chk in checks {
                let mark = if chk.ok { "ok  " } else { "FAIL" };
                print!("  [{}] {}", mark, chk.name);
                if let Some(d) = &chk.detail {
                    print!(" — {d}");
                }
                println!();
            }
        }
    }

    // Non-zero exit if any consistency check failed.
    let failed = report
        .checks
        .as_ref()
        .map(|cs| cs.iter().any(|c| !c.ok))
        .unwrap_or(false);
    Ok(if failed { 1 } else { 0 })
}
