use crate::cli::ExportCmd;
use crate::commands::{open_vault, resolve_source_id};
use crawl_core::registry::{query_documents, source_name_map, DocQuery, DocumentRow};
use crawl_core::DocStatus;
use std::path::Path;

pub fn run(cmd: ExportCmd, _json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    let source_id = resolve_source_id(&vault, cmd.source.as_deref())?;

    let status = match cmd.status.as_deref() {
        Some(s) => Some(DocStatus::from_str(s)?),
        None => None,
    };
    let q = DocQuery {
        status,
        source_id,
        extension: cmd.ext.clone(),
        name_like: None,
        limit: None,
    };
    let mut rows = query_documents(&vault.conn, &q)?;

    // By default export only live documents; --all or an explicit --status opts in.
    if status.is_none() && !cmd.all {
        rows.retain(|d| matches!(d.status, DocStatus::Present | DocStatus::Modified));
    }

    let body = match cmd.format.as_str() {
        "paths" => render_paths(&rows),
        "jsonl" => render_jsonl(&rows),
        "csv" => render_csv(&vault, &rows)?,
        other => {
            return Err(crawl_core::Error::config(format!(
                "unknown export format '{other}' (expected: paths, jsonl, csv)"
            ))
            .into())
        }
    };

    match &cmd.output {
        Some(path) => {
            std::fs::write(path, &body)?;
            eprintln!("Wrote {} document(s) to {}", rows.len(), path.display());
        }
        None => {
            print!("{body}");
        }
    }
    Ok(0)
}

fn render_paths(rows: &[DocumentRow]) -> String {
    let mut out = String::new();
    for d in rows {
        out.push_str(&d.uri);
        out.push('\n');
    }
    out
}

fn render_jsonl(rows: &[DocumentRow]) -> String {
    let mut out = String::new();
    for d in rows {
        if let Ok(line) = serde_json::to_string(d) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

fn render_csv(vault: &crawl_core::CrawlVault, rows: &[DocumentRow]) -> anyhow::Result<String> {
    let names = source_name_map(&vault.conn)?;
    let mut out = String::from("name,extension,size,modified_ms,status,source,uri\n");
    for d in rows {
        let source = names
            .get(&d.source_id)
            .cloned()
            .unwrap_or_else(|| d.source_id.to_string());
        let cells = [
            d.name.clone(),
            d.extension.clone().unwrap_or_default(),
            d.size.map(|s| s.to_string()).unwrap_or_default(),
            d.modified_ms.map(|m| m.to_string()).unwrap_or_default(),
            d.status.as_str().to_string(),
            source,
            d.uri.clone(),
        ];
        let line: Vec<String> = cells.iter().map(|c| csv_escape(c)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    Ok(out)
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
