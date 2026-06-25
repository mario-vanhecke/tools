//! Verifies `crawl fetch` for SharePoint: after discovery, it downloads each
//! file's bytes via the REST `$value` endpoint (with the session cookie) and
//! writes them into the local tree. Exercised against a mock REST server.

use crawl_core::crawl::{self, fetch, FetchOptions, RunOptions};
use crawl_core::registry::{self, sources::AddSourceOptions};
use crawl_core::source::{SourceKind, Strategy};
use crawl_core::CrawlVault;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

const FILE_BODY: &str = "DOWNLOADED-BYTES"; // 16 bytes

fn start_mock() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream);
        }
    });
    base
}

fn handle(mut stream: TcpStream) {
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
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    if content_length > 0 {
        let mut body = vec![0u8; content_length];
        let _ = reader.read_exact(&mut body);
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("");
    let body = route(path);
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn route(path: &str) -> String {
    let root = "/sites/Engineering";
    // The file download endpoint.
    if path.contains("/$value") {
        return FILE_BODY.to_string();
    }
    if path.contains("/_api/web/lists") {
        return json!({ "value": [
            { "Title": "Documents", "Hidden": false, "BaseTemplate": 101,
              "RootFolder": { "ServerRelativeUrl": format!("{root}/Shared Documents") } }
        ]})
        .to_string();
    }
    if path.contains("/Files") {
        return json!({ "value": [
            { "Name": "report.pdf",
              "ServerRelativeUrl": format!("{root}/Shared Documents/report.pdf"),
              "Length": "16", "TimeLastModified": "2024-03-04T05:06:07Z" }
        ]})
        .to_string();
    }
    json!({ "value": [] }).to_string() // /Folders
}

#[test]
fn fetch_downloads_sharepoint_files() {
    let base = start_mock();
    let tmp = tempfile::tempdir().unwrap();
    let vault = CrawlVault::init(tmp.path(), false).unwrap();
    std::env::set_var("CRAWL_TEST_FETCH_COOKIE", "FedAuth=x; rtFa=y");

    registry::sources::add_source(
        &vault.conn,
        &AddSourceOptions {
            name: "sp".into(),
            kind: SourceKind::SharePoint,
            uri: "contoso".into(),
            strategy: Strategy::Recursive,
            config: json!({
                "auth": "cookie",
                "site_hostname": "contoso.sharepoint.com",
                "site_path": "/sites/Engineering",
                "rest_base": base,
                "cookie_env": "CRAWL_TEST_FETCH_COOKIE",
            }),
            enabled: true,
        },
    )
    .unwrap();

    // Discover, then fetch into a local tree.
    crawl::run(&vault, &RunOptions::default()).unwrap();
    let out = tmp.path().join("files");
    let report = fetch::run(
        &vault,
        &FetchOptions {
            out_dir: out.clone(),
            source: None,
            extension: None,
            status: None,
            force: false,
        },
    )
    .unwrap();

    assert_eq!(report.sources[0].fetched, 1);
    assert_eq!(report.sources[0].errors, 0);

    // The downloaded bytes landed at <out>/<source>/<rel_path>.
    let dest = out.join("sp").join("Shared Documents").join("report.pdf");
    assert!(dest.exists(), "expected {}", dest.display());
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), FILE_BODY);

    // Re-fetch is incremental: size matches, so it's skipped.
    let again = fetch::run(
        &vault,
        &FetchOptions {
            out_dir: out,
            source: None,
            extension: None,
            status: None,
            force: false,
        },
    )
    .unwrap();
    assert_eq!(again.sources[0].fetched, 0);
    assert_eq!(again.sources[0].skipped, 1);
}
