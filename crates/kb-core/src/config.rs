//! The single declarative config that drives a build: `knowledge.toml`.
//!
//! ```toml
//! [[source]]
//! type = "local"
//! path = "C:/Users/me/Documents"
//!
//! [[source]]
//! type = "sharepoint"
//! site = "tenant.sharepoint.com/sites/Eng"
//! auth = "browser"
//!
//! [embedding]
//! endpoint = "http://localhost:11434/v1"   # OpenAI-compatible (Ollama default)
//! model    = "nomic-embed-text"
//! dims     = 768
//!
//! [output]
//! path = "./knowledge.kb"
//! ```

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Declared sources. TOML array-of-tables: `[[source]]`.
    #[serde(default, rename = "source")]
    pub sources: Vec<SourceConfig>,

    pub embedding: EmbeddingConfig,

    #[serde(default)]
    pub output: OutputConfig,

    #[serde(default)]
    pub chunk: ChunkConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SourceConfig {
    /// A local directory.
    Local {
        #[serde(default)]
        name: Option<String>,
        path: String,
    },
    /// A mounted SMB / network share. Enumerated like a local path (UNC or a
    /// mounted drive letter / mount point).
    Smb {
        #[serde(default)]
        name: Option<String>,
        path: String,
    },
    /// A SharePoint site, crawled recursively via its REST API.
    Sharepoint {
        #[serde(default)]
        name: Option<String>,
        /// e.g. `tenant.sharepoint.com/sites/Eng`
        site: String,
        /// `browser` (interactive) or `cookie` (session cookie from env).
        #[serde(default = "default_sp_auth")]
        auth: String,
        /// Env var holding a session cookie when `auth = "cookie"`.
        #[serde(default)]
        cookie_env: Option<String>,
    },
}

impl SourceConfig {
    /// Short stable name for this source, used as the `source` column and in
    /// progress output. Derived from `path`/`site` when not given explicitly.
    pub fn name(&self) -> String {
        match self {
            SourceConfig::Local { name, path } | SourceConfig::Smb { name, path } => {
                name.clone().unwrap_or_else(|| derive_name(path))
            }
            SourceConfig::Sharepoint { name, site, .. } => {
                name.clone().unwrap_or_else(|| derive_name(site))
            }
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            SourceConfig::Local { .. } => "local",
            SourceConfig::Smb { .. } => "smb",
            SourceConfig::Sharepoint { .. } => "sharepoint",
        }
    }
}

fn derive_name(s: &str) -> String {
    s.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|seg| !seg.is_empty())
        .unwrap_or("source")
        .to_string()
}

fn default_sp_auth() -> String {
    "browser".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingConfig {
    /// Base URL of an OpenAI-compatible embeddings API (the client POSTs to
    /// `{endpoint}/embeddings`). Default points at a local Ollama.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_model")]
    pub model: String,
    /// Output dimensionality of `model`. Baked into the vec0 table at create
    /// time, so it must match the model.
    pub dims: usize,
    /// Optional env var holding a bearer token (for hosted endpoints).
    #[serde(default)]
    pub api_key_env: Option<String>,
}

fn default_endpoint() -> String {
    "http://localhost:11434/v1".to_string()
}
fn default_model() -> String {
    "nomic-embed-text".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutputConfig {
    #[serde(default = "default_output")]
    pub path: String,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            path: default_output(),
        }
    }
}

fn default_output() -> String {
    "./knowledge.kb".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChunkConfig {
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    #[serde(default = "default_overlap")]
    pub overlap: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_chars: default_max_chars(),
            overlap: default_overlap(),
        }
    }
}

fn default_max_chars() -> usize {
    1200
}
fn default_overlap() -> usize {
    200
}

impl Config {
    /// Parse a `knowledge.toml` from disk.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("cannot read {}: {e}", path.display())))?;
        let cfg: Config = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self.sources.is_empty() {
            return Err(Error::Config(
                "no [[source]] entries — nothing to index".into(),
            ));
        }
        if self.embedding.dims == 0 {
            return Err(Error::Config("embedding.dims must be > 0".into()));
        }
        Ok(())
    }

    /// A ready-to-edit starter config.
    pub fn template() -> &'static str {
        TEMPLATE
    }
}

const TEMPLATE: &str = r#"# knowledge.toml — declares what `distill` indexes.
# Sources stay at their origin; only the index (output.path) is written.

[[source]]
type = "local"
path = "./docs"
# name = "docs"          # optional label; defaults to the last path segment

# [[source]]
# type = "smb"
# path = "//server/share/policies"

# [[source]]
# type = "sharepoint"
# site = "tenant.sharepoint.com/sites/Eng"
# auth = "cookie"          # cookie mode works on locked-down tenants
# cookie_env = "KB_SP_COOKIE"   # holds your FedAuth/rtFa session cookies

[embedding]
endpoint = "http://localhost:11434/v1"   # any OpenAI-compatible API (Ollama here)
model    = "nomic-embed-text"
dims     = 768                           # must match the model's output size
# api_key_env = "OPENAI_API_KEY"         # for hosted endpoints

[output]
path = "./knowledge.kb"

[chunk]
max_chars = 1200
overlap   = 200
"#;
