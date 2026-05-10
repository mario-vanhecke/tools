use crate::cli::AddCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use md_core::registry::{add_paths, AddOptions};
use std::path::Path;

pub fn run(cmd: AddCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let opts = AddOptions {
        skip_unsupported: cmd.skip_unsupported,
        no_ignore: cmd.no_ignore,
        force: cmd.force,
        dry_run: cmd.dry_run,
    };
    let report = add_paths(&vault, &cmd.paths, &opts)?;
    if json {
        emit_json(&report)?;
    } else {
        println!(
            "Registered {} files{}.",
            report.added,
            if cmd.dry_run { " (dry run)" } else { "" }
        );
        if report.already_registered > 0 {
            println!("Already registered: {}", report.already_registered);
        }
        if report.skipped_by_ignore > 0 {
            println!("Skipped by ignore:  {}", report.skipped_by_ignore);
        }
        if report.skipped_unsupported > 0 {
            println!("Skipped unsupported: {}", report.skipped_unsupported);
        }
        if report.added > 0 {
            println!("Run `md convert` to convert pending files.");
        }
    }
    Ok(0)
}
