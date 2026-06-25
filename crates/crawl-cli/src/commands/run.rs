use crate::cli::RunCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use crawl_core::crawl::{self, RunOptions};
use crawl_core::source::Strategy;
use crawl_core::vault_core::{acquire_lock, LockOptions};
use std::path::Path;

pub fn run(cmd: RunCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;

    // Forget cached SharePoint tokens so the next sign-in requests fresh scopes.
    if cmd.reauth {
        let cleared = clear_token_cache(&vault.state_dir)?;
        if !json && cleared > 0 {
            println!("Cleared {cleared} cached SharePoint token(s); will sign in again.");
        }
    }

    let strategy_override = match &cmd.strategy {
        Some(s) => Some(Strategy::from_str(s)?),
        None => None,
    };
    let hash = if cmd.hash {
        Some(true)
    } else if cmd.no_hash {
        Some(false)
    } else {
        None
    };

    let opts = RunOptions {
        source: cmd.source.clone(),
        strategy_override,
        hash,
        dry_run: cmd.dry_run,
        include_disabled: cmd.all,
    };

    // Serialize crawl passes against the same vault (skip on dry-run: no writes).
    let _lock = if cmd.dry_run {
        None
    } else {
        Some(acquire_lock(
            &vault.crawl_lock_path(),
            &LockOptions {
                no_wait: cmd.no_wait,
                wait_seconds: Some(cmd.wait),
            },
        )?)
    };

    let report = crawl::run(&vault, &opts)?;

    if json {
        emit_json(&report)?;
    } else {
        if report.sources.is_empty() {
            println!("No sources to crawl. Add one with `crawl source add ...`.");
            return Ok(0);
        }
        if report.dry_run {
            println!("Dry run — no changes written.\n");
        }
        let (mut d, mut u, mut g, mut s, mut e) = (0u32, 0u32, 0u32, 0u32, 0u32);
        for r in &report.sources {
            let marker = match r.status.as_str() {
                "ok" => "",
                "partial" => "  [partial]",
                _ => "  [ERROR]",
            };
            println!(
                "{:<18} {:<11} {:<12} +{} ~{} -{} (skipped {}, errors {}){}",
                r.source,
                r.kind,
                r.strategy,
                r.discovered,
                r.updated,
                r.gone,
                r.skipped,
                r.errors,
                marker
            );
            if let Some(note) = &r.note {
                println!("    {note}");
            }
            d += r.discovered;
            u += r.updated;
            g += r.gone;
            s += r.skipped;
            e += r.errors;
        }
        println!(
            "\nTotal: {} discovered, {} updated, {} gone, {} skipped, {} errors",
            d, u, g, s, e
        );
    }

    // Exit non-zero if any source hard-errored, so scripts notice.
    let any_error = report.sources.iter().any(|r| r.status == "error");
    Ok(if any_error { 1 } else { 0 })
}

/// Delete cached SharePoint auth tokens (`sharepoint-*.token.json`) from the
/// vault state dir. Returns how many were removed.
fn clear_token_cache(state_dir: &Path) -> anyhow::Result<u32> {
    let mut n = 0;
    if let Ok(entries) = std::fs::read_dir(state_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("sharepoint-")
                && name.ends_with(".token.json")
                && std::fs::remove_file(entry.path()).is_ok()
            {
                n += 1;
            }
        }
    }
    Ok(n)
}
