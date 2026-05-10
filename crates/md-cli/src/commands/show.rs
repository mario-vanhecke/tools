use crate::cli::ShowCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use md_core::registry;
use serde_json::json;
use std::path::Path;

pub fn run(cmd: ShowCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;

    // Try input-path lookup first; if found, print the converted file
    // (or report that nothing has been converted yet).
    if let Some(row) = registry::find_by_input_path(&vault.conn, &cmd.target)? {
        if json {
            emit_json(&row)?;
            return Ok(0);
        }
        println!("Input:    {}", row.input_path);
        println!("Status:   {}", row.status.as_str());
        if let Some(out) = &row.output_path {
            let out_abs = vault.output_dir_abs().join(out);
            println!("Output:   {}", out_abs.display());
            if let Ok(text) = std::fs::read_to_string(&out_abs) {
                println!("---");
                println!("{}", text);
            }
        }
        return Ok(0);
    }

    // Otherwise treat as an output-path lookup (relative to output_dir).
    if let Some(row) = registry::find_by_output_path(&vault.conn, &cmd.target)? {
        if json {
            emit_json(&row)?;
        } else {
            let out_abs = vault.output_dir_abs().join(&cmd.target);
            println!("Source:   {}", row.input_path);
            println!("Output:   {}", out_abs.display());
            if let Ok(text) = std::fs::read_to_string(&out_abs) {
                println!("---");
                println!("{}", text);
            }
        }
        return Ok(0);
    }

    if json {
        emit_json(&json!({"error": "not_found", "target": cmd.target}))?;
    } else {
        anyhow::bail!(
            "not found: no row for input or output path '{}'",
            cmd.target
        );
    }
    Ok(1)
}
