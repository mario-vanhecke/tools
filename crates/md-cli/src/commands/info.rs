use crate::cli::InfoCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use md_core::info::compute;
use std::path::Path;

pub fn run(_cmd: InfoCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let report = compute(&vault)?;
    if json {
        emit_json(&report)?;
    } else {
        println!("Path:          {}", report.path);
        println!("Vault ID:      {}", report.vault_id);
        println!("Name:          {}", report.name);
        println!("Schema:        {}", report.schema_version);
        println!("Tool version:  {}", report.tool_version);
        println!("Output dir:    {}", report.output_dir);
        println!("Annotate:      {}", report.annotate);
        println!();
        let c = &report.counts;
        println!(
            "Counts: registered={} converted={} pending={} failed={} conflict={}",
            c.registered, c.converted, c.pending, c.failed, c.conflict
        );
        println!("Database size: {} bytes", report.size_bytes);
    }
    Ok(0)
}
