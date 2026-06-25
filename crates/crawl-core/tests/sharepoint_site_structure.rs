//! Regression test for a real-world SharePoint shape (the structure the Graph
//! path failed on but cookie/REST handles):
//!   - the documents live in a *non-default* library ("Project Documents"),
//!     while the default "Documents" (Shared Documents) library is **empty**;
//!   - every document is in a **subfolder**, none at a library root;
//!   - system libraries (Site Assets), non-document lists (BaseTemplate != 101),
//!     and the system "Forms" folder must be skipped;
//!   - a `.cs` source file is filtered out by the default document-type config.

use crawl_core::crawl::{self, RunOptions};
use crawl_core::registry::{self, sources::AddSourceOptions, DocQuery};
use crawl_core::source::{SourceKind, Strategy};
use crawl_core::CrawlVault;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

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
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn file(name: &str, srv: &str, len: &str) -> serde_json::Value {
    json!({ "Name": name, "ServerRelativeUrl": srv, "Length": len, "TimeLastModified": "2024-03-04T05:06:07Z" })
}

fn route(path: &str) -> String {
    let root = "/sites/Engineering";
    if path.contains("/_api/web/lists") {
        return json!({ "value": [
            // Where the documents actually are (non-default name).
            { "Title": "Project Documents", "Hidden": false, "BaseTemplate": 101,
              "RootFolder": { "ServerRelativeUrl": format!("{root}/Project Documents") } },
            // The default library — empty.
            { "Title": "Documents", "Hidden": false, "BaseTemplate": 101,
              "RootFolder": { "ServerRelativeUrl": format!("{root}/Shared Documents") } },
            // System library — must be skipped by name.
            { "Title": "Site Assets", "Hidden": false, "BaseTemplate": 101,
              "RootFolder": { "ServerRelativeUrl": format!("{root}/SiteAssets") } },
            // Not a document library (BaseTemplate != 101) — must be skipped.
            { "Title": "Service Inventory", "Hidden": false, "BaseTemplate": 100,
              "RootFolder": { "ServerRelativeUrl": format!("{root}/Lists/Service Inventory") } },
        ]})
        .to_string();
    }
    if path.contains("/Files") {
        if path.contains("Reports") {
            return json!({ "value": [
                file("plan.docx", &format!("{root}/Project Documents/Reports/plan.docx"), "42000"),
                file("deck.pptx", &format!("{root}/Project Documents/Reports/deck.pptx"), "433000"),
                // A code file — skipped by the default document-type filter.
                file("Program.cs", &format!("{root}/Project Documents/Reports/Program.cs"), "4600"),
            ]})
            .to_string();
        }
        // Project Documents root and Shared Documents root are both empty.
        return json!({ "value": [] }).to_string();
    }
    if path.contains("/Folders") {
        // Only the Project Documents *root* has subfolders; subfolders/default lib have none.
        if !path.contains("Reports") && path.contains("Project%20Documents'") {
            return json!({ "value": [
                { "Name": "Reports", "ServerRelativeUrl": format!("{root}/Project Documents/Reports") },
                { "Name": "Forms", "ServerRelativeUrl": format!("{root}/Project Documents/Forms") }
            ]})
            .to_string();
        }
        return json!({ "value": [] }).to_string();
    }
    json!({ "value": [] }).to_string()
}

#[test]
fn finds_docs_in_nondefault_library_subfolders() {
    let base = start_mock();
    let tmp = tempfile::tempdir().unwrap();
    let vault = CrawlVault::init(tmp.path(), false).unwrap();
    std::env::set_var("CRAWL_TEST_STRUCT_COOKIE", "FedAuth=x; rtFa=y");

    registry::sources::add_source(
        &vault.conn,
        &AddSourceOptions {
            name: "site".into(),
            kind: SourceKind::SharePoint,
            uri: "contoso".into(),
            strategy: Strategy::Recursive,
            config: json!({
                "auth": "cookie",
                "site_hostname": "contoso.sharepoint.com",
                "site_path": "/sites/Engineering",
                "rest_base": base,
                "cookie_env": "CRAWL_TEST_STRUCT_COOKIE",
            }),
            enabled: true,
        },
    )
    .unwrap();

    // Default document-type config: the two office docs are recorded; the .cs is
    // skipped; the empty default library and system libraries contribute nothing.
    let report = crawl::run(&vault, &RunOptions::default()).unwrap();
    let s = &report.sources[0];
    assert_eq!(s.status, "ok", "note: {:?}", s.note);
    assert_eq!(s.discovered, 2, "the two office docs");
    assert_eq!(s.skipped, 1, "the .cs file");

    let mut docs = registry::query_documents(&vault.conn, &DocQuery::default()).unwrap();
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    let rels: Vec<&str> = docs
        .iter()
        .map(|d| d.rel_path.as_deref().unwrap())
        .collect();
    assert_eq!(
        rels,
        vec![
            "Project Documents/Reports/deck.pptx",
            "Project Documents/Reports/plan.docx",
        ]
    );
}
