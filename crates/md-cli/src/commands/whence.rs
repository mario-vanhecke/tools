use crate::cli::WhenceCmd;
use crate::commands::open_vault;
use crate::output::emit_json;
use md_core::whence::whence;
use std::path::Path;

pub fn run(cmd: WhenceCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg).ok();
    let result = whence(vault.as_ref(), &cmd.path)?;

    match result {
        Some(r) => {
            if json {
                emit_json(&r)?;
            } else {
                println!("source:        {}", r.source);
                if let Some(h) = &r.source_hash {
                    println!("source_hash:   {}", h);
                }
                if let Some(e) = &r.extractor {
                    println!("extractor:     {}", e);
                }
                if let Some(t) = r.converted_at_ms {
                    let dt = chrono::DateTime::from_timestamp_millis(t)
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_else(|| t.to_string());
                    println!("converted_at:  {}", dt);
                }
                if let Some(v) = &r.vault_root {
                    println!("vault:         {}", v.display());
                }
                println!("via:           {}", r.via);
            }
            Ok(0)
        }
        None => {
            if json {
                emit_json(
                    &serde_json::json!({"error": "no_source_found", "path": cmd.path.to_string_lossy()}),
                )?;
            } else {
                anyhow::bail!(
                    "could not determine source: no DB entry and no annotation in {}",
                    cmd.path.display()
                );
            }
            Ok(1)
        }
    }
}
