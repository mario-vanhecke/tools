//! Local-filesystem crawler. Also the engine behind the `smb` crawler, which
//! resolves a share to a mount point and walks it the same way.

use super::{CrawlContext, CrawlStats, Crawler, DiscoveredItem};
use crate::error::{Error, Result};
use crate::source::Source;
use crate::vault::IGNORE_FILE;
use crate::SourceKind;
use std::path::Path;
use std::time::UNIX_EPOCH;
use vault_core::VaultIgnore;

pub struct LocalCrawler;

impl Crawler for LocalCrawler {
    fn kind(&self) -> SourceKind {
        SourceKind::Local
    }

    fn crawl(
        &self,
        source: &Source,
        ctx: &CrawlContext,
        sink: &mut dyn FnMut(DiscoveredItem),
    ) -> Result<CrawlStats> {
        let root = resolve_root(source)?;
        walk(&root, ctx, sink)
    }
}

/// Resolve a local source's root to an existing directory, or fail with a
/// clear "unreachable" error.
pub(crate) fn resolve_root(source: &Source) -> Result<std::path::PathBuf> {
    let p = Path::new(&source.uri);
    let abs = p
        .canonicalize()
        .map_err(|e| Error::unreachable(&source.name, format!("{}: {e}", source.uri)))?;
    if !abs.is_dir() {
        return Err(Error::unreachable(
            &source.name,
            format!("{} is not a directory", abs.display()),
        ));
    }
    Ok(abs)
}

/// Walk `root` and emit every file, honoring the strategy's depth and
/// since-filter, this source's `.crawlignore`, and the symlink policy.
/// Shared by the local and smb crawlers.
pub(crate) fn walk(
    root: &Path,
    ctx: &CrawlContext,
    sink: &mut dyn FnMut(DiscoveredItem),
) -> Result<CrawlStats> {
    let mut stats = CrawlStats::default();

    let ignore = VaultIgnore::load(root, IGNORE_FILE, ctx.config.crawl.respect_crawlignore)?;

    let mut builder = walkdir::WalkDir::new(root).follow_links(ctx.config.crawl.follow_symlinks);
    if let Some(depth) = ctx.params.max_depth {
        builder = builder.max_depth(depth);
    }

    let walker = builder.into_iter().filter_entry(|e| {
        // Prune ignored directories so we never descend into them.
        if e.file_type().is_dir() {
            return !ignore.is_ignored(e.path(), true);
        }
        true
    });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                stats.item_errors += 1;
                continue;
            }
        };
        let path = entry.path();
        if path == root || !entry.file_type().is_file() {
            continue;
        }
        if ignore.is_ignored(path, false) {
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                stats.item_errors += 1;
                continue;
            }
        };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);

        // Incremental strategy: skip anything older than the since cutoff.
        if let (Some(since), Some(m)) = (ctx.params.since_ms, modified_ms) {
            if m < since {
                continue;
            }
        }

        let name = entry.file_name().to_string_lossy().to_string();
        let rel = path
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .ok();

        let mut item = DiscoveredItem::new(path.to_string_lossy().to_string(), name);
        item.rel_path = rel;
        item.size = Some(meta.len() as i64);
        item.modified_ms = modified_ms;
        item.local_path = Some(path.to_path_buf());
        sink(item);
    }

    Ok(stats)
}
