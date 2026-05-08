//! Parallel and sequential extraction paths must produce identical outcomes.

mod common;

use common::{run_index_stub, write};
use rag_core::config::Config;
use rag_core::index::{IndexOptions, Outcome};
use rag_core::registry::{add_paths, AddOptions};
use rag_core::Vault;

fn fresh_vault_with_files(extract_concurrency: u32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(dir.path(), false).unwrap();
    Config::set(
        &vault.conn,
        "indexing.extract_concurrency",
        serde_json::json!(extract_concurrency),
    )
    .unwrap();

    // A representative mix: indexable, empty (failed), unsupported, big enough
    // to chunk meaningfully.
    write(
        dir.path(),
        "a.md",
        "# A\n\n## sec1\n\nbody for a one.\n\n## sec2\n\nbody for a two.",
    );
    write(
        dir.path(),
        "b.md",
        "# B\n\n## sec1\n\nbody for b one.\n\n## sec2\n\nbody for b two.",
    );
    write(dir.path(), "c.md", "# C\n\n## sec1\n\nbody for c one.");
    write(dir.path(), "empty.md", ""); // → failed
    write(dir.path(), "img.png", "x"); // → unsupported (with skip_unsupported=false)

    add_paths(&vault, &[dir.path().to_path_buf()], &AddOptions::default()).unwrap();
    drop(vault);
    dir
}

#[test]
fn parallel_and_sequential_produce_same_outcomes() {
    fn outcomes_by_path(dir: &tempfile::TempDir) -> Vec<(String, Outcome)> {
        let mut vault = Vault::open(dir.path()).unwrap();
        let report = run_index_stub(&mut vault, IndexOptions::default());
        let mut out: Vec<(String, Outcome)> = report
            .results
            .into_iter()
            .map(|r| (r.path, r.outcome))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    let seq_dir = fresh_vault_with_files(1);
    let par_dir = fresh_vault_with_files(4);

    let seq = outcomes_by_path(&seq_dir);
    let par = outcomes_by_path(&par_dir);

    assert_eq!(
        seq, par,
        "parallel and sequential must produce identical (path, outcome) sets"
    );
}

#[test]
fn parallel_path_indexes_correctly() {
    let dir = fresh_vault_with_files(4);
    let mut vault = Vault::open(dir.path()).unwrap();
    let report = run_index_stub(&mut vault, IndexOptions::default());

    // 3 markdown files index, empty.md → failed, img.png → unsupported.
    assert_eq!(report.summary.indexed, 3);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(report.summary.unsupported, 1);

    // Results are sorted deterministically by path even though workers may
    // finish out of order.
    let paths: Vec<&str> = report.results.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, vec!["a.md", "b.md", "c.md", "empty.md", "img.png"]);

    // Consistency invariant must still hold across the parallel path.
    let chunks: i64 = vault
        .conn
        .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
        .unwrap();
    let vectors: i64 = vault
        .conn
        .query_row("SELECT COUNT(*) FROM chunk_vectors", [], |r| r.get(0))
        .unwrap();
    let fts: i64 = vault
        .conn
        .query_row("SELECT COUNT(*) FROM chunk_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(chunks, vectors);
    assert_eq!(chunks, fts);
    assert!(chunks > 0);
}
