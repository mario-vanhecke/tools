//! Two `md convert` invocations must not corrupt the manifest. With
//! `--no-wait`, the loser exits with LockContention.

use md_core::convert::{run_convert, ConvertOptions};
use md_core::extract_core::ExtractorRegistry;
use md_core::registry::{add_paths, AddOptions};
use md_core::vault_core::FileExt;
use md_core::MdVault;
use std::fs::OpenOptions;
use std::sync::Arc;

fn write(path: &std::path::Path, body: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

#[test]
fn no_wait_loser_returns_lock_contention() {
    let dir = tempfile::tempdir().unwrap();
    let vault = MdVault::init(dir.path(), false).unwrap();
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

    // Hold the convert lock manually so any concurrent run must observe it.
    let lock_path = vault.convert_lock_path();
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    lock_file.lock_exclusive().unwrap();

    drop(vault);

    let p = Arc::new(dir.path().to_path_buf());
    let p2 = p.clone();
    let h = std::thread::spawn(move || {
        let mut v = MdVault::open(&p2).unwrap();
        run_convert(
            &mut v,
            &ExtractorRegistry::standard(),
            &ConvertOptions {
                no_wait: true,
                ..Default::default()
            },
            None,
        )
    });

    let res = h.join().unwrap();
    match res {
        Err(md_core::Error::Vault(md_core::vault_core::Error::LockContention)) => {} // expected
        other => panic!("expected LockContention, got {other:?}"),
    }

    let _ = FileExt::unlock(&lock_file);
    let _ = p;
}

#[test]
fn waiter_eventually_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    {
        let vault = MdVault::init(dir.path(), false).unwrap();
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
    }

    let dir_path = Arc::new(dir.path().to_path_buf());
    let p1 = dir_path.clone();
    let p2 = dir_path.clone();

    let h1 = std::thread::spawn(move || {
        let mut v = MdVault::open(&p1).unwrap();
        run_convert(
            &mut v,
            &ExtractorRegistry::standard(),
            &ConvertOptions::default(),
            None,
        )
    });

    std::thread::sleep(std::time::Duration::from_millis(20));

    let h2 = std::thread::spawn(move || {
        let mut v = MdVault::open(&p2).unwrap();
        run_convert(
            &mut v,
            &ExtractorRegistry::standard(),
            &ConvertOptions {
                wait_seconds: Some(10),
                ..Default::default()
            },
            None,
        )
    });

    assert!(h1.join().unwrap().is_ok());
    assert!(
        h2.join().unwrap().is_ok(),
        "waiter should succeed once first releases the lock"
    );
}
