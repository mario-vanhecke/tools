//! Exercises the cookie-auth crawler against a mock SharePoint REST API
//! (`/_api/web/...`). Proves library discovery, folder recursion, system-library
//! skipping, item mapping, and that the session cookie is sent — without a live
//! tenant. Only the literal network round-trip to SharePoint is uncovered.

use crawl_core::crawl::{self, RunOptions};
use crawl_core::registry::{self, sources::AddSourceOptions, DocQuery};
use crawl_core::source::{SourceKind, Strategy};
use crawl_core::CrawlVault;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

/// Start a mock SharePoint REST server. Records the Cookie header of each
/// request so the test can assert the session cookie was sent.
fn start_mock(seen_cookie: Arc<Mutex<Vec<String>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &seen_cookie);
        }
    });
    base
}

fn handle(mut stream: TcpStream, seen_cookie: &Arc<Mutex<Vec<String>>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("cookie:") {
            // Capture from the original (case-preserving) line.
            seen_cookie
                .lock()
                .unwrap()
                .push(line["cookie:".len()..].trim().to_string());
        }
    }
    if content_length > 0 {
        let mut body = vec![0u8; content_length];
        let _ = reader.read_exact(&mut body);
    }

    let path = request_line.split_whitespace().nth(1).unwrap_or("");
    let body = route(path);
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn f(name: &str, srv: &str, len: &str) -> serde_json::Value {
    json!({
        "Name": name,
        "ServerRelativeUrl": srv,
        "Length": len,
        "TimeLastModified": "2024-03-04T05:06:07Z"
    })
}

fn route(path: &str) -> String {
    if path.contains("/_api/web/lists") {
        return json!({ "value": [
            { "Title": "Documents", "Hidden": false, "BaseTemplate": 101,
              "RootFolder": { "ServerRelativeUrl": "/sites/Marketing/Shared Documents" } },
            // A system library that must be skipped.
            { "Title": "Site Assets", "Hidden": false, "BaseTemplate": 101,
              "RootFolder": { "ServerRelativeUrl": "/sites/Marketing/SiteAssets" } },
            // A non-document-library list that must be skipped (template != 101).
            { "Title": "Site Pages", "Hidden": false, "BaseTemplate": 119,
              "RootFolder": { "ServerRelativeUrl": "/sites/Marketing/SitePages" } }
        ]})
        .to_string();
    }
    if path.contains("/Files") {
        if path.contains("Plans") {
            return json!({ "value": [
                f("strategy.pdf", "/sites/Marketing/Shared Documents/Plans/strategy.pdf", "40")
            ]})
            .to_string();
        }
        return json!({ "value": [
            f("policy.pdf", "/sites/Marketing/Shared Documents/policy.pdf", "11"),
            f("budget.xlsx", "/sites/Marketing/Shared Documents/budget.xlsx", "22")
        ]})
        .to_string();
    }
    if path.contains("/Folders") {
        if path.contains("Plans") {
            return json!({ "value": [] }).to_string();
        }
        return json!({ "value": [
            { "Name": "Plans", "ServerRelativeUrl": "/sites/Marketing/Shared Documents/Plans" },
            { "Name": "Forms", "ServerRelativeUrl": "/sites/Marketing/Shared Documents/Forms" }
        ]})
        .to_string();
    }
    json!({ "value": [] }).to_string()
}

#[test]
fn cookie_auth_crawls_sharepoint_rest() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let base = start_mock(seen.clone());

    let tmp = tempfile::tempdir().unwrap();
    let vault = CrawlVault::init(tmp.path(), false).unwrap();
    std::env::set_var("CRAWL_TEST_SP_COOKIE", "FedAuth=abc123; rtFa=def456");

    registry::sources::add_source(
        &vault.conn,
        &AddSourceOptions {
            name: "coe".into(),
            kind: SourceKind::SharePoint,
            uri: "contoso".into(),
            strategy: Strategy::Recursive,
            config: json!({
                "auth": "cookie",
                "site_hostname": "contoso.sharepoint.com",
                "site_path": "/sites/Marketing",
                "rest_base": base,
                "cookie_env": "CRAWL_TEST_SP_COOKIE",
            }),
            enabled: true,
        },
    )
    .unwrap();

    let report = crawl::run(&vault, &RunOptions::default()).unwrap();
    let s = &report.sources[0];
    assert_eq!(s.status, "ok", "note: {:?}", s.note);
    assert_eq!(s.discovered, 3, "two root files + one nested file");

    let mut docs = registry::query_documents(&vault.conn, &DocQuery::default()).unwrap();
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    let rels: Vec<&str> = docs
        .iter()
        .map(|d| d.rel_path.as_deref().unwrap())
        .collect();
    assert_eq!(
        rels,
        vec![
            "Shared Documents/budget.xlsx",
            "Shared Documents/policy.pdf",
            "Shared Documents/Plans/strategy.pdf",
        ]
    );

    // URI is the real SharePoint web URL; size parsed from the string Length.
    let policy = docs.iter().find(|d| d.name == "policy.pdf").unwrap();
    assert_eq!(
        policy.uri,
        "https://contoso.sharepoint.com/sites/Marketing/Shared Documents/policy.pdf"
    );
    assert_eq!(policy.size, Some(11));

    // The session cookie was actually sent on the REST requests.
    let cookies = seen.lock().unwrap();
    assert!(
        cookies.iter().any(|c| c.contains("FedAuth=abc123")),
        "expected the session cookie on REST requests, saw: {cookies:?}"
    );
}
