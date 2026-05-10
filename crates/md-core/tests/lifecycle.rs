//! End-to-end test of md's full lifecycle: init → add → convert →
//! whence → conflict resolution.

use md_core::convert::{run_convert, ConvertOptions, Outcome};
use md_core::registry::{add_paths, AddOptions};
use md_core::status::FileStatus;
use md_core::whence::whence;
use md_core::{extract_core::ExtractorRegistry, MdVault};

fn write(path: &std::path::Path, body: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn count(v: &MdVault, sql: &str) -> i64 {
    v.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn registry() -> ExtractorRegistry {
    ExtractorRegistry::standard()
}

#[test]
fn full_convert_and_whence() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = MdVault::init(dir.path(), false).unwrap();

    write(&dir.path().join("docs/a.md"), "# A\n\n## sec\n\nbody one.");
    write(&dir.path().join("docs/b.txt"), "plain text body two");

    add_paths(
        &vault,
        &[dir.path().to_path_buf()],
        &AddOptions {
            skip_unsupported: true,
            ..Default::default()
        },
    )
    .unwrap();

    let report = run_convert(&mut vault, &registry(), &ConvertOptions::default(), None).unwrap();
    assert_eq!(report.summary.converted, 2);
    assert_eq!(report.summary.failed, 0);

    let converted: Vec<&str> = report
        .results
        .iter()
        .filter(|r| r.outcome == Outcome::Converted)
        .map(|r| r.input_path.as_str())
        .collect();
    assert!(converted.contains(&"docs/a.md"));
    assert!(converted.contains(&"docs/b.txt"));

    // Output files exist on disk under output_dir.
    let out_dir = vault.output_dir_abs();
    assert!(out_dir.join("docs/a.md.md").is_file());
    assert!(out_dir.join("docs/b.txt.md").is_file());

    // whence via DB
    let r = whence(Some(&vault), &out_dir.join("docs/a.md.md"))
        .unwrap()
        .unwrap();
    assert_eq!(r.source, "docs/a.md");
    assert_eq!(r.via, "db");

    // whence via annotation: copy out, ask without vault
    let copied = dir.path().join("copied.md");
    std::fs::copy(out_dir.join("docs/a.md.md"), &copied).unwrap();
    let r = whence(None, &copied).unwrap().unwrap();
    assert_eq!(r.source, "docs/a.md");
    assert_eq!(r.via, "annotation");
}

#[test]
fn second_run_is_a_noop_when_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = MdVault::init(dir.path(), false).unwrap();
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
    let r1 = run_convert(&mut vault, &registry(), &ConvertOptions::default(), None).unwrap();
    assert_eq!(r1.summary.converted, 1);

    let r2 = run_convert(&mut vault, &registry(), &ConvertOptions::default(), None).unwrap();
    assert_eq!(r2.summary.converted, 0);
    assert_eq!(r2.summary.skipped, 1);
}

#[test]
fn modified_source_re_converts_only_that_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = MdVault::init(dir.path(), false).unwrap();
    write(&dir.path().join("a.md"), "# A original\n\nbody.");
    write(&dir.path().join("b.md"), "# B original\n\nbody.");
    add_paths(
        &vault,
        &[dir.path().to_path_buf()],
        &AddOptions {
            skip_unsupported: true,
            ..Default::default()
        },
    )
    .unwrap();
    run_convert(&mut vault, &registry(), &ConvertOptions::default(), None).unwrap();

    write(&dir.path().join("a.md"), "# A edited\n\nnew body.");
    let r = run_convert(&mut vault, &registry(), &ConvertOptions::default(), None).unwrap();
    assert_eq!(r.summary.converted, 1);
    assert_eq!(r.summary.skipped, 1);
}

#[test]
fn conflict_when_both_source_and_output_edited() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = MdVault::init(dir.path(), false).unwrap();
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
    run_convert(&mut vault, &registry(), &ConvertOptions::default(), None).unwrap();

    // Hand-edit the output
    let out_path = vault.output_dir_abs().join("a.md.md");
    let mut existing = std::fs::read_to_string(&out_path).unwrap();
    existing.push_str("\n<!-- user edit -->\n");
    std::fs::write(&out_path, &existing).unwrap();

    // Edit source
    write(&dir.path().join("a.md"), "# A re-edited\n\nnew body.");

    let r = run_convert(&mut vault, &registry(), &ConvertOptions::default(), None).unwrap();
    assert_eq!(r.summary.conflict, 1);
    assert_eq!(r.summary.converted, 0);

    // The .new sibling should exist
    let new_path = vault.output_dir_abs().join("a.md.md.new");
    assert!(
        new_path.is_file(),
        "expected .new file at {}",
        new_path.display()
    );

    // status persisted as 'conflict'
    let n_conflict = count(
        &vault,
        "SELECT COUNT(*) FROM outputs WHERE status='conflict'",
    );
    assert_eq!(n_conflict, 1);

    // --keep-existing resolves to converted status
    let r = run_convert(
        &mut vault,
        &registry(),
        &ConvertOptions {
            keep_existing: true,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(r.summary.converted, 1);
    let n_conflict = count(
        &vault,
        "SELECT COUNT(*) FROM outputs WHERE status='conflict'",
    );
    assert_eq!(n_conflict, 0);
    let _ = FileStatus::Converted; // silence unused-import warning
}
