//! gitignore-style ignore matcher with toolkit-wide built-in defaults plus
//! an optional user-authored ignore file at the vault root.
//!
//! Each tool decides what its ignore file is named (`.vaultignore` for rag,
//! `.mdignore` for md, etc.) and passes that to `VaultIgnore::load`.

use crate::error::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

/// Built-in patterns that are always excluded regardless of user config.
///
/// These cover state directories of the tools in this distribution, common
/// VCS metadata, OS junk files, and Python build artifacts that should never
/// be content.
pub const BUILT_IN_DEFAULTS: &[&str] = &[
    ".vault/",
    ".vaultignore",
    ".md/",
    ".mdignore",
    ".crawl/",
    ".crawlignore",
    ".git/",
    ".gitignore",
    ".svn/",
    ".hg/",
    "node_modules/",
    "__pycache__/",
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    "*.pyc",
    ".idea/",
    ".vscode/",
];

pub struct VaultIgnore {
    pub matcher: Gitignore,
    pub respects_user_ignore: bool,
}

impl VaultIgnore {
    /// Build a matcher: built-in defaults plus optional user-authored
    /// ignore file at `<root>/<ignore_filename>`. With
    /// `respect_user_ignore = false` only the built-in patterns apply.
    pub fn load(root: &Path, ignore_filename: &str, respect_user_ignore: bool) -> Result<Self> {
        let mut b = GitignoreBuilder::new(root);
        for pat in BUILT_IN_DEFAULTS {
            b.add_line(None, pat).expect("built-in pattern parses");
        }
        if respect_user_ignore {
            let p = root.join(ignore_filename);
            if p.is_file() {
                let _ = b.add(&p);
            }
        }
        let matcher = b
            .build()
            .map_err(|e| crate::error::Error::Other(e.to_string()))?;
        Ok(Self {
            matcher,
            respects_user_ignore: respect_user_ignore,
        })
    }

    /// Empty matcher — used by `--force` modes that bypass everything.
    pub fn empty(root: &Path) -> Result<Self> {
        let b = GitignoreBuilder::new(root);
        let matcher = b
            .build()
            .map_err(|e| crate::error::Error::Other(e.to_string()))?;
        Ok(Self {
            matcher,
            respects_user_ignore: false,
        })
    }

    /// Defaults-only matcher — used by `--no-ignore` modes.
    pub fn defaults_only(root: &Path) -> Result<Self> {
        let mut b = GitignoreBuilder::new(root);
        for pat in BUILT_IN_DEFAULTS {
            b.add_line(None, pat).expect("built-in pattern parses");
        }
        let matcher = b
            .build()
            .map_err(|e| crate::error::Error::Other(e.to_string()))?;
        Ok(Self {
            matcher,
            respects_user_ignore: false,
        })
    }

    /// Returns true if the path should be excluded.
    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        matches!(
            self.matcher.matched_path_or_any_parents(path, is_dir),
            ::ignore::Match::Ignore(_)
        )
    }
}
