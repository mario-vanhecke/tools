//! The crawl orchestrator. For each selected source it picks the matching
//! crawler, enumerates per the source's strategy, upserts every recorded
//! document, and marks anything no longer seen as `gone`. Each source is
//! processed in its own transaction so one unreachable source never corrupts
//! the others.

use super::{crawler_for, CrawlContext, DiscoveredItem, DocFilter};
use crate::error::Result;
use crate::registry::{self, sources, DocumentRow};
use crate::source::{Source, Strategy};
use crate::status::DocStatus;
use crate::vault::CrawlVault;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Only crawl the source with this name. `None` = every enabled source.
    pub source: Option<String>,
    /// Override each source's configured strategy for this run.
    pub strategy_override: Option<Strategy>,
    /// Override `crawl.hash`. `None` = use vault config.
    pub hash: Option<bool>,
    /// Enumerate and report, but write nothing.
    pub dry_run: bool,
    /// Crawl disabled sources too (normally skipped).
    pub include_disabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunReport {
    pub dry_run: bool,
    pub sources: Vec<SourceRunReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRunReport {
    pub source: String,
    pub kind: String,
    pub strategy: String,
    pub discovered: u32,
    pub updated: u32,
    pub gone: u32,
    pub skipped: u32,
    pub errors: u32,
    pub status: String, // ok | error | partial
    pub note: Option<String>,
}

pub fn run(vault: &CrawlVault, opts: &RunOptions) -> Result<RunReport> {
    let mut all = sources::list_sources(&vault.conn)?;
    if let Some(name) = &opts.source {
        all.retain(|s| &s.name == name);
        if all.is_empty() {
            return Err(crate::error::Error::NoSuchSource(name.clone()));
        }
    } else if !opts.include_disabled {
        all.retain(|s| s.enabled);
    }

    let mut report = RunReport {
        dry_run: opts.dry_run,
        sources: Vec::new(),
    };
    for src in &all {
        report.sources.push(run_one(vault, src, opts)?);
    }
    Ok(report)
}

fn run_one(vault: &CrawlVault, source: &Source, opts: &RunOptions) -> Result<SourceRunReport> {
    // A strategy override is realized by cloning the source with the new strategy.
    let mut effective = source.clone();
    if let Some(strat) = opts.strategy_override {
        effective.strategy = strat;
    }
    let params = effective.resolve_params(vault.config.crawl.default_max_depth as usize);
    // Only an exhaustive pass (no since-filter, no depth cap) may conclude that
    // unseen documents are gone. Incremental and shallow crawls see a subset,
    // so absence there means "not visited", not "deleted".
    let full_enumeration = params.since_ms.is_none() && params.max_depth.is_none();
    let filter = DocFilter::build(&vault.config, &params);
    let do_hash = opts.hash.unwrap_or(vault.config.crawl.hash);
    let size_cap = vault.config.documents.size_cap_bytes;
    let now = chrono::Utc::now().timestamp_millis();

    let mut rep = SourceRunReport {
        source: source.name.clone(),
        kind: source.kind.as_str().to_string(),
        strategy: effective.strategy.as_str().to_string(),
        discovered: 0,
        updated: 0,
        gone: 0,
        skipped: 0,
        errors: 0,
        status: "ok".to_string(),
        note: None,
    };

    // Enumerate first; decouples the crawler from DB writes and keeps the
    // trait simple. Trades memory for clarity — fine for a metadata registry.
    let crawler = crawler_for(source.kind);
    let ctx = CrawlContext {
        params,
        config: &vault.config,
        cache_dir: &vault.state_dir,
    };
    let mut items: Vec<DiscoveredItem> = Vec::new();
    let crawl_result = crawler.crawl(&effective, &ctx, &mut |item| items.push(item));

    let (stats, hard_error) = match crawl_result {
        Ok(s) => (s, None),
        Err(e) => (Default::default(), Some(e.to_string())),
    };
    rep.errors += stats.item_errors;

    // Everything below this point writes; wrap it in a single transaction
    // (absent on dry-run) and reference its run row.
    let tx = if opts.dry_run {
        None
    } else {
        Some(vault.conn.unchecked_transaction()?)
    };

    if let Some(err) = hard_error {
        rep.status = "error".to_string();
        rep.note = Some(err.clone());
        if let Some(tx) = tx {
            let run_id = open_run(vault, source.id, effective.strategy, now)?;
            finalize_run(vault, run_id, now, &rep)?;
            sources::update_after_run(
                &vault.conn,
                source.id,
                now,
                run_id,
                "error",
                Some(&err),
                None,
            )?;
            tx.commit()?;
        }
        return Ok(rep);
    }

    let run_id = if opts.dry_run {
        0
    } else {
        open_run(vault, source.id, effective.strategy, now)?
    };

    let mut seen: HashSet<String> = HashSet::new();
    for item in &items {
        // Document filter: extension include/exclude + targeted globs.
        if !filter.ext_ok(item.extension.as_deref())
            || !filter.path_ok(item.rel_path.as_deref(), &item.name)
        {
            rep.skipped += 1;
            continue;
        }
        seen.insert(item.uri.clone());

        let oversize = size_cap > 0 && item.size.map(|s| s as u64 > size_cap).unwrap_or(false);
        let content_hash = if oversize {
            None
        } else if let Some(h) = &item.provider_hash {
            Some(h.clone())
        } else if do_hash {
            hash_local(item)
        } else {
            None
        };

        let existing = registry::find_by_uri(&vault.conn, source.id, &item.uri)?;
        let metadata_str = serde_json::to_string(&item.metadata).unwrap_or_else(|_| "{}".into());

        match existing {
            None => {
                let status = if oversize {
                    DocStatus::TooLarge
                } else {
                    DocStatus::Present
                };
                if !opts.dry_run {
                    vault.conn.execute(
                        "INSERT INTO documents
                         (source_id, uri, name, rel_path, extension, size, modified_ms,
                          content_hash, metadata, status, discovered_at, first_run_id,
                          last_seen, last_run_id)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?11,?12)",
                        params![
                            source.id,
                            item.uri,
                            item.name,
                            item.rel_path,
                            item.extension,
                            item.size,
                            item.modified_ms,
                            content_hash,
                            metadata_str,
                            status.as_str(),
                            now,
                            run_id,
                        ],
                    )?;
                }
                rep.discovered += 1;
            }
            Some(prev) => {
                let changed = doc_changed(&prev, item, &content_hash);
                let status = if oversize {
                    DocStatus::TooLarge
                } else if changed {
                    DocStatus::Modified
                } else {
                    DocStatus::Present
                };
                if !opts.dry_run {
                    vault.conn.execute(
                        "UPDATE documents SET name=?1, rel_path=?2, extension=?3, size=?4,
                         modified_ms=?5, content_hash=COALESCE(?6, content_hash), metadata=?7,
                         status=?8, last_seen=?9, last_run_id=?10
                         WHERE id=?11",
                        params![
                            item.name,
                            item.rel_path,
                            item.extension,
                            item.size,
                            item.modified_ms,
                            content_hash,
                            metadata_str,
                            status.as_str(),
                            now,
                            run_id,
                            prev.id,
                        ],
                    )?;
                }
                if changed {
                    rep.updated += 1;
                }
            }
        }
    }

    // Anything in this source not touched this run has vanished — but only an
    // exhaustive pass is entitled to draw that conclusion.
    if full_enumeration {
        if opts.dry_run {
            let mut stmt = vault
                .conn
                .prepare("SELECT uri FROM documents WHERE source_id = ?1 AND status != 'gone'")?;
            let prev_uris: Vec<String> = stmt
                .query_map(params![source.id], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<_, _>>()?;
            rep.gone = prev_uris.iter().filter(|u| !seen.contains(*u)).count() as u32;
        } else {
            let gone = vault.conn.execute(
                "UPDATE documents SET status='gone', last_run_id=?1
                 WHERE source_id=?2 AND last_run_id IS NOT ?1 AND status != 'gone'",
                params![run_id, source.id],
            )?;
            rep.gone = gone as u32;
        }
    }

    rep.status = if rep.errors > 0 { "partial" } else { "ok" }.to_string();

    if let Some(tx) = tx {
        finalize_run(vault, run_id, now, &rep)?;
        let patched_config = stats.config_patch.as_ref().map(|(k, v)| {
            let mut cfg = source.config.clone();
            if !cfg.is_object() {
                cfg = serde_json::json!({});
            }
            cfg.as_object_mut().unwrap().insert(k.clone(), v.clone());
            cfg
        });
        sources::update_after_run(
            &vault.conn,
            source.id,
            now,
            run_id,
            &rep.status,
            None,
            patched_config.as_ref(),
        )?;
        tx.commit()?;
    }

    Ok(rep)
}

fn open_run(vault: &CrawlVault, source_id: i64, strategy: Strategy, now: i64) -> Result<i64> {
    vault.conn.execute(
        "INSERT INTO runs (source_id, strategy, started_at, status) VALUES (?1, ?2, ?3, 'partial')",
        params![source_id, strategy.as_str(), now],
    )?;
    Ok(vault.conn.last_insert_rowid())
}

fn finalize_run(vault: &CrawlVault, run_id: i64, now: i64, rep: &SourceRunReport) -> Result<()> {
    vault.conn.execute(
        "UPDATE runs SET finished_at=?1, discovered=?2, updated=?3, gone=?4, skipped=?5,
         errors=?6, status=?7, note=?8 WHERE id=?9",
        params![
            now,
            rep.discovered,
            rep.updated,
            rep.gone,
            rep.skipped,
            rep.errors,
            rep.status,
            rep.note,
            run_id,
        ],
    )?;
    Ok(())
}

fn doc_changed(prev: &DocumentRow, item: &DiscoveredItem, new_hash: &Option<String>) -> bool {
    if prev.size != item.size || prev.modified_ms != item.modified_ms {
        return true;
    }
    if let Some(h) = new_hash {
        if prev.content_hash.as_deref() != Some(h.as_str()) {
            return true;
        }
    }
    // A row that had vanished and reappeared counts as changed.
    prev.status == DocStatus::Gone
}

fn hash_local(item: &DiscoveredItem) -> Option<String> {
    let path = item.local_path.as_ref()?;
    let bytes = std::fs::read(path).ok()?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Some(format!("{:x}", h.finalize()))
}
