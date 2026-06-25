//! Exercises the SharePoint crawler end-to-end against a mock Microsoft Graph
//! server. This proves the real logic — OAuth2 token call, children traversal
//! with pagination, folder recursion, item mapping, hashes, rel-paths, and the
//! `/delta` incremental path — without talking to a live tenant. The only thing
//! it cannot cover is the network round-trip to Microsoft itself.

use crawl_core::crawl::{self, RunOptions};
use crawl_core::registry::{self, sources::AddSourceOptions, DocQuery};
use crawl_core::source::{SourceKind, Strategy};
use crawl_core::CrawlVault;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

/// Start a throwaway HTTP server that mimics the Graph endpoints the crawler
/// hits. Returns its base URL (e.g. `http://127.0.0.1:54321`).
fn start_mock_graph() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let base_for_thread = base.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &base_for_thread);
        }
    });
    base
}

fn handle(mut stream: std::net::TcpStream, base: &str) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    // Consume headers; note any body length so we can drain it.
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    if content_length > 0 {
        let mut body = vec![0u8; content_length];
        let _ = reader.read_exact(&mut body);
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let body = route(method, path, base);

    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn route(method: &str, path: &str, base: &str) -> String {
    // OAuth2 device-code request: hand back a code with interval 0 (no wait).
    if method == "POST" && path.contains("/devicecode") {
        return json!({
            "device_code": "DEV-CODE-123",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://microsoft.com/devicelogin",
            "message": "To sign in, open https://microsoft.com/devicelogin and enter ABCD-EFGH",
            "interval": 0,
            "expires_in": 900
        })
        .to_string();
    }
    // OAuth2 token endpoint (client-credentials, device-code poll, refresh).
    if method == "POST" && path.contains("/token") {
        return json!({
            "access_token": "TESTTOKEN",
            "refresh_token": "REFRESH-1",
            "expires_in": 3600
        })
        .to_string();
    }
    // Site lookup by server-relative path (the `:/path` form has a colon).
    if method == "GET" && path.contains("/sites/") && path.contains(':') {
        return json!({ "id": "SITE1", "name": "Marketing" }).to_string();
    }
    // List a site's document libraries (drives).
    if method == "GET" && path.ends_with("/drives") {
        return json!({ "value": [ { "id": "LIB1", "name": "Documents" } ] }).to_string();
    }
    // The default library (fallback path).
    if method == "GET" && path.ends_with("/drive") {
        return json!({ "id": "LIB1", "name": "Documents" }).to_string();
    }
    // A folder's children: one nested file.
    if path.contains("/items/FOLDER1/children") {
        return json!({
            "value": [ file("c.txt", "F3", 4, "QXH3", "/drive/root:/Sub") ]
        })
        .to_string();
    }
    // Root listing, paginated: page 1 -> a file + a folder + nextLink; page 2 -> a file.
    if path.contains("/root/children") {
        if path.contains("page=2") {
            return json!({
                "value": [ file("b.docx", "F2", 22, "QXH2", "/drive/root:") ]
            })
            .to_string();
        }
        return json!({
            "value": [
                file("a.pdf", "F1", 11, "QXH1", "/drive/root:"),
                folder("Sub", "FOLDER1")
            ],
            "@odata.nextLink": format!("{base}/drives/D/root/children?page=2")
        })
        .to_string();
    }
    // Delta: page 1 has a live file and a deletion; page 2 ends with a deltaLink.
    if path.contains("/root/delta") {
        if path.contains("token=2") {
            return json!({
                "value": [],
                "@odata.deltaLink": format!("{base}/drives/D/root/delta?token=final")
            })
            .to_string();
        }
        return json!({
            "value": [
                file("a.pdf", "F1", 11, "QXH1", "/drive/root:"),
                { "id": "GONE", "name": "old.pdf", "deleted": { "state": "deleted" } }
            ],
            "@odata.nextLink": format!("{base}/drives/D/root/delta?token=2")
        })
        .to_string();
    }
    json!({ "value": [] }).to_string()
}

fn file(name: &str, id: &str, size: i64, hash: &str, parent: &str) -> serde_json::Value {
    json!({
        "name": name,
        "id": id,
        "size": size,
        "webUrl": format!("https://contoso.sharepoint.com/{name}"),
        "lastModifiedDateTime": "2024-03-04T05:06:07Z",
        "file": { "hashes": { "quickXorHash": hash } },
        "parentReference": { "path": parent }
    })
}

fn folder(name: &str, id: &str) -> serde_json::Value {
    json!({
        "name": name,
        "id": id,
        "folder": { "childCount": 1 },
        "parentReference": { "path": "/drive/root:" }
    })
}

fn setup_vault() -> (tempfile::TempDir, CrawlVault) {
    let tmp = tempfile::tempdir().unwrap();
    let vault = CrawlVault::init(tmp.path(), false).unwrap();
    (tmp, vault)
}

fn add_sp(vault: &CrawlVault, strategy: Strategy, config: serde_json::Value) {
    registry::sources::add_source(
        &vault.conn,
        &AddSourceOptions {
            name: "sp".into(),
            kind: SourceKind::SharePoint,
            uri: "contoso".into(),
            strategy,
            config,
            enabled: true,
        },
    )
    .unwrap();
}

fn add_sharepoint(vault: &CrawlVault, base: &str, strategy: Strategy, secret_env: &str) {
    std::env::set_var(secret_env, "test-secret");
    add_sp(
        vault,
        strategy,
        json!({
            "auth": "client_credentials",
            "tenant_id": "test-tenant",
            "client_id": "test-client",
            "drive_id": "D",
            "secret_env": secret_env,
            "graph_base": base,
            "oauth_base": base,
        }),
    );
}

fn docs(vault: &CrawlVault) -> Vec<crawl_core::registry::DocumentRow> {
    let mut v = registry::query_documents(&vault.conn, &DocQuery::default()).unwrap();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

#[test]
fn recursive_crawl_paginates_and_recurses_folders() {
    let base = start_mock_graph();
    let (_tmp, vault) = setup_vault();
    add_sharepoint(&vault, &base, Strategy::Recursive, "CRAWL_TEST_SP_SECRET_1");

    let report = crawl::run(&vault, &RunOptions::default()).unwrap();
    let s = &report.sources[0];
    assert_eq!(s.status, "ok", "note: {:?}", s.note);
    assert_eq!(s.discovered, 3, "two root files + one nested file");

    let d = docs(&vault);
    let names: Vec<&str> = d.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, vec!["a.pdf", "b.docx", "c.txt"]);

    // URI is the webUrl; the provider hash is captured; rel_path reflects nesting.
    let a = &d[0];
    assert_eq!(a.uri, "https://contoso.sharepoint.com/a.pdf");
    assert_eq!(a.content_hash.as_deref(), Some("QXH1"));
    assert_eq!(a.extension.as_deref(), Some("pdf"));
    let c = d.iter().find(|x| x.name == "c.txt").unwrap();
    assert_eq!(c.rel_path.as_deref(), Some("Sub/c.txt"));
}

#[test]
fn incremental_delta_skips_deletions_and_stores_delta_link() {
    let base = start_mock_graph();
    let (_tmp, vault) = setup_vault();
    add_sharepoint(
        &vault,
        &base,
        Strategy::Incremental,
        "CRAWL_TEST_SP_SECRET_2",
    );

    let report = crawl::run(&vault, &RunOptions::default()).unwrap();
    let s = &report.sources[0];
    assert_eq!(s.status, "ok", "note: {:?}", s.note);
    assert_eq!(
        s.discovered, 1,
        "only the live file; the deletion is skipped"
    );

    let d = docs(&vault);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].name, "a.pdf");

    // The refreshed delta link is persisted on the source for the next run.
    let src = registry::sources::get_source_by_name(&vault.conn, "sp")
        .unwrap()
        .unwrap();
    let stored = src
        .config
        .get("delta_link")
        .and_then(|v| v.as_str())
        .unwrap();
    assert!(
        stored.ends_with("token=final"),
        "stored delta link: {stored}"
    );
}

