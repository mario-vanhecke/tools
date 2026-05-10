use crate::cli::InitCmd;
use crate::output::emit_json;
use md_core::MdVault;
use serde_json::json;

pub fn run(cmd: InitCmd, json: bool) -> anyhow::Result<i32> {
    let dir = cmd
        .directory
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    let vault = MdVault::init(&dir, cmd.force)?;
    let (vault_id, _, tool_version) = vault.meta()?;
    let schema_version = vault.schema_version()?;
    if json {
        emit_json(&json!({
            "vault_path": vault.root.to_string_lossy(),
            "vault_id": vault_id,
            "schema_version": schema_version,
            "tool_version": tool_version,
            "output_dir": vault.output_dir_abs().to_string_lossy(),
        }))?;
    } else {
        println!("Created md vault at {}", vault.root.display());
        println!("  vault_id:       {}", vault_id);
        println!("  schema_version: {}", schema_version);
        println!("  tool_version:   {}", tool_version);
        println!("  output_dir:     {}", vault.output_dir_abs().display());
    }
    Ok(0)
}
