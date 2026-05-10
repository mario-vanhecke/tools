use crate::error::{Error, Result};
use crate::registry;
use crate::vault::MdVault;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    /// Registered, not yet converted.
    Pending,
    /// Successfully converted; output file exists and matches our last write.
    Converted,
    /// Last conversion errored.
    Failed,
    /// Extension not handled.
    Unsupported,
    /// Extension excluded by config.
    Excluded,
    /// File size exceeds configured cap.
    TooLarge,
    /// PDF (or similar) had no extractable text.
    NeedsOcr,
    /// Source file no longer on disk.
    Missing,
    /// Source AND output both changed since last convert; user must choose.
    Conflict,
}

impl FileStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Converted => "converted",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
            Self::Excluded => "excluded",
            Self::TooLarge => "too_large",
            Self::NeedsOcr => "needs_ocr",
            Self::Missing => "missing",
            Self::Conflict => "conflict",
        }
    }
    pub fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "pending" => Self::Pending,
            "converted" => Self::Converted,
            "failed" => Self::Failed,
            "unsupported" => Self::Unsupported,
            "excluded" => Self::Excluded,
            "too_large" => Self::TooLarge,
            "needs_ocr" => Self::NeedsOcr,
            "missing" => Self::Missing,
            "conflict" => Self::Conflict,
            other => return Err(Error::other(format!("unknown status: {other}"))),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusReport {
    pub vault: VaultBlock,
    pub summary: SummaryBlock,
    pub files: Vec<FileBlock>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultBlock {
    pub path: String,
    pub name: String,
    pub output_dir: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SummaryBlock {
    pub registered: u32,
    pub converted: u32,
    pub pending: u32,
    pub reconvert: u32, // source changed since last convert
    pub output_modified: u32,
    pub conflict: u32,
    pub failed: u32,
    pub needs_ocr: u32,
    pub unsupported: u32,
    pub missing: u32,
    pub excluded: u32,
    pub too_large: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBlock {
    pub input_path: String,
    pub output_path: Option<String>,
    pub status: String,
    pub source_changed: bool,
    pub output_modified: bool,
}

#[derive(Debug, Clone, Default)]
pub struct StatusOptions {
    pub filter: Option<FileStatus>,
    pub no_stat: bool,
}

pub fn compute(vault: &MdVault, opts: &StatusOptions) -> Result<StatusReport> {
    let rows = registry::list_filtered(&vault.conn, opts.filter)?;
    let mut report = StatusReport::default();
    report.vault.path = vault.root.to_string_lossy().to_string();
    report.vault.name = if vault.config.vault_name.is_empty() {
        vault
            .root
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        vault.config.vault_name.clone()
    };
    report.vault.output_dir = vault.output_dir_abs().to_string_lossy().to_string();

    report.summary.registered = rows.len() as u32;

    for r in &rows {
        let mut src_changed = false;
        let mut out_modified = false;

        if !opts.no_stat {
            // Source change detection (by hash).
            let abs = vault.absolutize(&r.input_path);
            if let Ok(bytes) = std::fs::read(&abs) {
                let h = crate::convert::pipeline::sha256_hex(&bytes);
                src_changed = match &r.last_src_hash {
                    Some(prev) => prev != &h,
                    None => true,
                };
            }
            // Output drift detection (by hash).
            if let Some(rel) = &r.output_path {
                let outp = vault.output_dir_abs().join(rel);
                if let Ok(bytes) = std::fs::read(&outp) {
                    let h = crate::convert::pipeline::sha256_hex(&bytes);
                    out_modified = match &r.last_out_hash {
                        Some(prev) => prev != &h,
                        None => false,
                    };
                }
            }
        }

        match r.status {
            FileStatus::Pending => report.summary.pending += 1,
            FileStatus::Converted => {
                report.summary.converted += 1;
                if src_changed && out_modified {
                    report.summary.conflict += 1;
                } else if src_changed {
                    report.summary.reconvert += 1;
                } else if out_modified {
                    report.summary.output_modified += 1;
                }
            }
            FileStatus::Failed => report.summary.failed += 1,
            FileStatus::NeedsOcr => report.summary.needs_ocr += 1,
            FileStatus::Unsupported => report.summary.unsupported += 1,
            FileStatus::Missing => report.summary.missing += 1,
            FileStatus::Excluded => report.summary.excluded += 1,
            FileStatus::TooLarge => report.summary.too_large += 1,
            FileStatus::Conflict => report.summary.conflict += 1,
        }

        report.files.push(FileBlock {
            input_path: r.input_path.clone(),
            output_path: r.output_path.clone(),
            status: r.status.as_str().to_string(),
            source_changed: src_changed,
            output_modified: out_modified,
        });
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trip() {
        for s in [
            FileStatus::Pending,
            FileStatus::Converted,
            FileStatus::Failed,
            FileStatus::Unsupported,
            FileStatus::Excluded,
            FileStatus::TooLarge,
            FileStatus::NeedsOcr,
            FileStatus::Missing,
            FileStatus::Conflict,
        ] {
            assert_eq!(FileStatus::from_str(s.as_str()).unwrap(), s);
        }
    }
}
