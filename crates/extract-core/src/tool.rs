//! Locate external helper tools (`pandoc`, `pdftotext`).
//!
//! Resolution order:
//!   1. `PATH`, via the `which` crate — a system install always wins.
//!   2. Next to the running executable, and a sibling `bin/` directory.
//!
//! Step 2 is what makes the prebuilt "tools" bundle portable: it can ship
//! `pandoc`/`poppler` alongside the binaries and have them picked up with no
//! PATH configuration, while a normal install is unaffected (PATH is tried
//! first).

use std::path::{Path, PathBuf};

/// Resolve `name` to an executable path, or `None` if it can't be found.
pub fn locate(name: &str) -> Option<PathBuf> {
    if let Ok(p) = which::which(name) {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    locate_in(exe.parent()?, name)
}

/// Look for `name` directly in `dir` or in `dir/bin` (the bundle layout).
/// Factored out from [`locate`] so it can be unit-tested without touching the
/// real executable path.
fn locate_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    [dir.join(&file), dir.join("bin").join(&file)]
        .into_iter()
        .find(|c| c.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_tool_beside_exe_and_in_bin_subdir() {
        let root = std::env::temp_dir().join(format!("extract-tool-{}", std::process::id()));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = format!("helper{}", std::env::consts::EXE_SUFFIX);

        // Nothing there yet.
        assert!(locate_in(&root, "helper").is_none());

        // Found in bin/.
        std::fs::write(bin.join(&exe), b"x").unwrap();
        assert_eq!(locate_in(&root, "helper"), Some(bin.join(&exe)));

        // A copy directly beside the exe takes precedence over bin/.
        std::fs::write(root.join(&exe), b"x").unwrap();
        assert_eq!(locate_in(&root, "helper"), Some(root.join(&exe)));

        let _ = std::fs::remove_dir_all(&root);
    }
}
