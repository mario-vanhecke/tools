use crate::cli::InitCmd;
use crate::output::emit_json;
use crawl_core::CrawlVault;
use serde_json::json;

pub fn run(cmd: InitCmd, json: bool) -> anyhow::Result<i32> {
    let dir = cmd
        .directory
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    let vault = CrawlVault::init(&dir, cmd.force)?;
    let (vault_id, _, tool_version) = vault.meta()?;
    let schema_version = vault.schema_version()?;
    if json {
        emit_json(&json!({
            "vault_path": vault.root.to_string_lossy(),
            "vault_id": vault_id,
            "schema_version": schema_version,
            "tool_version": tool_version,
        }))?;
    } else {
        println!("Created crawl vault at {}", vault.root.display());
        println!("  vault_id:       {}", vault_id);
        println!("  schema_version: {}", schema_version);
        println!("  tool_version:   {}", tool_version);
        println!();
        println!("Next: register a source, e.g.");
        println!("  crawl source add my-docs local ./docs");
        println!("  crawl run");
    }
    Ok(0)
}
