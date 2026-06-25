pub mod config;
pub mod export;
pub mod find;
pub mod info;
pub mod init;
pub mod ls;
pub mod prune;
pub mod rm;
pub mod run;
pub mod source;
pub mod status;

use crawl_core::CrawlVault;
use std::path::Path;

pub fn open_vault(vault_arg: Option<&Path>) -> anyhow::Result<CrawlVault> {
    let v = match vault_arg {
        Some(p) => CrawlVault::open(p)?,
        None => {
            let cwd = std::env::current_dir()?;
            CrawlVault::discover(&cwd)?
        }
    };
    Ok(v)
}

/// Resolve an optional `--source NAME` flag to a source id, erroring if the
/// named source does not exist.
pub fn resolve_source_id(vault: &CrawlVault, name: Option<&str>) -> anyhow::Result<Option<i64>> {
    match name {
        None => Ok(None),
        Some(n) => {
            let src = crawl_core::registry::get_source_by_name(&vault.conn, n)?
                .ok_or_else(|| crawl_core::Error::NoSuchSource(n.to_string()))?;
            Ok(Some(src.id))
        }
    }
}
