//! Each crawl strategy is a different traversal policy; these tests pin the
//! observable difference between them on the same tree.

use crawl_core::crawl::{self, RunOptions};
use crawl_core::registry::{self, sources::AddSourceOptions, DocQuery};
use crawl_core::source::{SourceKind, Strategy};
use crawl_core::{CrawlVault, DocStatus};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

fn write(p: &Path, contents: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, contents).unwrap();
}

fn set_mtime(p: &Path, secs: u64) {
    let f = fs::File::options().write(true).open(p).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

fn setup() -> (tempfile::TempDir, CrawlVault, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let docs = tmp.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    let vault = CrawlVault::init(tmp.path(), false).unwrap();
    let docs_abs = docs.canonicalize().unwrap();
    (tmp, vault, docs_abs)
}

fn add(vault: &CrawlVault, root: &Path, strategy: Strategy, config: Value) {
    registry::sources::add_source(
        &vault.conn,
        &AddSourceOptions {
            name: "docs".into(),
            kind: SourceKind::Local,
            uri: root.to_string_lossy().to_string(),
            strategy,
            config,
            enabled: true,
        },
    )
    .unwrap();
}

fn names(vault: &CrawlVault) -> Vec<String> {
    let mut v: Vec<String> = registry::query_documents(&vault.conn, &DocQuery::default())
        .unwrap()
        .into_iter()
        .map(|d| d.rel_path.unwrap_or(d.name))
        .collect();
    v.sort();
    v
}

#[test]
fn recursive_finds_every_level() {
    let (_t, vault, docs) = setup();
    write(&docs.join("top.txt"), "1");
    write(&docs.join("a/mid.txt"), "2");
    write(&docs.join("a/b/deep.txt"), "3");
    add(&vault, &docs, Strategy::Recursive, json!({}));

    crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(names(&vault), vec!["a/b/deep.txt", "a/mid.txt", "top.txt"]);
}

#[test]
fn shallow_finds_only_top_level() {
    let (_t, vault, docs) = setup();
    write(&docs.join("top.txt"), "1");
    write(&docs.join("a/mid.txt"), "2");
    add(&vault, &docs, Strategy::Shallow, json!({}));

    let r = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(r.sources[0].discovered, 1);
    assert_eq!(names(&vault), vec!["top.txt"]);
}

#[test]
fn targeted_records_only_matching_globs() {
    let (_t, vault, docs) = setup();
    write(&docs.join("report.pdf"), "1");
    write(&docs.join("notes.txt"), "2");
    write(&docs.join("deck.pdf"), "3");
    add(
        &vault,
        &docs,
        Strategy::Targeted,
        json!({ "include_globs": ["*.pdf"] }),
    );

    let r = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(r.sources[0].discovered, 2);
    assert_eq!(r.sources[0].skipped, 1);
    assert_eq!(names(&vault), vec!["deck.pdf", "report.pdf"]);
}

#[test]
fn incremental_only_visits_files_newer_than_cutoff() {
    let (_t, vault, docs) = setup();
    let old = docs.join("old.txt");
    let new = docs.join("new.txt");
    write(&old, "old");
    write(&new, "new");
    set_mtime(&old, 1_000); // 1000 s after epoch
    set_mtime(&new, 3_000); // 3000 s after epoch

    // Cutoff at 2000 s (in ms) — only `new.txt` qualifies.
    add(
        &vault,
        &docs,
        Strategy::Incremental,
        json!({ "since_ms": 2_000_000i64 }),
    );

    let r = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(r.sources[0].discovered, 1);
    assert_eq!(names(&vault), vec!["new.txt"]);

    // Incremental is a partial pass: it must NOT mark the unseen old file gone
    // (it was simply never recorded), and a later full crawl picks both up.
    assert_eq!(
        registry::query_documents(
            &vault.conn,
            &DocQuery {
                status: Some(DocStatus::Gone),
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        0
    );
}

#[test]
fn shallow_does_not_mark_deeper_files_gone() {
    let (_t, vault, docs) = setup();
    write(&docs.join("top.txt"), "1");
    write(&docs.join("a/deep.txt"), "2");

    // First a full recursive crawl records both files.
    add(&vault, &docs, Strategy::Recursive, json!({}));
    crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(names(&vault).len(), 2);

    // Then a shallow override must not conclude the deep file is gone.
    let r = crawl::run(
        &vault,
        &RunOptions {
            strategy_override: Some(Strategy::Shallow),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.sources[0].gone, 0, "shallow pass cannot declare gone");
    assert_eq!(
        registry::query_documents(
            &vault.conn,
            &DocQuery {
                status: Some(DocStatus::Gone),
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        0
    );
}
