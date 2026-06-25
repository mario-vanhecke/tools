//! Network/SMB share crawler.
//!
//! Portable, dependency-free SMB clients are heavyweight; the robust and
//! standard operational approach is to mount the share and walk it as a normal
//! filesystem path. This crawler resolves a share locator to its local mount
//! point and then reuses the local walker. It accepts:
//!
//!   * an already-mounted path (`/Volumes/team`, `/mnt/share`)
//!   * a `mount` override in the source config (the exact local mount path)
//!   * a UNC path (`\\server\share\dir`) or `smb://server/share/dir` URL,
//!     resolved against common mount roots
//!
//! If the share isn't mounted, the crawl fails with an actionable message
//! rather than silently finding nothing.

use super::{local, CrawlContext, CrawlStats, Crawler, DiscoveredItem};
use crate::error::{Error, Result};
use crate::source::Source;
use crate::SourceKind;
use std::path::{Path, PathBuf};

pub struct SmbCrawler;

impl Crawler for SmbCrawler {
    fn kind(&self) -> SourceKind {
        SourceKind::Smb
    }

    fn crawl(
        &self,
        source: &Source,
        ctx: &CrawlContext,
        sink: &mut dyn FnMut(DiscoveredItem),
    ) -> Result<CrawlStats> {
        let root = resolve_mount(source)?;
        local::walk(&root, ctx, sink)
    }
}

fn resolve_mount(source: &Source) -> Result<PathBuf> {
    // 1. Explicit mount override always wins.
    if let Some(m) = source.config_str("mount") {
        return check_dir(source, Path::new(&m));
    }

    // 2. The locator may already be a mounted/local path.
    let raw = Path::new(&source.uri);
    if raw.exists() {
        return check_dir(source, raw);
    }

    // 3. Resolve a UNC/smb URL to a likely mount point.
    let (host, share) = parse_share(&source.uri);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(share) = &share {
        candidates.push(PathBuf::from("/Volumes").join(share));
        candidates.push(PathBuf::from("/mnt").join(share));
        if let Ok(user) = std::env::var("USER") {
            candidates.push(PathBuf::from("/media").join(&user).join(share));
        }
    }
    if let Some(host) = &host {
        candidates.push(PathBuf::from("/Volumes").join(host));
    }
    for c in &candidates {
        if c.is_dir() {
            return check_dir(source, c);
        }
    }

    Err(Error::unreachable(
        &source.name,
        format!(
            "share '{}' is not mounted. Mount it (e.g. macOS: `mount_smbfs //user@{}/{} /Volumes/{}`; \
             Linux: `mount -t cifs //{}/{} /mnt/{}`) or set this source's `mount` config to the \
             local mount path.",
            source.uri,
            host.as_deref().unwrap_or("server"),
            share.as_deref().unwrap_or("share"),
            share.as_deref().unwrap_or("share"),
            host.as_deref().unwrap_or("server"),
            share.as_deref().unwrap_or("share"),
            share.as_deref().unwrap_or("share"),
        ),
    ))
}

fn check_dir(source: &Source, p: &Path) -> Result<PathBuf> {
    let abs = p
        .canonicalize()
        .map_err(|e| Error::unreachable(&source.name, format!("{}: {e}", p.display())))?;
    if !abs.is_dir() {
        return Err(Error::unreachable(
            &source.name,
            format!("{} is not a directory", abs.display()),
        ));
    }
    Ok(abs)
}

/// Pull `(host, share)` out of `\\host\share\...`, `//host/share/...`, or
/// `smb://host/share/...`.
fn parse_share(uri: &str) -> (Option<String>, Option<String>) {
    let mut s = uri.replace('\\', "/");
    for prefix in ["smb://", "cifs://", "nfs://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
            break;
        }
    }
    let s = s.trim_start_matches('/');
    let mut parts = s.split('/').filter(|p| !p.is_empty());
    let host = parts.next().map(String::from);
    let share = parts.next().map(String::from);
    (host, share)
}

#[cfg(test)]
mod tests {
    use super::parse_share;

    #[test]
    fn parses_unc_and_smb() {
        assert_eq!(
            parse_share(r"\\server\team\docs"),
            (Some("server".into()), Some("team".into()))
        );
        assert_eq!(
            parse_share("smb://files.corp/shared/x"),
            (Some("files.corp".into()), Some("shared".into()))
        );
        assert_eq!(
            parse_share("//host/share"),
            (Some("host".into()), Some("share".into()))
        );
    }
}
