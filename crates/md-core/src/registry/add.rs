use crate::error::{Error, Result};
use crate::vault::MdVault;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use vault_core::VaultIgnore;

#[derive(Debug, Clone, Default)]
pub struct AddOptions {
    pub skip_unsupported: bool,
    pub no_ignore: bool,
    pub force: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddReport {
    pub added: u32,
    pub already_registered: u32,
    pub skipped_by_ignore: u32,
    pub skipped_unsupported: u32,
    pub files: Vec<AddedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddedFile {
    pub path: String,
    pub action: AddAction,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AddAction {
    Added,
    AlreadyRegistered,
    SkippedUnsupported,
}

pub fn add_paths(vault: &MdVault, paths: &[PathBuf], opts: &AddOptions) -> Result<AddReport> {
    let ignore_matcher = if opts.force {
        VaultIgnore::empty(&vault.root)?
    } else if opts.no_ignore {
        VaultIgnore::defaults_only(&vault.root)?
    } else {
        VaultIgnore::load(
            &vault.root,
            ".mdignore",
            vault.config.files.respect_mdignore,
        )?
    };

    let supported: std::collections::HashSet<String> = vault
        .config
        .files
        .supported_extensions
        .iter()
        .map(|s| s.to_lowercase())
        .collect();

    let mut report = AddReport::default();
    let mut to_add: Vec<PathBuf> = Vec::new();

    for input in paths {
        let abs0 = if input.is_absolute() {
            input.clone()
        } else {
            std::env::current_dir()?.join(input)
        };
        if !abs0.exists() {
            return Err(Error::Vault(vault_core::Error::InvalidPath(format!(
                "{} does not exist",
                input.display()
            ))));
        }
        let abs = abs0.canonicalize().unwrap_or(abs0);
        if abs.is_file() {
            let rel = vault.relativize(&abs)?;
            if !opts.force && ignore_matcher.is_ignored(&abs, false) {
                report.skipped_by_ignore += 1;
                continue;
            }
            if opts.skip_unsupported && !is_supported(&rel, &supported) {
                report.skipped_unsupported += 1;
                continue;
            }
            to_add.push(abs);
        } else if abs.is_dir() {
            collect_dir(
                &abs,
                vault,
                &ignore_matcher,
                opts,
                &supported,
                &mut to_add,
                &mut report,
            )?;
        }
    }

    let now = chrono::Utc::now().timestamp_millis();
    if !opts.dry_run {
        let tx = vault.conn.unchecked_transaction()?;
        for abs in &to_add {
            let rel = vault.relativize(abs)?;
            let rel_s = rel.to_string_lossy().to_string();
            let exists: bool = tx
                .query_row(
                    "SELECT 1 FROM outputs WHERE input_path = ?1",
                    params![rel_s],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if exists {
                report.already_registered += 1;
                report.files.push(AddedFile {
                    path: rel_s,
                    action: AddAction::AlreadyRegistered,
                });
                continue;
            }
            tx.execute(
                "INSERT INTO outputs (input_path, added_at, status, attempts)
                 VALUES (?1, ?2, 'pending', 0)",
                params![rel_s, now],
            )?;
            report.added += 1;
            report.files.push(AddedFile {
                path: rel_s,
                action: AddAction::Added,
            });
        }
        tx.commit()?;
    } else {
        for abs in &to_add {
            let rel = vault.relativize(abs)?;
            let rel_s = rel.to_string_lossy().to_string();
            let exists: bool = vault
                .conn
                .query_row(
                    "SELECT 1 FROM outputs WHERE input_path = ?1",
                    params![rel_s],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if exists {
                report.already_registered += 1;
                report.files.push(AddedFile {
                    path: rel_s,
                    action: AddAction::AlreadyRegistered,
                });
            } else {
                report.added += 1;
                report.files.push(AddedFile {
                    path: rel_s,
                    action: AddAction::Added,
                });
            }
        }
    }

    Ok(report)
}

fn collect_dir(
    dir: &Path,
    vault: &MdVault,
    ignore_matcher: &VaultIgnore,
    opts: &AddOptions,
    supported: &std::collections::HashSet<String>,
    out: &mut Vec<PathBuf>,
    report: &mut AddReport,
) -> Result<()> {
    let state_dir = vault.state_dir.clone();
    let output_dir = vault.output_dir_abs();
    let force = opts.force;
    let walker = walkdir::WalkDir::new(dir).follow_links(false);
    let walker = walker.into_iter().filter_entry(move |e| {
        let p = e.path();
        // Never descend into our own state dir, and never re-add converted
        // outputs back as new inputs.
        if p.starts_with(&state_dir) || p.starts_with(&output_dir) {
            return false;
        }
        if force {
            return true;
        }
        if e.file_type().is_dir() {
            return !ignore_matcher.is_ignored(p, true);
        }
        true
    });
    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path == dir {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        if !opts.force && ignore_matcher.is_ignored(path, false) {
            report.skipped_by_ignore += 1;
            continue;
        }
        let rel = vault.relativize(path)?;
        if opts.skip_unsupported && !is_supported(&rel, supported) {
            report.skipped_unsupported += 1;
            continue;
        }
        out.push(path.to_path_buf());
    }
    Ok(())
}

fn is_supported(rel: &Path, supported: &std::collections::HashSet<String>) -> bool {
    match rel.extension().and_then(|e| e.to_str()) {
        Some(ext) => supported.contains(&ext.to_lowercase()),
        None => false,
    }
}
