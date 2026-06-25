//! The crawl strategy engine: a `Crawler` per source kind, a uniform
//! `DiscoveredItem` they emit, and the document filter the orchestrator
//! applies on top. The orchestrator itself lives in [`run`].

pub mod local;
pub mod run;
pub mod sharepoint;
pub mod smb;

pub use run::{run, RunOptions, RunReport, SourceRunReport};

use crate::config::Config;
use crate::error::Result;
use crate::source::{Source, SourceKind, StrategyParams};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;

/// One file a crawler discovered. Crawlers emit files only; the orchestrator
/// decides what to record and in what status.
#[derive(Debug, Clone)]
pub struct DiscoveredItem {
    /// Canonical, stable locator (absolute path for local/smb, webUrl for
    /// SharePoint). Unique within a source.
    pub uri: String,
    pub name: String,
    /// Path relative to the source root, forward slashes. `None` if unknown.
    pub rel_path: Option<String>,
    /// Lowercase extension without the dot. `None` if the name has none.
    pub extension: Option<String>,
    pub size: Option<i64>,
    /// Source-reported last-modified time, epoch milliseconds.
    pub modified_ms: Option<i64>,
    /// Content hash supplied by the provider (e.g. SharePoint quickXorHash).
    pub provider_hash: Option<String>,
    pub metadata: Value,
    /// Local filesystem path to read for `--hash`. `None` when content is not
    /// locally readable (SharePoint).
    pub local_path: Option<PathBuf>,
}

impl DiscoveredItem {
    /// Build an item from a name + locator, deriving the extension.
    pub fn new(uri: impl Into<String>, name: impl Into<String>) -> Self {
        let name = name.into();
        let extension = std::path::Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());
        Self {
            uri: uri.into(),
            name,
            rel_path: None,
            extension,
            size: None,
            modified_ms: None,
            provider_hash: None,
            metadata: serde_json::json!({}),
            local_path: None,
        }
    }
}

/// Everything a crawler needs for one pass over one source.
pub struct CrawlContext<'a> {
    pub params: StrategyParams,
    pub config: &'a Config,
    /// Vault state dir (`.crawl/`), where crawlers may cache auth tokens.
    pub cache_dir: &'a std::path::Path,
}

/// Soft outcome of a crawl. Hard failures return `Err` from `crawl`.
#[derive(Debug, Default)]
pub struct CrawlStats {
    /// Items the crawler could not stat/read (counted, not recorded).
    pub item_errors: u32,
    /// A config key/value the crawler wants persisted on the source (e.g. a
    /// refreshed SharePoint delta link for the next incremental run).
    pub config_patch: Option<(String, Value)>,
}

/// One traversal strategy realized for one source kind.
pub trait Crawler {
    fn kind(&self) -> SourceKind;
    /// Enumerate the source, invoking `sink` for every discovered file.
    /// Returns soft stats; returns `Err` only on failures that abort the
    /// source entirely (mount missing, auth failure, network down).
    fn crawl(
        &self,
        source: &Source,
        ctx: &CrawlContext,
        sink: &mut dyn FnMut(DiscoveredItem),
    ) -> Result<CrawlStats>;
}

pub fn crawler_for(kind: SourceKind) -> Box<dyn Crawler> {
    match kind {
        SourceKind::Local => Box::new(local::LocalCrawler),
        SourceKind::Smb => Box::new(smb::SmbCrawler),
        SourceKind::SharePoint => Box::new(sharepoint::SharePointCrawler),
    }
}

/// Post-enumeration filter: which discovered files actually get recorded.
/// Extension include/exclude come from vault config; the glob include/exclude
/// come from the source's strategy params (the `targeted` strategy).
pub struct DocFilter {
    extensions: HashSet<String>,
    excluded: HashSet<String>,
    include_globs: Vec<glob::Pattern>,
    exclude_globs: Vec<glob::Pattern>,
}

impl DocFilter {
    pub fn build(config: &Config, params: &StrategyParams) -> Self {
        let extensions = config
            .documents
            .extensions
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        let excluded = config
            .documents
            .excluded_extensions
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        let compile = |globs: &[String]| -> Vec<glob::Pattern> {
            globs
                .iter()
                .filter_map(|g| glob::Pattern::new(g).ok())
                .collect()
        };
        Self {
            extensions,
            excluded,
            include_globs: compile(&params.include_globs),
            exclude_globs: compile(&params.exclude_globs),
        }
    }

    /// True if a file with this extension is a document we record. An empty
    /// `documents.extensions` set means "record every extension".
    pub fn ext_ok(&self, ext: Option<&str>) -> bool {
        let e = ext.map(|e| e.to_lowercase());
        if let Some(e) = &e {
            if self.excluded.contains(e) {
                return false;
            }
        }
        if self.extensions.is_empty() {
            return true;
        }
        match e {
            Some(e) => self.extensions.contains(&e),
            None => false,
        }
    }

    /// True if the path passes the targeted include/exclude globs. Globs are
    /// matched against both the relative path and the bare name, so `*.pdf`
    /// works at any depth.
    pub fn path_ok(&self, rel_path: Option<&str>, name: &str) -> bool {
        let rel = rel_path.unwrap_or(name);
        let matches =
            |pats: &[glob::Pattern]| pats.iter().any(|p| p.matches(rel) || p.matches(name));
        if !self.exclude_globs.is_empty() && matches(&self.exclude_globs) {
            return false;
        }
        if self.include_globs.is_empty() {
            return true;
        }
        matches(&self.include_globs)
    }
}
