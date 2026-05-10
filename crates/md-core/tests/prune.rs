//! `md prune` removes rows in non-`converted` states.

use md_core::convert::{run_convert, ConvertOptions};
use md_core::extract_core::ExtractorRegistry;
use md_core::registry::{add_paths, prune, AddOptions, PruneOptions};
use md_core::status::FileStatus;
use md_core::MdVault;
use rusqlite::params;

fn write(path: &std::path::Path, body: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn count(v: &MdVault, sql: &str) -> i64 {
    v.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// Build a vault containing one of each useful state:
///   - 1 converted (ok.md)
///   - 1 failed (synthetic dir-as-input)
///   - 1 missing (was converted then deleted)
fn setup_with_mixed_states() -> (tempfile::TempDir, MdVault) {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = MdVault::init(dir.path(), false).unwrap();

    write(&dir.path().join("ok.md"), "# OK\n\nbody.");
    write(&dir.path().join("delete_me.md"), "# X\n\nbody.");
    add_paths(
        &vault,
        &[dir.path().to_path_buf()],
        &AddOptions {
            skip_unsupported: true,
            ..Default::default()
        },
    )
    .unwrap();

    // Synthetic failed row: register a directory path by hand, run convert.
    std::fs::create_dir_all(dir.path().join("dir_input")).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    vault
        .conn
        .execute(
            "INSERT INTO outputs (input_path, added_at, status, attempts)
             VALUES ('dir_input', ?1, 'pending', 0)",
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

    // Now: ok.md=converted, delete_me.md=converted, dir_input=failed.
    std::fs::remove_file(dir.path().join("delete_me.md")).unwrap();
    run_convert(
        &mut vault,
        &ExtractorRegistry::standard(),
        &ConvertOptions::default(),
        None,
    )
    .unwrap();
    // Now: ok.md=converted, delete_me.md=missing, dir_input=failed.
    (dir, vault)
}

#[test]
fn default_targets_missing() {
    let (_dir, vault) = setup_with_mixed_states();
    let r = prune(&vault, &PruneOptions::default()).unwrap();
    assert_eq!(r.removed, 1);
    assert_eq!(count(&vault, "SELECT COUNT(*) FROM outputs"), 2);
    assert_eq!(
        count(
            &vault,
            "SELECT COUNT(*) FROM outputs WHERE status='missing'"
        ),
        0
    );
}

#[test]
fn dry_run_makes_no_changes() {
    let (_dir, vault) = setup_with_mixed_states();
    let before = count(&vault, "SELECT COUNT(*) FROM outputs");
    let r = prune(
        &vault,
        &PruneOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.removed, 1);
    assert_eq!(count(&vault, "SELECT COUNT(*) FROM outputs"), before);
}

#[test]
fn status_filter_targets_specific_state() {
    let (_dir, vault) = setup_with_mixed_states();
    let r = prune(
        &vault,
        &PruneOptions {
            status: Some(FileStatus::Failed),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.removed, 1);
    assert_eq!(
        count(&vault, "SELECT COUNT(*) FROM outputs WHERE status='failed'"),
        0
    );
    assert_eq!(count(&vault, "SELECT COUNT(*) FROM outputs"), 2);
}

#[test]
fn all_non_converted_clears_everything_except_converted() {
    let (_dir, vault) = setup_with_mixed_states();
    let r = prune(
        &vault,
        &PruneOptions {
            all_non_converted: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.removed, 2);
    assert_eq!(
        count(&vault, "SELECT COUNT(*) FROM outputs"),
        1,
        "only the converted row remains"
    );
}

#[test]
fn prune_does_not_remove_converted_rows() {
    let (_dir, vault) = setup_with_mixed_states();
    let _ = prune(
        &vault,
        &PruneOptions {
            all_non_converted: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        count(
            &vault,
            "SELECT COUNT(*) FROM outputs WHERE status='converted'"
        ),
        1
    );
}
