//! The `sources` model: what crawl knows how to enumerate, and how.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The kind of place a source points at. Each kind maps to one `Crawler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A directory on a locally-attached filesystem.
    Local,
    /// A network/SMB share. Crawled through its mount point (see `smb` crawler).
    Smb,
    /// A SharePoint document library (a Microsoft Graph drive).
    SharePoint,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Smb => "smb",
            Self::SharePoint => "sharepoint",
        }
    }
    pub fn from_str(s: &str) -> Result<Self> {
        Ok(match s.to_lowercase().as_str() {
            "local" | "dir" | "directory" => Self::Local,
            "smb" | "share" | "unc" | "nfs" => Self::Smb,
            "sharepoint" | "sp" | "graph" => Self::SharePoint,
            other => return Err(Error::UnknownKind(other.to_string())),
        })
    }
}

/// How thoroughly to crawl a source. The strategy is the tool's answer to
/// "find documents on its own" — each one is a different traversal policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// Walk the entire tree, every level. The exhaustive default.
    Recursive,
    /// Only the top level of the source root (depth 1). Fast reconnaissance.
    Shallow,
    /// Only items modified since the source's last successful crawl
    /// (a delta walk). Cheap re-crawls of large, slow-changing trees.
    Incremental,
    /// Only items matching the configured include globs (e.g. `**/*.pdf`).
    /// Hunt for a specific class of document across a noisy tree.
    Targeted,
}

impl Strategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Recursive => "recursive",
            Self::Shallow => "shallow",
            Self::Incremental => "incremental",
            Self::Targeted => "targeted",
        }
    }
    pub fn from_str(s: &str) -> Result<Self> {
        Ok(match s.to_lowercase().as_str() {
            "recursive" | "full" | "deep" => Self::Recursive,
            "shallow" | "top" => Self::Shallow,
            "incremental" | "delta" | "since" => Self::Incremental,
            "targeted" | "glob" | "pattern" => Self::Targeted,
            other => return Err(Error::UnknownStrategy(other.to_string())),
        })
    }
}

/// A registered crawl source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: i64,
    pub name: String,
    pub kind: SourceKind,
    pub uri: String,
    pub strategy: Strategy,
    pub config: Value,
    pub enabled: bool,
    pub added_at: i64,
    pub last_crawled: Option<i64>,
    pub last_run_id: Option<i64>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
}

/// Strategy parameters resolved from a source's `config` JSON plus the
/// strategy itself and the source's last-crawl timestamp. Crawlers consult
/// this to decide what to enumerate.
#[derive(Debug, Clone, Default)]
pub struct StrategyParams {
    /// Maximum traversal depth from the root. `None` = unlimited. `Shallow`
    /// pins this to 1.
    pub max_depth: Option<usize>,
    /// Only emit items modified at or after this epoch-ms. Set by `Incremental`.
    pub since_ms: Option<i64>,
    /// Only emit items whose path matches one of these globs. Set by `Targeted`.
    pub include_globs: Vec<String>,
    /// Never emit items whose path matches one of these globs.
    pub exclude_globs: Vec<String>,
}

impl Source {
    /// Read a string field from the source's JSON config.
    pub fn config_str(&self, key: &str) -> Option<String> {
        self.config
            .get(key)
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    pub fn config_u64(&self, key: &str) -> Option<u64> {
        self.config.get(key).and_then(|v| v.as_u64())
    }

    fn config_globs(&self, key: &str) -> Vec<String> {
        match self.config.get(key) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        }
    }

    /// Resolve the effective traversal parameters for this source's strategy.
    /// `default_max_depth` (0 = unlimited) comes from vault config and is used
    /// only when the source config does not pin its own `max_depth`.
    pub fn resolve_params(&self, default_max_depth: usize) -> StrategyParams {
        let cfg_depth = self.config_u64("max_depth").map(|d| d as usize);
        let base_depth = cfg_depth.or(if default_max_depth == 0 {
            None
        } else {
            Some(default_max_depth)
        });

        let mut p = StrategyParams {
            max_depth: base_depth,
            since_ms: None,
            include_globs: self.config_globs("include_globs"),
            exclude_globs: self.config_globs("exclude_globs"),
        };

        match self.strategy {
            Strategy::Recursive => {}
            Strategy::Shallow => p.max_depth = Some(1),
            Strategy::Incremental => {
                // Explicit `since_ms` in config wins; otherwise the last crawl.
                p.since_ms = self
                    .config
                    .get("since_ms")
                    .and_then(|v| v.as_i64())
                    .or(self.last_crawled);
            }
            Strategy::Targeted => {
                if p.include_globs.is_empty() {
                    // A targeted crawl with no globs configured matches nothing
                    // useful; fall back to "all files" so it degrades to a walk.
                    p.include_globs.push("**/*".to_string());
                }
            }
        }
        p
    }
}
