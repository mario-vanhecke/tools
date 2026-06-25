//! End-to-end document lifecycle: discover → present → modified → gone → prune,
//! driven through a local source.

use crawl_core::crawl::{self, RunOptions};
use crawl_core::registry::{self, sources::AddSourceOptions, DocQuery};
use crawl_core::source::{SourceKind, Strategy};
use crawl_core::{CrawlVault, DocStatus};
use serde_json::json;
use std::fs;
use std::path::Path;

fn write(p: &Path, contents: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, contents).unwrap();
}

fn setup() -> (tempfile::TempDir, CrawlVault, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let docs = tmp.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    let vault = CrawlVault::init(tmp.path(), false).unwrap();
    let docs_abs = docs.canonicalize().unwrap();
    (tmp, vault, docs_abs)
}

fn add_local(vault: &CrawlVault, root: &Path, strategy: Strategy) {
    registry::sources::add_source(
        &vault.conn,
        &AddSourceOptions {
            name: "docs".into(),
            kind: SourceKind::Local,
            uri: root.to_string_lossy().to_string(),
            strategy,
            config: json!({}),
            enabled: true,
        },
    )
    .unwrap();
}

fn count(vault: &CrawlVault, status: Option<DocStatus>) -> usize {
    registry::query_documents(
        &vault.conn,
        &DocQuery {
            status,
            ..Default::default()
        },
    )
    .unwrap()
    .len()
}

#[test]
fn full_lifecycle() {
    let (_tmp, vault, docs) = setup();
    write(&docs.join("a.pdf"), "alpha");
    write(&docs.join("b.txt"), "bravo");
    write(&docs.join("sub/c.md"), "charlie");
    write(&docs.join("ignore.bin"), "binary"); // unsupported extension

    add_local(&vault, &docs, Strategy::Recursive);

    // First crawl: three documents discovered, the .bin skipped.
    let r = crawl::run(&vault, &RunOptions::default()).unwrap();
    let s = &r.sources[0];
    assert_eq!(s.discovered, 3, "pdf+txt+md discovered");
    assert_eq!(s.skipped, 1, ".bin skipped");
    assert_eq!(count(&vault, Some(DocStatus::Present)), 3);

    // Re-crawl with no changes: nothing updated, still all present.
    let r = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(r.sources[0].discovered, 0);
    assert_eq!(r.sources[0].updated, 0);
    assert_eq!(count(&vault, Some(DocStatus::Present)), 3);

    // Modify one file (size changes -> detected without hashing).
    write(&docs.join("a.pdf"), "alpha-much-longer-now");
    let r = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(r.sources[0].updated, 1);
    assert_eq!(count(&vault, Some(DocStatus::Modified)), 1);

    // A clean re-crawl resolves modified back to present.
    let r = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(r.sources[0].updated, 0);
    assert_eq!(count(&vault, Some(DocStatus::Present)), 3);

    // Delete a file: a full crawl marks it gone.
    fs::remove_file(docs.join("b.txt")).unwrap();
    let r = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(r.sources[0].gone, 1);
    assert_eq!(count(&vault, Some(DocStatus::Gone)), 1);
    assert_eq!(count(&vault, None), 3, "gone rows are kept until pruned");

    // Prune removes the gone row.
    let p = registry::prune(&vault.conn, &registry::PruneOptions::default()).unwrap();
    assert_eq!(p.pruned, 1);
    assert_eq!(count(&vault, None), 2);
}

#[test]
fn hashing_detects_content_change_at_same_size() {
    let (_tmp, vault, docs) = setup();
    write(&docs.join("a.txt"), "AAAAA");
    add_local(&vault, &docs, Strategy::Recursive);

    let hash_opts = RunOptions {
        hash: Some(true),
        ..Default::default()
    };
    crawl::run(&vault, &hash_opts).unwrap();
    assert_eq!(count(&vault, Some(DocStatus::Present)), 1);

    // Same byte length, different content: only a hash run notices.
    write(&docs.join("a.txt"), "BBBBB");
    let r = crawl::run(&vault, &hash_opts).unwrap();
    assert_eq!(r.sources[0].updated, 1, "hash run sees the content change");
}

#[test]
fn dry_run_writes_nothing() {
    let (_tmp, vault, docs) = setup();
    write(&docs.join("a.pdf"), "x");
    add_local(&vault, &docs, Strategy::Recursive);

    let r = crawl::run(
        &vault,
        &RunOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.sources[0].discovered, 1, "reported as it would discover");
    assert_eq!(count(&vault, None), 0, "but nothing was written");
}
