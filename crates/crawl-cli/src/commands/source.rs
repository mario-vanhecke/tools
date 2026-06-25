use crate::cli::{SourceAction, SourceAddCmd, SourceCmd};
use crate::commands::open_vault;
use crate::output::{emit_json, fmt_time};
use crawl_core::registry::sources::{self, AddSourceOptions};
use crawl_core::source::{SourceKind, Strategy};
use serde_json::{json, Value};
use std::path::Path;

pub fn run(cmd: SourceCmd, json: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    match cmd.action {
        SourceAction::Add(c) => add(&vault, c, json),
        SourceAction::Ls => ls(&vault, json),
        SourceAction::Show { name } => show(&vault, &name, json),
        SourceAction::Rm { name } => rm(&vault, &name, json),
        SourceAction::Enable { name } => set_enabled(&vault, &name, true, json),
        SourceAction::Disable { name } => set_enabled(&vault, &name, false, json),
    }
}

fn add(vault: &crawl_core::CrawlVault, cmd: SourceAddCmd, json: bool) -> anyhow::Result<i32> {
    let kind = SourceKind::from_str(&cmd.kind)?;
    let strategy = match &cmd.strategy {
        Some(s) => Strategy::from_str(s)?,
        None => Strategy::from_str(&vault.config.crawl.default_strategy)?,
    };

    // Build the config object from --config JSON, then layer --set / convenience flags.
    let mut config: Value = match &cmd.config {
        Some(raw) => serde_json::from_str(raw)
            .map_err(|e| crawl_core::Error::config(format!("--config is not valid JSON: {e}")))?,
        None => json!({}),
    };
    if !config.is_object() {
        return Err(crawl_core::Error::config("--config must be a JSON object").into());
    }
    let obj = config.as_object_mut().unwrap();
    for pair in &cmd.set {
        let (k, v) = pair.split_once('=').ok_or_else(|| {
            crawl_core::Error::config(format!("--set expects KEY=VALUE, got '{pair}'"))
        })?;
        // Parse the value as JSON when possible (numbers, bools), else string.
        let val = serde_json::from_str::<Value>(v).unwrap_or_else(|_| Value::String(v.to_string()));
        obj.insert(k.to_string(), val);
    }
    if !cmd.include.is_empty() {
        obj.insert("include_globs".into(), json!(cmd.include));
    }
    if !cmd.exclude.is_empty() {
        obj.insert("exclude_globs".into(), json!(cmd.exclude));
    }
    if let Some(d) = cmd.max_depth {
        obj.insert("max_depth".into(), json!(d));
    }

    // For local sources, store an absolute path so the locator is stable.
    let uri = if kind == SourceKind::Local {
        Path::new(&cmd.uri)
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(cmd.uri)
    } else {
        cmd.uri
    };

    let opts = AddSourceOptions {
        name: cmd.name,
        kind,
        uri,
        strategy,
        config,
        enabled: !cmd.disabled,
    };
    let src = sources::add_source(&vault.conn, &opts)?;

    if json {
        emit_json(&src)?;
    } else {
        println!("Added source '{}' ({})", src.name, src.kind.as_str());
        println!("  uri:      {}", src.uri);
        println!("  strategy: {}", src.strategy.as_str());
        if src
            .config
            .as_object()
            .map(|o| !o.is_empty())
            .unwrap_or(false)
        {
            println!("  config:   {}", src.config);
        }
        if !src.enabled {
            println!("  (disabled)");
        }
        println!("Run `crawl run` to discover documents.");
    }
    Ok(0)
}

fn ls(vault: &crawl_core::CrawlVault, json: bool) -> anyhow::Result<i32> {
    let srcs = sources::list_sources(&vault.conn)?;
    if json {
        emit_json(&srcs)?;
    } else if srcs.is_empty() {
        println!("No sources. Add one with `crawl source add <name> <kind> <uri>`.");
    } else {
        let (h_name, h_kind, h_strat, h_docs, h_when, h_uri) =
            ("NAME", "KIND", "STRATEGY", "DOCS", "LAST CRAWLED", "URI");
        println!("{h_name:<18} {h_kind:<11} {h_strat:<12} {h_docs:<8} {h_when:<16} {h_uri}");
        for s in &srcs {
            let docs: i64 = vault
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM documents WHERE source_id = ?1",
                    [s.id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let name = if s.enabled {
                s.name.clone()
            } else {
                format!("{} (off)", s.name)
            };
            println!(
                "{:<18} {:<11} {:<12} {:<8} {:<16} {}",
                name,
                s.kind.as_str(),
                s.strategy.as_str(),
                docs,
                fmt_time(s.last_crawled),
                s.uri
            );
        }
    }
    Ok(0)
}

fn show(vault: &crawl_core::CrawlVault, name: &str, json: bool) -> anyhow::Result<i32> {
    let src = sources::get_source_by_name(&vault.conn, name)?
        .ok_or_else(|| crawl_core::Error::NoSuchSource(name.to_string()))?;
    if json {
        emit_json(&src)?;
    } else {
        println!("Source '{}'", src.name);
        println!("  kind:         {}", src.kind.as_str());
        println!("  uri:          {}", src.uri);
        println!("  strategy:     {}", src.strategy.as_str());
        println!("  enabled:      {}", src.enabled);
        println!("  config:       {}", src.config);
        println!("  last crawled: {}", fmt_time(src.last_crawled));
        println!(
            "  last status:  {}",
            src.last_status.as_deref().unwrap_or("-")
        );
        if let Some(e) = &src.last_error {
            println!("  last error:   {e}");
        }
    }
    Ok(0)
}

fn rm(vault: &crawl_core::CrawlVault, name: &str, json: bool) -> anyhow::Result<i32> {
    let removed = sources::remove_source(&vault.conn, name)?;
    if !removed {
        return Err(crawl_core::Error::NoSuchSource(name.to_string()).into());
    }
    if json {
        emit_json(&json!({"removed": name}))?;
    } else {
        println!("Removed source '{name}' and its discovered documents.");
    }
    Ok(0)
}

fn set_enabled(
    vault: &crawl_core::CrawlVault,
    name: &str,
    enabled: bool,
    json: bool,
) -> anyhow::Result<i32> {
    let ok = sources::set_enabled(&vault.conn, name, enabled)?;
    if !ok {
        return Err(crawl_core::Error::NoSuchSource(name.to_string()).into());
    }
    if json {
        emit_json(&json!({"source": name, "enabled": enabled}))?;
    } else {
        println!(
            "Source '{name}' {}.",
            if enabled { "enabled" } else { "disabled" }
        );
    }
    Ok(0)
}
