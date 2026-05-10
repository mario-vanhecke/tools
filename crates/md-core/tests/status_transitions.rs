//! Each non-`converted` status must be reachable and resolvable.

use md_core::config::Config;
use md_core::convert::{run_convert, ConvertOptions};
use md_core::extract_core::ExtractorRegistry;
use md_core::registry::{add_paths, AddOptions};
use md_core::status::FileStatus;
use md_core::MdVault;
use rusqlite::params;

fn write(path: &std::path::Path, body: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn count(v: &MdVault, status: FileStatus) -> i64 {
    v.conn
        .query_row(
            "SELECT COUNT(*) FROM outputs WHERE status = ?1",
            params![status.as_str()],
            |r| r.get(0),
        )
        .unwrap()
}

fn open() -> (tempfile::TempDir, MdVault) {
    let dir = tempfile::tempdir().unwrap();
    let vault = MdVault::init(dir.path(), false).unwrap();
    (dir, vault)
}

#[test]
fn pending_to_converted() {
    let (dir, mut vault) = open();
    write(&dir.path().join("a.md"), "# A\n\nbody.");
    add_paths(
        &vault,
        &[dir.path().to_path_buf()],
        &AddOptions {
            skip_unsupported: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Pending), 1);
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Converted), 1);
}

#[test]
fn unsupported_extension_then_recovers_after_config_change() {
    let (dir, mut vault) = open();
    // .json is not in the default supported extensions
    write(&dir.path().join("a.json"), "{\"x\":1}");
    add_paths(
        &vault,
        &[dir.path().to_path_buf()],
        // skip_unsupported=false so the row IS created
        &AddOptions::default(),
    )
    .unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Unsupported), 1);

    // Add .json to supported. md doesn't have a JSON extractor, so this
    // transitions to 'failed' (no_extractor_available) — proving the
    // unsupported state was cleared on re-evaluation.
    Config::set(
        &vault.conn,
        "files.supported_extensions",
        serde_json::json!(["md", "markdown", "docx", "pdf", "epub", "txt", "json"]),
    )
    .unwrap();
    let mut vault = MdVault::open(dir.path()).unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Unsupported), 0);
}

#[test]
fn excluded_then_indexed_after_unset() {
    let (dir, vault) = open();
    write(&dir.path().join("a.txt"), "plaintext content body.");
    add_paths(&vault, &[dir.path().to_path_buf()], &AddOptions::default()).unwrap();

    Config::set(
        &vault.conn,
        "files.excluded_extensions",
        serde_json::json!(["txt"]),
    )
    .unwrap();
    let mut vault = MdVault::open(dir.path()).unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Excluded), 1);

    Config::set(
        &vault.conn,
        "files.excluded_extensions",
        serde_json::json!([]),
    )
    .unwrap();
    let mut vault = MdVault::open(dir.path()).unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Converted), 1);
}

#[test]
fn too_large_then_size_cap_raised() {
    let (dir, _vault) = open();
    let body = "x".repeat(100_000);
    write(&dir.path().join("big.md"), &format!("# Big\n\n{body}"));
    let vault = MdVault::open(dir.path()).unwrap();
    add_paths(&vault, &[dir.path().to_path_buf()], &AddOptions::default()).unwrap();

    Config::set(&vault.conn, "files.size_cap_bytes", serde_json::json!(1024)).unwrap();
    let mut vault = MdVault::open(dir.path()).unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::TooLarge), 1);

    Config::set(
        &vault.conn,
        "files.size_cap_bytes",
        serde_json::json!(10_000_000),
    )
    .unwrap();
    let mut vault = MdVault::open(dir.path()).unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Converted), 1);
}

#[test]
fn missing_then_restored() {
    let (dir, mut vault) = open();
    write(&dir.path().join("a.md"), "# A\n\nbody one two three.");
    add_paths(
        &vault,
        &[dir.path().to_path_buf()],
        &AddOptions {
            skip_unsupported: true,
            ..Default::default()
        },
    )
    .unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Converted), 1);

    std::fs::remove_file(dir.path().join("a.md")).unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Missing), 1);

    write(&dir.path().join("a.md"), "# A back\n\nfresh content.");
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Converted), 1);
    assert_eq!(count(&vault, FileStatus::Missing), 0);
}

#[test]
fn path_not_a_file_to_failed() {
    // Insert a row by hand whose input_path points at a directory.
    let (dir, mut vault) = open();
    std::fs::create_dir_all(dir.path().join("not_a_file_dir")).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    vault
        .conn
        .execute(
            "INSERT INTO outputs (input_path, added_at, status, attempts)
             VALUES ('not_a_file_dir', ?1, 'pending', 0)",
            params![now],
        )
        .unwrap();

    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Failed), 1);
    let detail: String = vault
        .conn
        .query_row("SELECT status_detail FROM outputs LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(detail, "path_not_a_file");
}

#[test]
fn failed_skipped_unless_retry_failed() {
    let (dir, mut vault) = open();
    // A .md file with invalid UTF-8 — the markdown extractor returns
    // Failed { detail: "not_utf8", ... }. After the first run the row
    // sits in `failed` status as a regular file (so subsequent path
    // checks pass), exercising the Failed-skip-if-not-retry branch.
    std::fs::write(dir.path().join("bad.md"), [0xFF, 0xFE, 0xFD, 0x00]).unwrap();
    add_paths(
        &vault,
        &[dir.path().to_path_buf()],
        &AddOptions {
            skip_unsupported: true,
            ..Default::default()
        },
    )
    .unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(count(&vault, FileStatus::Failed), 1);

    // Default: failed rows are skipped on subsequent runs.
    let r = run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(r.summary.skipped, 1);
    assert_eq!(r.summary.failed, 0);

    // With --retry-failed: row is retried; extraction fails again because
    // the file is still non-UTF-8.
    let r = run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions {
            retry_failed: true,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(r.summary.failed, 1);
}

#[test]
fn conflict_resolved_via_overwrite() {
    let (dir, mut vault) = open();
    write(&dir.path().join("a.md"), "# A original\n\nbody.");
    add_paths(
        &vault,
        &[dir.path().to_path_buf()],
        &AddOptions {
            skip_unsupported: true,
            ..Default::default()
        },
    )
    .unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();

    // Hand-edit output and source to provoke a conflict.
    let out_path = vault.output_dir_abs().join("a.md.md");
    let mut out = std::fs::read_to_string(&out_path).unwrap();
    out.push_str("\n<!-- user edit -->\n");
    std::fs::write(&out_path, &out).unwrap();
    write(&dir.path().join("a.md"), "# A re-edited\n\nnew body.");

    let r = run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(r.summary.conflict, 1);

    // --overwrite: discard the user's edits, write the fresh conversion.
    let r = run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions {
            overwrite: true,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(r.summary.converted, 1);
    assert_eq!(count(&vault, FileStatus::Conflict), 0);

    // Output should contain the new content, NOT the user's hand edit.
    let final_out = std::fs::read_to_string(&out_path).unwrap();
    assert!(final_out.contains("re-edited"));
    assert!(!final_out.contains("user edit"));
}
