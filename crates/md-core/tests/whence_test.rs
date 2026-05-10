//! `whence` resolution edge cases: DB hit, annotation fallback,
//! detached-file lookup, missing-source.

use md_core::convert::{run_convert, ConvertOptions};
use md_core::extract_core::ExtractorRegistry;
use md_core::registry::{add_paths, AddOptions};
use md_core::whence::whence;
use md_core::MdVault;

fn write(path: &std::path::Path, body: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn setup() -> (tempfile::TempDir, MdVault) {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = MdVault::init(dir.path(), false).unwrap();
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
    (dir, vault)
}

#[test]
fn db_hit_with_full_hash_and_vault_root() {
    let (_dir, vault) = setup();
    let out = vault.output_dir_abs().join("a.md.md");
    let r = whence(Some(&vault), &out).unwrap().expect("expected a hit");
    assert_eq!(r.via, "db");
    assert_eq!(r.source, "a.md");
    assert!(r.vault_root.is_some());
    // DB stores the full SHA-256 hex (64 chars).
    assert_eq!(r.source_hash.as_ref().unwrap().len(), 64);
}

#[test]
fn annotation_fallback_when_no_vault_provided() {
    let (_dir, vault) = setup();
    let out = vault.output_dir_abs().join("a.md.md");
    let r = whence(None, &out).unwrap().expect("expected a hit");
    assert_eq!(r.via, "annotation");
    assert_eq!(r.source, "a.md");
    assert!(r.vault_root.is_none());
    // Annotation only stores the truncated 12-char hash.
    assert_eq!(r.source_hash.as_ref().unwrap().len(), 12);
}

#[test]
fn annotation_fallback_when_path_outside_vault() {
    let (_dir, vault) = setup();
    let original = vault.output_dir_abs().join("a.md.md");
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::copy(&original, outside.path()).unwrap();

    // Even though we PASS a vault, this file isn't under output_dir, so
    // DB lookup misses and we fall through to annotation parsing.
    let r = whence(Some(&vault), outside.path())
        .unwrap()
        .expect("expected an annotation-based hit");
    assert_eq!(r.via, "annotation");
    assert_eq!(r.source, "a.md");
}

#[test]
fn no_annotation_no_db_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain.md");
    write(&plain, "# Just markdown\n\nNo annotation here.");
    let r = whence(None, &plain).unwrap();
    assert!(
        r.is_none(),
        "plain markdown without annotation must not match"
    );
}

#[test]
fn missing_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let r = whence(None, &dir.path().join("does-not-exist.md")).unwrap();
    assert!(r.is_none());
}

#[test]
fn db_lookup_works_after_file_moved_inside_output_dir() {
    // Simulate: convert produces output, then we look up via the actual
    // path. A symlink or rename outside output_dir should still resolve
    // via annotation as a fallback path.
    let (_dir, vault) = setup();
    let original = vault.output_dir_abs().join("a.md.md");
    // Move INSIDE the same vault output_dir, but to a different name.
    let moved = vault.output_dir_abs().join("renamed.md");
    std::fs::rename(&original, &moved).unwrap();
    // DB still has the old output_path; the moved path doesn't match
    // anything in DB. Fallback should be the annotation.
    let r = whence(Some(&vault), &moved)
        .unwrap()
        .expect("annotation fallback");
    assert_eq!(r.via, "annotation");
    assert_eq!(r.source, "a.md");
}
