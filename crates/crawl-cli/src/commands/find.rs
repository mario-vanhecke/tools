use crate::cli::FindCmd;
use crate::commands::{open_vault, resolve_source_id};
use crate::output::{emit_json, print_documents};
use crawl_core::registry::{query_documents, source_name_map, DocQuery};
use crawl_core::DocStatus;
use std::path::Path;

pub fn run(cmd: FindCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let source_id = resolve_source_id(&vault, cmd.source.as_deref())?;
    let q = DocQuery {
        status: match cmd.status.as_deref() {
            Some(s) => Some(DocStatus::from_str(s)?),
            None => None,
        },
        source_id,
        extension: cmd.ext.clone(),
        name_like: Some(cmd.query.clone()),
        limit: Some(cmd.limit),
    };
    let rows = query_documents(&vault.conn, &q)?;
    if json {
        emit_json(&rows)?;
    } else {
        let names = source_name_map(&vault.conn)?;
        print_documents(&rows, &names);
        if rows.len() == cmd.limit {
            println!("\n(showing first {}; raise --limit for more)", cmd.limit);
        }
    }
    Ok(0)
}
