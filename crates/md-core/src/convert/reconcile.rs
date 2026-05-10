use super::pipeline::{sha256_hex, write_conflict, write_converted, write_status_only};
use super::{ConvertOptions, FileResult, Outcome};
use crate::annotation::Annotation;
use crate::error::Result;
use crate::registry::OutputRow;
use crate::status::FileStatus;
use crate::vault::MdVault;
use extract_core::{ExtractionResult, Extractor, ExtractorRegistry};
use std::path::{Path, PathBuf};

pub fn process_one(
    vault: &MdVault,
    row: &OutputRow,
    extractors: &ExtractorRegistry,
    opts: &ConvertOptions,
) -> Result<FileResult> {
    let abs = vault.absolutize(&row.input_path);
    let meta = std::fs::metadata(&abs);

    let meta = match meta {
        Ok(m) => m,
        Err(_) => {
            write_status_only(
                vault,
                row,
                FileStatus::Missing,
                None,
                None,
                row.status != FileStatus::Missing,
            )?;
            return Ok(FileResult {
                input_path: row.input_path.clone(),
                output_path: row.output_path.clone(),
                outcome: Outcome::Missing,
                status_detail: None,
                status_note: None,
            });
        }
    };
    if !meta.is_file() {
        write_status_only(
            vault,
            row,
            FileStatus::Failed,
            Some("path_not_a_file"),
            Some("registered path is not a regular file"),
            true,
        )?;
        return Ok(FileResult {
            input_path: row.input_path.clone(),
            output_path: row.output_path.clone(),
            outcome: Outcome::Failed,
            status_detail: Some("path_not_a_file".to_string()),
            status_note: Some("registered path is not a regular file".to_string()),
        });
    }

    let ext = Path::new(&row.input_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if !vault
        .config
        .files
        .supported_extensions
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&ext))
    {
        write_status_only(
            vault,
            row,
            FileStatus::Unsupported,
            Some("extension_not_supported"),
            None,
            false,
        )?;
        return Ok(FileResult {
            input_path: row.input_path.clone(),
            output_path: row.output_path.clone(),
            outcome: Outcome::Unsupported,
            status_detail: Some("extension_not_supported".to_string()),
            status_note: None,
        });
    }
    if vault
        .config
        .files
        .excluded_extensions
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&ext))
    {
        write_status_only(
            vault,
            row,
            FileStatus::Excluded,
            Some("extension_excluded_by_config"),
            None,
            false,
        )?;
        return Ok(FileResult {
            input_path: row.input_path.clone(),
            output_path: row.output_path.clone(),
            outcome: Outcome::Excluded,
            status_detail: Some("extension_excluded_by_config".to_string()),
            status_note: None,
        });
    }
    if meta.len() > vault.config.files.size_cap_bytes {
        write_status_only(
            vault,
            row,
            FileStatus::TooLarge,
            Some("size_exceeds_cap"),
            None,
            false,
        )?;
        return Ok(FileResult {
            input_path: row.input_path.clone(),
            output_path: row.output_path.clone(),
            outcome: Outcome::TooLarge,
            status_detail: Some("size_exceeds_cap".to_string()),
            status_note: None,
        });
    }

    let src_bytes = std::fs::read(&abs)?;
    let src_hash = sha256_hex(&src_bytes);
    let src_mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    let src_size = meta.len() as i64;

    let src_changed = match &row.last_src_hash {
        Some(h) => h != &src_hash,
        None => true,
    };

    // Output state: none, untouched, hand-edited.
    let (out_changed, out_abs) = output_state(vault, row);

    // Decision tree per the design doc — see ADR pending in docs/adr/.
    if row.status == FileStatus::Failed && !opts.retry_failed && !opts.force {
        return Ok(FileResult {
            input_path: row.input_path.clone(),
            output_path: row.output_path.clone(),
            outcome: Outcome::Skipped,
            status_detail: row.status_detail.clone(),
            status_note: row.status_note.clone(),
        });
    }

    // Conflict state: was already in conflict OR (src_changed && out_changed).
    if row.status == FileStatus::Conflict || (src_changed && out_changed) {
        if opts.overwrite {
            // Fall through to re-convert path.
        } else if opts.keep_existing {
            // Treat output as authoritative: clear the conflict by recording
            // the *current* output hash + source hash as the new baseline.
            if let Some(out) = &out_abs {
                let out_bytes = std::fs::read(out).unwrap_or_default();
                let out_hash = sha256_hex(&out_bytes);
                let now = chrono::Utc::now().timestamp_millis();
                vault.conn.execute(
                    "UPDATE outputs SET
                        status = 'converted',
                        status_detail = NULL,
                        status_note = NULL,
                        last_src_hash = ?1, last_src_mtime = ?2, last_src_size = ?3,
                        last_out_hash = ?4, last_converted = ?5, last_attempt = ?5
                     WHERE id = ?6",
                    rusqlite::params![src_hash, src_mtime_ms, src_size, out_hash, now, row.id],
                )?;
            }
            return Ok(FileResult {
                input_path: row.input_path.clone(),
                output_path: row.output_path.clone(),
                outcome: Outcome::Converted, // we treat the user's hand-edited file as the converted baseline
                status_detail: Some("kept_existing_output".to_string()),
                status_note: None,
            });
        } else {
            // Don't trample. Mark conflict, optionally write `.new` side-by-side.
            return handle_conflict(vault, row, extractors, &ext, &abs, &src_bytes, &src_hash);
        }
    }

    // Output-modified-but-source-unchanged: just skip. User has taken
    // ownership of the output; we should not re-convert.
    if !src_changed && out_changed {
        return Ok(FileResult {
            input_path: row.input_path.clone(),
            output_path: row.output_path.clone(),
            outcome: Outcome::Skipped,
            status_detail: Some("output_modified".to_string()),
            status_note: None,
        });
    }

    // Already converted, source unchanged, output unchanged → clean. Skip
    // unless `--force`.
    if row.status == FileStatus::Converted && !src_changed && !out_changed && !opts.force {
        return Ok(FileResult {
            input_path: row.input_path.clone(),
            output_path: row.output_path.clone(),
            outcome: Outcome::Skipped,
            status_detail: None,
            status_note: None,
        });
    }

    // Process: extract → annotate → write.
    let extractor = match extractors.for_extension(&ext) {
        Some(e) => e.clone(),
        None => {
            let detail = "no_extractor_available";
            let message = format!("no extractor registered for .{}", ext);
            write_status_only(
                vault,
                row,
                FileStatus::Failed,
                Some(detail),
                Some(&message),
                true,
            )?;
            return Ok(FileResult {
                input_path: row.input_path.clone(),
                output_path: row.output_path.clone(),
                outcome: Outcome::Failed,
                status_detail: Some(detail.to_string()),
                status_note: Some(message),
            });
        }
    };

    match extractor.extract(&abs) {
        ExtractionResult::NeedsOcr => {
            write_status_only(
                vault,
                row,
                FileStatus::NeedsOcr,
                Some("no_extractable_text"),
                None,
                false,
            )?;
            Ok(FileResult {
                input_path: row.input_path.clone(),
                output_path: row.output_path.clone(),
                outcome: Outcome::NeedsOcr,
                status_detail: Some("no_extractable_text".to_string()),
                status_note: None,
            })
        }
        ExtractionResult::Failed { detail, message } => {
            write_status_only(
                vault,
                row,
                FileStatus::Failed,
                Some(&detail),
                Some(&message),
                true,
            )?;
            Ok(FileResult {
                input_path: row.input_path.clone(),
                output_path: row.output_path.clone(),
                outcome: Outcome::Failed,
                status_detail: Some(detail),
                status_note: Some(message),
            })
        }
        ExtractionResult::Ok(extracted) => {
            // Plan output filename + write.
            let extractor_name = extractor_name_for(&extractor);
            let output_rel = plan_output_path(vault, &row.input_path);
            let output_abs = vault.output_dir_abs().join(&output_rel);
            let output_rel_str = output_rel.to_string_lossy().to_string();

            let body = if vault.config.output.annotate {
                let ann = Annotation {
                    source: row.input_path.clone(),
                    source_hash: src_hash.clone(),
                    extractor: extractor_name.to_string(),
                    converted_at_ms: chrono::Utc::now().timestamp_millis(),
                };
                format!("{}{}", ann.render(), extracted.markdown)
            } else {
                extracted.markdown
            };
            let body_bytes = body.into_bytes();

            write_converted(
                vault,
                row,
                &output_rel_str,
                &output_abs,
                &body_bytes,
                src_mtime_ms,
                src_size,
                &src_hash,
                extractor_name,
            )?;

            Ok(FileResult {
                input_path: row.input_path.clone(),
                output_path: Some(output_rel_str),
                outcome: Outcome::Converted,
                status_detail: None,
                status_note: None,
            })
        }
    }
}

