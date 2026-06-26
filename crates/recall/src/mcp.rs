//! Minimal MCP (Model Context Protocol) server logic — JSON-RPC 2.0 dispatch,
//! transport-agnostic. The stdio and HTTP transports both feed parsed messages
//! here and serialize whatever `Response` comes back.
//!
//! Implements just what a knowledge server needs: `initialize`, `tools/list`,
//! `tools/call` (`kb_search`, `kb_get`), and `ping`.

use crate::backend::Backend;
use kb_core::locator;
use serde_json::{json, Value};

const SERVER_NAME: &str = "recall";
const DEFAULT_PROTOCOL: &str = "2024-11-05";

/// Handle one parsed JSON-RPC message. Returns `Some(response)` for requests
/// and `None` for notifications (which get no reply).
pub fn handle(backend: &Backend, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    // Notifications (no `id`) are acknowledged silently.
    let id = msg.get("id").cloned()?;

    let result = match method {
        "initialize" => Ok(initialize_result(msg)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list()),
        "tools/call" => tools_call(backend, msg.get("params")),
        other => Err(rpc_error(-32601, &format!("method not found: {other}"))),
    };

    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(err) => json!({ "jsonrpc": "2.0", "id": id, "error": err }),
    })
}

fn initialize_result(msg: &Value) -> Value {
    // Echo the client's requested protocol version when present.
    let protocol = msg
        .get("params")
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_PROTOCOL);
    json!({
        "protocolVersion": protocol,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") }
    })
}

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "kb_search",
                "description": "Semantic search over the knowledge base. Returns the most \
                                relevant passages, each with a locator (file://, smb://, or a \
                                SharePoint URL) you can cite or open. Use this to ground answers \
                                in the indexed documents.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Natural-language question or keywords." },
                        "k": { "type": "integer", "description": "Number of passages to return (default 5).", "minimum": 1, "maximum": 50 }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "kb_get",
                "description": "Fetch the full indexed text of one document by its locator (as \
                                returned by kb_search). Returns the text held in the index; the \
                                original file stays at its source.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "locator": { "type": "string", "description": "A locator from kb_search (without any #page anchor)." }
                    },
                    "required": ["locator"]
                }
            }
        ]
    })
}

fn tools_call(backend: &Backend, params: Option<&Value>) -> Result<Value, Value> {
    let params = params.ok_or_else(|| rpc_error(-32602, "missing params"))?;
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| rpc_error(-32602, "missing tool name"))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    match name {
        "kb_search" => call_search(backend, &args),
        "kb_get" => call_get(backend, &args),
        other => Err(rpc_error(-32602, &format!("unknown tool: {other}"))),
    }
}

fn call_search(backend: &Backend, args: &Value) -> Result<Value, Value> {
    let query = args
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| rpc_error(-32602, "kb_search needs a `query` string"))?;
    let k = args.get("k").and_then(|k| k.as_u64()).unwrap_or(5) as usize;

    match backend.search(query, k) {
        Ok(hits) if hits.is_empty() => Ok(tool_text(format!("No matches for: {query}"))),
        Ok(hits) => {
            let mut out = format!("{} result(s) for: {query}\n", hits.len());
            for (i, h) in hits.iter().enumerate() {
                let loc = locator::with_page(&h.locator, h.page);
                out.push_str(&format!(
                    "\n[{}] {}  (source: {})\n  {}\n  {}\n",
                    i + 1,
                    h.title,
                    h.source,
                    loc,
                    h.text.replace('\n', " ")
                ));
            }
            Ok(tool_text(out))
        }
        Err(e) => Ok(tool_error(format!("search failed: {e:#}"))),
    }
}

fn call_get(backend: &Backend, args: &Value) -> Result<Value, Value> {
    let locator = args
        .get("locator")
        .and_then(|l| l.as_str())
        .ok_or_else(|| rpc_error(-32602, "kb_get needs a `locator` string"))?;
    // Tolerate a #page anchor by trimming it.
    let base = locator.split('#').next().unwrap_or(locator);
    match backend.get(base) {
        Ok(Some(doc)) => Ok(tool_text(format!(
            "{}  (source: {})\n{}\n\n{}",
            doc.title, doc.source, doc.locator, doc.text
        ))),
        Ok(None) => Ok(tool_error(format!("no document with locator: {base}"))),
        Err(e) => Ok(tool_error(format!("get failed: {e:#}"))),
    }
}

fn tool_text(text: String) -> Value {
    json!({ "content": [ { "type": "text", "text": text } ] })
}

fn tool_error(text: String) -> Value {
    json!({ "content": [ { "type": "text", "text": text } ], "isError": true })
}

fn rpc_error(code: i64, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kb_core::{EmbeddingConfig, Index};

    fn temp_backend() -> (tempfile::TempDir, Backend) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.kb");
        let emb = EmbeddingConfig {
            endpoint: "http://localhost:11434/v1".into(),
            model: "m".into(),
            dims: 3,
            api_key_env: None,
        };
        Index::open_or_create(&path, &emb).unwrap();
        let backend = Backend::open(&path).unwrap();
        (dir, backend)
    }

    #[test]
    fn initialize_reports_server_and_echoes_protocol() {
        let (_d, b) = temp_backend();
        let r = handle(
            &b,
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize",
                    "params":{"protocolVersion":"2025-06-18"}}),
        )
        .unwrap();
        assert_eq!(r["result"]["serverInfo"]["name"], "recall");
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert!(r["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_exposes_search_and_get() {
        let (_d, b) = temp_backend();
        let r = handle(&b, &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        let tools = r["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"kb_search"));
        assert!(names.contains(&"kb_get"));
    }

    #[test]
    fn notifications_get_no_reply() {
        let (_d, b) = temp_backend();
        assert!(handle(
            &b,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none());
    }

    #[test]
    fn unknown_method_is_rpc_error() {
        let (_d, b) = temp_backend();
        let r = handle(&b, &json!({"jsonrpc":"2.0","id":9,"method":"bogus"})).unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }
}
