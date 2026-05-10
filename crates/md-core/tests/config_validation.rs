//! Config get/set/unset validation. Each KEY in keys.rs has a validator;
//! these tests exercise the validators end-to-end via Config::set and
//! verify that Config::load reads them back as the typed Config struct.

use md_core::config::{keys, Config};
use md_core::MdVault;
use serde_json::json;

fn fresh_vault() -> (tempfile::TempDir, MdVault) {
    let dir = tempfile::tempdir().unwrap();
    let v = MdVault::init(dir.path(), false).unwrap();
    (dir, v)
}

#[test]
fn defaults_load_correctly() {
    let (_dir, vault) = fresh_vault();
    assert_eq!(vault.config.output.dir, "converted");
    assert!(vault.config.output.annotate);
    assert!(vault.config.output.collision_aware_naming);
    assert!(vault.config.files.respect_mdignore);
    assert_eq!(vault.config.files.size_cap_bytes, 104_857_600); // 100 MB
    assert_eq!(vault.config.convert.concurrency, 3);
}

#[test]
fn set_and_get_string() {
    let (_dir, vault) = fresh_vault();
    Config::set(&vault.conn, keys::OUTPUT_DIR, json!("my-out")).unwrap();
    let v = Config::get(&vault.conn, keys::OUTPUT_DIR).unwrap();
    assert_eq!(v, json!("my-out"));
}

#[test]
fn set_and_get_bool_persists_across_reload() {
    let (dir, vault) = fresh_vault();
    Config::set(&vault.conn, keys::OUTPUT_ANNOTATE, json!(false)).unwrap();
    drop(vault);
    let v = MdVault::open(dir.path()).unwrap();
    assert!(!v.config.output.annotate);
}

#[test]
fn unset_reverts_to_default() {
    let (_dir, vault) = fresh_vault();
    Config::set(&vault.conn, keys::CONVERT_CONCURRENCY, json!(8)).unwrap();
    assert_eq!(
        Config::get(&vault.conn, keys::CONVERT_CONCURRENCY).unwrap(),
        json!(8)
    );
    Config::unset(&vault.conn, keys::CONVERT_CONCURRENCY).unwrap();
    assert_eq!(
        Config::get(&vault.conn, keys::CONVERT_CONCURRENCY).unwrap(),
        json!(3)
    );
}

#[test]
fn unknown_key_errors_on_set() {
    let (_dir, vault) = fresh_vault();
    let r = Config::set(&vault.conn, "no.such.key", json!("x"));
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(msg.contains("unknown config key"));
}

#[test]
fn unknown_key_errors_on_get() {
    let (_dir, vault) = fresh_vault();
    let r = Config::get(&vault.conn, "no.such.key");
    assert!(r.is_err());
}

#[test]
fn type_mismatch_rejected() {
    let (_dir, vault) = fresh_vault();
    // OUTPUT_ANNOTATE is bool; stringy input rejected
    let r = Config::set(&vault.conn, keys::OUTPUT_ANNOTATE, json!("yes"));
    assert!(r.is_err());
    let msg = r.err().unwrap().to_string();
    assert!(msg.contains("must be a boolean"));

    // CONVERT_CONCURRENCY must be a positive int; 0 rejected
    let r = Config::set(&vault.conn, keys::CONVERT_CONCURRENCY, json!(0));
    assert!(r.is_err());

    // FILES_SUPPORTED_EXTENSIONS must be a string array; non-string elements rejected
    let r = Config::set(
        &vault.conn,
        keys::FILES_SUPPORTED_EXTENSIONS,
        json!(["md", 123]),
    );
    assert!(r.is_err());

    // FILES_SIZE_CAP_BYTES must be a non-negative int; negative rejected
    let r = Config::set(&vault.conn, keys::FILES_SIZE_CAP_BYTES, json!(-1));
    assert!(r.is_err());
}

#[test]
fn array_round_trip() {
    let (_dir, vault) = fresh_vault();
    let v = json!(["md", "txt"]);
    Config::set(&vault.conn, keys::FILES_SUPPORTED_EXTENSIONS, v.clone()).unwrap();
    assert_eq!(
        Config::get(&vault.conn, keys::FILES_SUPPORTED_EXTENSIONS).unwrap(),
        v
    );
}

#[test]
fn list_all_includes_every_known_key() {
    let (_dir, vault) = fresh_vault();
    let entries = Config::list_all(&vault.conn).unwrap();
    assert_eq!(entries.len(), keys::KEYS.len());
    for d in keys::KEYS {
        assert!(
            entries.iter().any(|e| e.key == d.key),
            "list_all missing key {}",
            d.key
        );
    }
    // All defaults initially.
    assert!(entries.iter().all(|e| e.is_default));
}
