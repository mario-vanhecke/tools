//! Source enumeration. Each source yields `SourceDoc`s described **by
//! reference** — a locator back to the origin plus a lazy `read` closure, so
//! unchanged documents can be skipped without ever reading their bytes.

pub mod local;
pub mod sharepoint;

use anyhow::Result;
use kb_core::SourceConfig;

pub type ReadFn = Box<dyn FnOnce() -> Result<Vec<u8>> + Send>;

pub struct SourceDoc {
    /// Origin pointer stored in the index (file://, smb://, SharePoint webUrl).
    pub locator: String,
    /// Human title (usually the file name).
    pub title: String,
    /// Lower-cased file extension, used to route extraction.
    pub ext: String,
    pub modified_at: Option<String>,
    pub size: Option<u64>,
    /// Lazily read the document's bytes (only called when (re)indexing).
    pub read: ReadFn,
}

/// File extensions we know how to extract. Enumeration is limited to these so
/// we never read arbitrary binaries.
pub const SUPPORTED_EXTS: &[&str] = &[
    "txt", "text", "log", "csv", "tsv", "md", "markdown", "rst", "org", "html", "htm", "xhtml",
    "pdf", "docx", "pptx", "xlsx", "epub", "odt", "rtf",
];

pub fn is_supported(ext: &str) -> bool {
    SUPPORTED_EXTS.contains(&ext.to_ascii_lowercase().as_str())
}

/// Enumerate all documents for a source.
pub fn enumerate(src: &SourceConfig) -> Result<Vec<SourceDoc>> {
    match src {
        SourceConfig::Local { path, .. } | SourceConfig::Smb { path, .. } => local::enumerate(path),
        SourceConfig::Sharepoint { .. } => sharepoint::enumerate(src),
    }
}