fn handle_conflict(
    vault: &MdVault,
    row: &OutputRow,
    extractors: &ExtractorRegistry,
    ext: &str,
    abs: &Path,
    _src_bytes: &[u8],
    src_hash: &str,
) -> Result<FileResult> {
    // Try a fresh extraction so we can write a `<output>.new` for the user
    // to diff against. Failures here just mean "we couldn't even produce a
    // candidate"; the conflict status still gets recorded.
    let mut new_path: Option<PathBuf> = None;
    let mut new_bytes: Option<Vec<u8>> = None;
    if let Some(extractor) = extractors.for_extension(ext) {
        if let ExtractionResult::Ok(extracted) = extractor.extract(abs) {
            let extractor_name = extractor_name_for(extractor);
            let body = if vault.config.output.annotate {
                let ann = Annotation {
                    source: row.input_path.clone(),
                    source_hash: src_hash.to_string(),
                    extractor: extractor_name.to_string(),
                    converted_at_ms: chrono::Utc::now().timestamp_millis(),
                };
                format!("{}{}", ann.render(), extracted.markdown)
            } else {
                extracted.markdown
            };
            let output_rel = plan_output_path(vault, &row.input_path);
            let mut p = vault.output_dir_abs().join(&output_rel);
            // Append `.new` so we don't trample the user's edits.
            let mut filename = p.file_name().map(|n| n.to_os_string()).unwrap_or_default();
            filename.push(".new");
            p.set_file_name(filename);
            new_path = Some(p);
            new_bytes = Some(body.into_bytes());
        }
    }

    let note = match &new_path {
        Some(p) => format!(
            "source and output both changed since last convert; new conversion written to {} for your review (run with --overwrite to discard your edits, --keep-existing to accept your edits)",
            p.display()
        ),
        None => "source and output both changed since last convert; no extractor available to produce a side-by-side diff (run with --overwrite to force or --keep-existing to accept your edits)".to_string(),
    };

    write_conflict(vault, row, new_path.as_deref(), new_bytes.as_deref(), &note)?;

    Ok(FileResult {
        input_path: row.input_path.clone(),
        output_path: row.output_path.clone(),
        outcome: Outcome::Conflict,
        status_detail: Some("output_modified_with_source_change".to_string()),
        status_note: Some(note),
    })
}

