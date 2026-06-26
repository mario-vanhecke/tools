# `distill` + `recall` — the knowledge stack

A second, independent take on RAG that solves the same problem as `crawl → md →
rag`, the other way around:

- **`distill`** (producer) — reads one declarative `knowledge.toml`, pulls
  documents from your sources, and builds a single portable index.
- **`recall`** (server) — serves that index to an LLM harness (Claude Code,
  opencode, …) over **MCP**, so the model searches your documents itself.

## How it differs from rag / md / crawl

| | rag · md · crawl | distill · recall |
|---|---|---|
| Pipeline | 4 steps + on-disk `files/` and `converted/` trees | one pass, config → index |
| Config | SQLite vault + walk-up discovery | one `knowledge.toml` |
| Embeddings | bundled candle model (~2.2 GB) | pluggable HTTP endpoint, **no model in the binary** |
| Sources | copied/fetched locally | **referenced in place** — nothing is duplicated |
| Conversion | written to `converted/**/*.md` | **in memory** (transient temp file only for hard PDFs/Office) |
| Access | `rag search` in your shell | **MCP tools** the model calls itself, local or remote |

The index is the *only* artifact written. Each document is stored by
**reference** — a locator (`file://`, `smb://`, or a clickable SharePoint URL)
plus the derived searchable chunk text — never a copy of the file.

## Quickstart

```sh
# 1. an embeddings endpoint — local & private (nothing leaves your network):
ollama pull nomic-embed-text          # or point at any OpenAI-compatible API

# 2. describe what to index
distill init                          # writes knowledge.toml
#   edit it: add [[source]] blocks (local / smb / sharepoint), set the endpoint

# 3. build the index (sources stay at their origin)
distill build                         # → knowledge.kb
distill search "how do we handle releases?"   # quick local check

# 4. serve it to your harness
recall serve knowledge.kb --stdio
```

## `knowledge.toml`

```toml
[[source]]
type = "local"
path = "C:/Users/me/Documents"

[[source]]
type = "smb"
path = "//server/share/policies"

[[source]]
type = "sharepoint"
site = "tenant.sharepoint.com/sites/Eng"
auth = "cookie"                       # works on admin-locked tenants
cookie_env = "KB_SP_COOKIE"           # holds your FedAuth/rtFa session cookies

[embedding]
endpoint = "http://localhost:11434/v1"   # any OpenAI-compatible API (Ollama here)
model    = "nomic-embed-text"
dims     = 768                           # must match the model's output size
# api_key_env = "OPENAI_API_KEY"         # for hosted endpoints

[output]
path = "./knowledge.kb"
```

Builds are **incremental**: a document unchanged since the last build (by
mtime+size, then content hash) is skipped without re-embedding. Documents that
vanished at their origin are retired from the index.

### Sources

- **local / smb** — a directory or a mounted share / UNC path, walked
  recursively. The locator is a `file://` URL.
- **sharepoint** — `auth = "cookie"`: paste a browser session cookie (the
  `FedAuth` + `rtFa` pair) into the env var named by `cookie_env`. `distill`
  calls SharePoint's REST API directly (listing every document library and
  recursing), which sidesteps the Microsoft Graph admin-consent wall on
  locked-down tenants. Each file's clickable `webUrl` becomes its locator.
  (Interactive `browser` OAuth is a planned addition.)

### Embeddings

Any OpenAI-compatible `/v1/embeddings` endpoint. Point it at a local Ollama for
privacy, an internal service, or a hosted API. The endpoint, model, and `dims`
are recorded in the index, so `recall` can serve a `.kb` with just the file and
embed queries the same way the documents were embedded.

## Serving over MCP

`recall` exposes two tools to the model:

- **`kb_search(query, k?)`** — semantic search; returns passages each with their
  origin locator, so the model can cite or open the source.
- **`kb_get(locator)`** — the full indexed text of one document.

### Local (stdio)

```sh
recall serve knowledge.kb --stdio
```

Register it with your harness, e.g. Claude Code:

```sh
claude mcp add recall -- recall serve /abs/path/knowledge.kb --stdio
```

### Remote (HTTP)

Run one server; point many clients at it:

```sh
recall serve knowledge.kb --http 0.0.0.0:7077
```

Then add the URL as an MCP server in Claude Code / opencode. (The HTTP transport
is request/response JSON-RPC — pragmatic Streamable-HTTP, no server-initiated
SSE.)

## Extraction & temp files

Extraction happens **in memory** for plain text, HTML, PDF (pure-Rust), and
Office Open XML (parsed straight from the zip). For higher-fidelity PDF/Office
text, `distill` will use `pdftotext`/`pandoc` if present — writing the bytes to
**one** uniquely-named temp file, converting, and deleting it immediately (RAII,
even on error). Documents are processed one at a time, so at most one temp file
exists at any instant, and the temp dir is swept on startup. The only persistent
output is the `.kb` index.
