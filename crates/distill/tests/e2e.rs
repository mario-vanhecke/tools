//! End-to-end: a mock OpenAI-compatible embeddings server + the real `distill`
//! binary. Builds an index from local files and searches it, exercising the
//! full path (config → enumerate → extract → HTTP embed → SQLite+sqlite-vec →
//! search) with no external services.

use std::path::Path;
use std::process::Command;

const DIMS: usize = 16;

/// A toy deterministic embedding: each token bumps the dimension it hashes to.
/// Documents/queries sharing words end up close in L2 space, so search ranking
/// is meaningful in the test.
fn keyword_vec(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; DIMS];
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let mut h: u32 = 2166136261;
        for b in word.to_ascii_lowercase().bytes() {
            h = (h ^ b as u32).wrapping_mul(16777619);
        }
        v[(h as usize) % DIMS] += 1.0;
    }
    v
}

/// Start a mock embeddings endpoint on a random port; returns its `/v1` base.
fn spawn_mock_embedder() -> String {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let inputs = parsed["input"].as_array().cloned().unwrap_or_default();
            let data: Vec<_> = inputs
                .iter()
                .map(|t| serde_json::json!({ "embedding": keyword_vec(t.as_str().unwrap_or("")) }))
                .collect();
            let payload = serde_json::json!({ "data": data }).to_string();
            let header =
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .unwrap();
            let _ = req.respond(tiny_http::Response::from_string(payload).with_header(header));
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

fn write(path: &Path, content: &str) {
    std::fs::write(path, content).unwrap();
}

#[test]
fn build_then_search_end_to_end() {
    let endpoint = spawn_mock_embedder();
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    write(
        &docs.join("fruit.md"),
        "Apples and bananas are sweet fruit. Carrot is a vegetable.",
    );
    write(
        &docs.join("vehicles.txt"),
        "Trucks and engines and diesel torque and gearbox.",
    );
    // A nested path with a space, to exercise recursion + locator encoding.
    std::fs::create_dir_all(docs.join("nested")).unwrap();
    write(
        &docs.join("nested/space file.md"),
        "Quarterly revenue and budget forecast and spreadsheet.",
    );

    let kb = dir.path().join("team.kb");
    let cfg_path = dir.path().join("knowledge.toml");
    write(
        &cfg_path,
        &format!(
            r#"
[[source]]
type = "local"
path = "{docs}"

[embedding]
endpoint = "{endpoint}"
model = "mock"
dims = {DIMS}

[output]
path = "{kb}"
"#,
            docs = docs.display().to_string().replace('\\', "/"),
            kb = kb.display().to_string().replace('\\', "/"),
        ),
    );

    // distill build
    let build = Command::new(env!("CARGO_BIN_EXE_distill"))
        .args(["--config", cfg_path.to_str().unwrap(), "build"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(kb.exists(), "index file should exist");

    // distill search (JSON) — the fruit query should surface the fruit doc.
    let search = Command::new(env!("CARGO_BIN_EXE_distill"))
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "search",
            "banana fruit",
            "-k",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        search.status.success(),
        "search failed: {}",
        String::from_utf8_lossy(&search.stderr)
    );
    let out = String::from_utf8_lossy(&search.stdout);
    let hits: serde_json::Value = serde_json::from_str(&out).unwrap();
    let top = &hits[0];
    assert!(
        top["locator"].as_str().unwrap().contains("fruit.md"),
        "top hit should be fruit.md, got: {out}"
    );
    // Locator is a clickable file:// URL with spaces encoded where present.
    assert!(top["locator"].as_str().unwrap().starts_with("file://"));

    // Re-build is incremental: nothing changed, so nothing re-indexed.
    let rebuild = Command::new(env!("CARGO_BIN_EXE_distill"))
        .args(["--config", cfg_path.to_str().unwrap(), "--json", "build"])
        .output()
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&rebuild.stdout)).unwrap();
    let indexed: i64 = report["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["indexed"].as_i64().unwrap())
        .sum();
    assert_eq!(indexed, 0, "incremental rebuild should re-index nothing");
}