/// Inspect the on-disk state of the output file (if known).
/// Returns (output_was_modified_since_last_convert, absolute_path).
fn output_state(vault: &MdVault, row: &OutputRow) -> (bool, Option<PathBuf>) {
    let Some(rel) = &row.output_path else {
        return (false, None);
    };
    let abs = vault.output_dir_abs().join(rel);
    let bytes = match std::fs::read(&abs) {
        Ok(b) => b,
        Err(_) => return (false, Some(abs)), // output missing — treat as untouched (will be re-created)
    };
    let cur_hash = sha256_hex(&bytes);
    let modified = match &row.last_out_hash {
        Some(h) => h != &cur_hash,
        None => false,
    };
    (modified, Some(abs))
}

/// Compute the output path (relative to `output_dir`) for a given input.
/// Default: replace the extension with `.md`. With `collision_aware_naming`,
/// we'd append the source extension; collision detection across rows is a
/// future enhancement (today's policy: replace extension; users with two
/// inputs that collide can rename one source).
fn plan_output_path(vault: &MdVault, input_path: &str) -> PathBuf {
    let p = Path::new(input_path);
    let stem_with_ext = if vault.config.output.collision_aware_naming {
        // foo.pdf → foo.pdf.md
        format!("{}.md", p.file_name().unwrap().to_string_lossy())
    } else {
        // foo.pdf → foo.md
        let stem = p.file_stem().unwrap().to_string_lossy();
        format!("{stem}.md")
    };
    match p.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(stem_with_ext),
        _ => PathBuf::from(stem_with_ext),
    }
}

/// Identify which concrete extractor produced a given Arc<dyn Extractor> by
/// matching its declared extension list. Crude but stable across the
/// extract-core registry's standard set.
fn extractor_name_for(e: &std::sync::Arc<dyn Extractor>) -> &'static str {
    match e.extensions().first().copied() {
        Some("pdf") => "pdf",
        Some("md") | Some("markdown") => "markdown",
        Some("txt") => "plaintext",
        Some("docx") | Some("epub") => "pandoc",
        _ => "other",
    }
}
