use crate::cli::StatusCmd;
use crate::commands::open_vault;
use crate::output::{emit_json, fmt_time};
use crawl_core::status::compute;
use std::path::Path;

pub fn run(_cmd: StatusCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let report = compute(&vault)?;

    if json {
        emit_json(&report)?;
    } else {
        println!("Vault: {} ({})", report.vault.path, report.vault.name);
        let s = &report.summary;
        println!(
            "Sources: {} ({} enabled)   Documents: {}",
            s.sources, s.sources_enabled, s.documents
        );
        println!(
            "Present: {:<8} Modified: {:<6} Gone: {:<6} TooLarge: {:<5} Error: {}",
            s.present, s.modified, s.gone, s.too_large, s.error
        );
        println!("New since last run: {}", s.new_last_run);
        if !report.sources.is_empty() {
            println!();
            let (h_src, h_kind, h_strat, h_docs, h_when, h_stat) = (
                "SOURCE",
                "KIND",
                "STRATEGY",
                "DOCS",
                "LAST CRAWLED",
                "STATUS",
            );
            println!("{h_src:<18} {h_kind:<11} {h_strat:<12} {h_docs:<8} {h_when:<16} {h_stat}");
            for src in &report.sources {
                let name = if src.enabled {
                    src.name.clone()
                } else {
                    format!("{} (off)", src.name)
                };
                println!(
                    "{:<18} {:<11} {:<12} {:<8} {:<16} {}",
                    name,
                    src.kind,
                    src.strategy,
                    src.documents,
                    fmt_time(src.last_crawled),
                    src.last_status.as_deref().unwrap_or("-")
                );
            }
        }
    }
    Ok(0)
}
