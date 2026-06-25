//! Tenant-wide crawl (`all_sites=true`): enumerate sites via the Search API,
//! then crawl each recursively. Verifies site discovery, per-site traversal,
//! and that sites on a different host (e.g. a `-my` OneDrive host the cookie
//! can't auth) are skipped. Exercised against a mock; verified live separately.

use crawl_core::crawl::{self, RunOptions};
use crawl_core::registry::{self, sources::AddSourceOptions, DocQuery};
use crawl_core::source::{SourceKind, Strategy};
use crawl_core::CrawlVault;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

fn start_mock() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
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
    let body = route(path, base);
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn cell_path(p: &str) -> serde_json::Value {
    json!({ "Cells": [ { "Key": "Path", "Value": p } ] })
}

fn route(path: &str, base: &str) -> String {
    // Site enumeration via Search: two crawlable sites + one on another host.
    if path.contains("/_api/search/query") {
        return json!({
            "PrimaryQueryResult": { "RelevantResults": {
                "TotalRows": 3,
                "Table": { "Rows": [
                    cell_path(&format!("{base}/sites/Alpha")),
                    cell_path(&format!("{base}/sites/Beta")),
                    // Different host (like contoso-my.sharepoint.com) -> must be skipped.
                    cell_path("https://elsewhere.example/personal/someone"),
                ]}
            }}
        })
        .to_string();
    }
    if path.contains("/_api/web/lists") {
        let site = if path.contains("/sites/Beta/") {
            "Beta"
        } else {
            "Alpha"
        };
        return json!({ "value": [
            { "Title": "Documents", "Hidden": false, "BaseTemplate": 101,
              "RootFolder": { "ServerRelativeUrl": format!("/sites/{site}/Shared Documents") } }
        ]})
        .to_string();
    }
    if path.contains("/Files") {
        if path.contains("/sites/Alpha/") {
            return json!({ "value": [
                { "Name": "alpha.pdf", "ServerRelativeUrl": "/sites/Alpha/Shared Documents/alpha.pdf", "Length": "10", "TimeLastModified": "2024-01-02T03:04:05Z" }
            ]})
            .to_string();
        }
        if path.contains("/sites/Beta/") {
            return json!({ "value": [
                { "Name": "beta.docx", "ServerRelativeUrl": "/sites/Beta/Shared Documents/beta.docx", "Length": "20", "TimeLastModified": "2024-01-02T03:04:05Z" }
            ]})
            .to_string();
        }
        return json!({ "value": [] }).to_string();
    }
    json!({ "value": [] }).to_string() // /Folders etc.
}

#[test]
fn all_sites_enumerates_and_crawls_each() {
    let base = start_mock();
    let tmp = tempfile::tempdir().unwrap();
    let vault = CrawlVault::init(tmp.path(), false).unwrap();
    std::env::set_var("CRAWL_TEST_ALLSITES_COOKIE", "FedAuth=x; rtFa=y");

    registry::sources::add_source(
        &vault.conn,
        &AddSourceOptions {
            name: "tenant".into(),
            kind: SourceKind::SharePoint,
            uri: "contoso".into(),
            strategy: Strategy::Recursive,
            config: json!({
                "auth": "cookie",
                "site_hostname": "contoso.sharepoint.com",
                "all_sites": true,
                "rest_base": base,
                "cookie_env": "CRAWL_TEST_ALLSITES_COOKIE",
            }),
            enabled: true,
        },
    )
    .unwrap();

    let report = crawl::run(&vault, &RunOptions::default()).unwrap();
    let s = &report.sources[0];
    assert_eq!(s.status, "ok", "note: {:?}", s.note);
    // Two same-host sites crawled; the off-host (OneDrive) site is skipped.
    assert_eq!(s.discovered, 2, "alpha.pdf + beta.docx");

    let mut docs = registry::query_documents(&vault.conn, &DocQuery::default()).unwrap();
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    let rels: Vec<&str> = docs
        .iter()
        .map(|d| d.rel_path.as_deref().unwrap())
        .collect();
    // Paths keep the full server-relative path so the two sites stay distinct.
    assert_eq!(
        rels,
        vec![
            "sites/Alpha/Shared Documents/alpha.pdf",
            "sites/Beta/Shared Documents/beta.docx",
        ]
    );
}