#[test]
fn resolves_site_by_path_and_lists_libraries() {
    // No drive_id: crawl must resolve the site from site_hostname/site_path,
    // list its document libraries, and crawl them — the path your tenant hits.
    let base = start_mock_graph();
    let (_tmp, vault) = setup_vault();
    std::env::set_var("CRAWL_TEST_SP_SECRET_SITE", "x");
    add_sp(
        &vault,
        Strategy::Recursive,
        json!({
            "auth": "client_credentials",
            "tenant_id": "t",
            "client_id": "c",
            "secret_env": "CRAWL_TEST_SP_SECRET_SITE",
            "site_hostname": "contoso.sharepoint.com",
            "site_path": "/sites/Marketing",
            "graph_base": base,
            "oauth_base": base,
        }),
    );

    let report = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(
        report.sources[0].status, "ok",
        "note: {:?}",
        report.sources[0].note
    );
    assert_eq!(report.sources[0].discovered, 3);
    let names: Vec<String> = docs(&vault).into_iter().map(|d| d.name).collect();
    assert_eq!(names, vec!["a.pdf", "b.docx", "c.txt"]);
}

#[test]
fn device_code_signin_caches_token_and_crawls() {
    let base = start_mock_graph();
    let (tmp, vault) = setup_vault();
    add_sp(
        &vault,
        Strategy::Recursive,
        json!({
            "auth": "device_code",
            "tenant_id": "test-tenant",
            "client_id": "public-client",
            "drive_id": "D",
            "graph_base": base,
            "oauth_base": base,
        }),
    );

    // No secret, no env var: the device-code flow signs in (the mock returns the
    // code with interval 0 and an immediately-successful token poll).
    let report = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(
        report.sources[0].status, "ok",
        "note: {:?}",
        report.sources[0].note
    );
    assert_eq!(report.sources[0].discovered, 3);

    // The token (and refresh token) were cached under .crawl/ for reuse.
    // The filename carries the resource tag ("graph" for the Graph token).
    let cache = tmp.path().join(".crawl/sharepoint-sp-graph.token.json");
    assert!(
        cache.exists(),
        "expected cached token at {}",
        cache.display()
    );
    let cached: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    assert_eq!(cached["access_token"], "TESTTOKEN");
    assert_eq!(cached["refresh_token"], "REFRESH-1");
}

// Uses a shell-script stub for `az`, which only Unix can exec directly. The
// azure_cli code path itself is cross-platform (it shells out to a real `az`).
#[cfg(unix)]
#[test]
fn azure_cli_auth_uses_local_login() {
    let base = start_mock_graph();
    let (tmp, vault) = setup_vault();

    // A fake `az` that mimics `az account get-access-token --output json`.
    let az = tmp.path().join("fake-az.sh");
    std::fs::write(
        &az,
        "#!/bin/sh\nprintf '{\"accessToken\":\"AZ-TOKEN\",\"expiresOn\":\"2099-01-01\"}\\n'\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&az, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    add_sp(
        &vault,
        Strategy::Recursive,
        json!({
            "auth": "azure_cli",
            "drive_id": "D",
            "graph_base": base,
            "az_path": az.to_string_lossy(),
        }),
    );

    let report = crawl::run(&vault, &RunOptions::default()).unwrap();
    assert_eq!(
        report.sources[0].status, "ok",
        "note: {:?}",
        report.sources[0].note
    );
    assert_eq!(
        report.sources[0].discovered, 3,
        "crawled via the az-provided token"
    );
}
