//! MCP transports.
//!
//! * **stdio** — newline-delimited JSON-RPC on stdin/stdout. This is how a
//!   local harness (Claude Code, opencode) spawns the server as a subprocess.
//! * **http** — a pragmatic Streamable-HTTP endpoint: the client POSTs a
//!   JSON-RPC message and gets a JSON-RPC response (`application/json`). One
//!   shared server, many remote clients. (Stateless; no server-initiated SSE.)

use crate::backend::Backend;
use crate::mcp;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

pub fn serve_stdio(backend: &Backend) -> Result<()> {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break; // EOF
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(trimmed) {
            Ok(msg) => mcp::handle(backend, &msg),
            Err(_) => Some(parse_error()),
        };
        if let Some(v) = reply {
            out.write_all(serde_json::to_string(&v)?.as_bytes())?;
            out.write_all(b"\n")?;
            out.flush()?;
        }
    }
    Ok(())
}

pub fn serve_http(backend: &Backend, addr: &str) -> Result<()> {
    let server = tiny_http::Server::http(addr).map_err(|e| anyhow!("cannot bind {addr}: {e}"))?;
    eprintln!("recall: MCP HTTP server listening on http://{addr}  (POST JSON-RPC)");
    let json_header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .map_err(|_| anyhow!("bad header"))?;

    for mut req in server.incoming_requests() {
        if *req.method() != tiny_http::Method::Post {
            let _ = req.respond(tiny_http::Response::empty(405));
            continue;
        }
        let mut body = String::new();
        if req.as_reader().read_to_string(&mut body).is_err() {
            let _ = req.respond(tiny_http::Response::empty(400));
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&body) {
            Ok(msg) => mcp::handle(backend, &msg),
            Err(_) => Some(parse_error()),
        };
        let response = match reply {
            Some(v) => {
                let data = serde_json::to_string(&v).unwrap_or_else(|_| "{}".into());
                tiny_http::Response::from_string(data).with_header(json_header.clone())
            }
            // Notification: nothing to return.
            None => {
                let _ = req.respond(tiny_http::Response::empty(202));
                continue;
            }
        };
        let _ = req.respond(response);
    }
    Ok(())
}

fn parse_error() -> Value {
    json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "parse error" } })
}
