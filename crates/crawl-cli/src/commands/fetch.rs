use crate::cli::FetchCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use crawl_core::crawl::{fetch, FetchOptions};
use crawl_core::vault_core::{acquire_lock, LockOptions};
use crawl_core::DocStatus;
use std::path::Path;

pub fn run(cmd: FetchCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let status = match cmd.status.as_deref() {
        Some(s) => Some(DocStatus::from_str(s)?),
        None => None,
    };
    let opts = FetchOptions {
        out_dir: cmd.out.clone(),
        source: cmd.source.clone(),
        extension: cmd.ext.clone(),
        status,
        force: cmd.force,
    };

    // Serialize against other crawl passes on the same vault.
    let _lock = acquire_lock(
        &vault.crawl_lock_path(),
        &LockOptions {
            no_wait: cmd.no_wait,
            wait_seconds: Some(cmd.wait),
        },
    )?;

    let report = fetch::run(&vault, &opts)?;

    if json {
        emit_json(&report)?;
    } else {
        let (mut f, mut s, mut e, mut b) = (0u32, 0u32, 0u32, 0u64);
        for r in &report.sources {
            println!(
                "{:<18} {:<11} fetched {}  up-to-date {}  errors {}",
                r.source, r.kind, r.fetched, r.skipped, r.errors
            );
            f += r.fetched;
            s += r.skipped;
            e += r.errors;
            b += r.bytes;
        }
        println!(
            "\nMaterialized {} file(s) into {}/ ({}, {} up-to-date, {} errors)",
            f,
            report.out_dir,
            human_bytes(b),
            s,
            e
        );
        if f > 0 {
            println!(
                "Feed it onward: `md add {}` or `rag add {}`",
                report.out_dir, report.out_dir
            );
        }
    }

    let any_err = report.sources.iter().any(|r| r.errors > 0);
    Ok(if any_err { 1 } else { 0 })
}

fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1}{}", UNITS[u])
    }
}
