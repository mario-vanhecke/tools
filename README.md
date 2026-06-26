# `rag` — a small distribution of CLI tools for working with knowledge

| Tool | What it does |
|------|--------------|
| **[`rag`](docs/rag/)** | Index and search a directory of documents. Hybrid retrieval (vector + FTS5) with strong consistency guarantees. |
| **[`md`](docs/md/)** | Convert documents (PDF, EPUB, DOCX, ...) to markdown with idempotent state tracking and bidirectional source ↔ output traceability. |
| **[`crawl`](docs/crawl/)** | Discover documents across local directories, mounted SMB shares, and SharePoint. Records what it finds into a registry you can feed to `rag` / `md`. |
| **[`distill`](docs/knowledge/)** | Build a single portable index (SQLite + sqlite-vec) from local/SMB/SharePoint sources, embedding via a pluggable endpoint. Reference-only — sources stay put. |
| **[`recall`](docs/knowledge/)** | Serve a `distill` index to an LLM harness (Claude Code, opencode) over **MCP**, so the model searches your documents itself — local (stdio) or remote (HTTP). |

`rag` / `md` / `crawl` share the same conventions: a vault directory holds a
small SQLite manifest and tracks items through a lifecycle (`add`/`run` →
process → `status`). All ship as small static binaries; `rag`/`md` use optional
`pandoc`/`poppler` for extra format support, and `crawl` is pure Rust.

`distill` + `recall` are an **independent, alternative** take on RAG (a single
`knowledge.toml`, remote embeddings with no bundled model, reference-only index,
served to the model over MCP). See **[docs/knowledge/](docs/knowledge/)**.

---

## Install

### Homebrew (macOS / Linux) — recommended

```sh
brew install mario-vanhecke/tools/rag    # rag: indexer & search
brew install mario-vanhecke/tools/md     # md:  converter
```

Each formula auto-installs `pandoc` (for DOCX/EPUB) and `poppler` (for
high-quality PDF extraction via `pdftotext`) as recommended dependencies.
Skip them with `--without-pandoc` / `--without-poppler` if you don't want
them.

### One-line installer (no Homebrew) — installs `rag`, `md`, and `crawl`

**macOS / Linux:**

```sh
curl -fsSL https://github.com/mario-vanhecke/tools/raw/main/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://github.com/mario-vanhecke/tools/raw/main/install.ps1 | iex
```

Set `RAG_TOOLS=rag` (or `md`, or `crawl`) to install just one. Optional tools
(`pandoc`, `poppler`) install separately — see the table below.

### Prebuilt bundle — download, unzip, run (no installer)

Each [release](https://github.com/mario-vanhecke/tools/releases/latest) ships a
single **`tools-<target>`** archive that contains all three tools (`crawl`,
`md`, `rag`) plus a `GETTING-STARTED.txt`, so one download does the whole
workflow:

| Platform | Asset |
|----------|-------|
| Windows  | `tools-x86_64-pc-windows-msvc.zip` |
| macOS (Apple Silicon) | `tools-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `tools-x86_64-apple-darwin.tar.gz` |
| Linux    | `tools-x86_64-unknown-linux-gnu.tar.gz` |

Unzip it, then run the binaries from that folder (or add it to your `PATH`).
The **Windows** bundle also includes the document converters (`pandoc` +
`poppler`/`pdftotext`) under `bin/`, so `.docx`/`.epub`/`.pdf` conversion works
offline with no extra installs — `md` finds them automatically next to its own
executable. (On macOS/Linux, install those via `brew`/`apt`.)

The one thing never bundled is `rag`'s embedding model (~2.2 GB), downloaded
on first `rag index` — it exceeds GitHub's 2 GB asset limit. See
`GETTING-STARTED.txt` in the archive for details.

### From source (any platform with Rust toolchain)

```sh
cargo install --git https://github.com/mario-vanhecke/tools rag-cli     # just rag
cargo install --git https://github.com/mario-vanhecke/tools md-cli      # just md
cargo install --git https://github.com/mario-vanhecke/tools crawl-cli   # just crawl
```

Add `--features metal` on Apple Silicon for ~9× faster embedding in `rag`,
or `--features cuda` on Linux with CUDA toolkit for NVIDIA acceleration.
`md` is CPU-only (no embedder).

---

### What formats are supported (applies to both `rag` and `md`)

| Format | Built-in | With `pandoc`     | With `poppler` (`pdftotext`) |
|--------|----------|-------------------|------------------------------|
| `.md` / `.markdown` | ✅ | — | — |
| `.txt`              | ✅ | — | — |
| `.docx`             | — | ✅ | — |
| `.epub`             | — | ✅ | — |
| `.pdf`              | ✅ (pure-Rust pdf-extract; some unusual fonts crash) | — | ✅ (recommended; handles everything pdf-extract can't) |

The PDF extractor picks its backend at startup: if `pdftotext` is on PATH,
it uses that. Otherwise it falls back to the bundled pure-Rust extractor.
Both produce the same text; the difference is reliability on hard PDFs
(image-heavy, unusual fonts, scanned scientific papers).

### Manual install instructions for the optional tools

| Tool      | macOS                    | Debian/Ubuntu              | Windows                   |
|-----------|--------------------------|----------------------------|---------------------------|
| `pandoc`  | `brew install pandoc`    | `apt install pandoc`       | `winget install pandoc`   |
| `poppler` | `brew install poppler`   | `apt install poppler-utils`| `choco install poppler`   |

### Verifying your setup

```sh
$ rag --version
rag 0.2.0
$ md --version
md 0.2.0
$ which pandoc pdftotext      # both present means you're set
/opt/homebrew/bin/pandoc
/opt/homebrew/bin/pdftotext
```

You can also check after a `rag index` or `md convert` run: failed PDFs
include a `status_note` that says either *"Install poppler..."* (you're
on the pure-Rust fallback) or a real pdftotext error message (you're on
the high-quality path).

## Quickstart

**Index and search** (using `rag`):

```sh
cd ~/notes
rag init .                 # creates ./.vault/
rag add docs/              # registers each matching file as 'pending'
rag index                  # downloads bge-m3 on first use, ~2.2 GB
rag search "branching strategy"
```

**Convert documents to markdown** (using `md`):

```sh
cd ~/library
md init .                  # creates ./.md/
md add books/              # registers source files as 'pending'
md convert                 # writes .md outputs under ./converted/
md whence converted/books/foo.md.md   # → tells you the source file
```

**Discover & fetch documents from anywhere** (using `crawl`):

```sh
cd ~/knowledge
crawl init .               # creates ./.crawl/
crawl source add docs  local      ./Documents
crawl source add team   sharepoint contoso.sharepoint.com/sites/Eng \
    --set auth=browser_rest --set site_hostname=contoso.sharepoint.com --set site_path=/sites/Eng
crawl discover             # records what it finds, recursively, across all sources
crawl fetch --out ./files  # materializes the actual files into one local tree
```

Each tool is independent — use one, two, or all three. They keep separate vault
directories (`.vault/`, `.md/`, `.crawl/`) so they never interfere in the same
tree. And they **compose** into a pipeline — see the next section.

---

## The workflow: discover → convert → search

The three tools chain into one end-to-end pipeline. `crawl` finds and fetches
documents from anywhere, `md` normalizes them to markdown, and `rag` makes them
searchable:

```sh
crawl discover                     # find docs across local dirs, SMB shares, SharePoint
crawl fetch --out ./files          # materialize one local tree (copy local/SMB, download SharePoint)

md add ./files && md convert       # → ./converted/**/*.md

rag add converted/ && rag index    # downloads the bge-m3 embedding model on first use
rag search "what's our Tier 1 SLA?"
```

The key is that **`crawl fetch` produces the same shape no matter the source** —
a tree of files under `files/<source>/<rel_path>` — so `md` and `rag` consume it
identically whether a document came from a laptop folder, a mounted network
share, or a SharePoint library. Every stage is incremental: re-running
`discover` / `fetch` / `convert` / `index` only processes what changed.

---

## What `rag` gets you

- **Hybrid retrieval** — vector (sqlite-vec) + full-text (FTS5) fused with
  reciprocal rank fusion.
- **Strong consistency** — every chunk in the index belongs to a file currently
  in `indexed` state. No orphan chunks, no stale results. See
  [ADR 0006](docs/adr/0006-consistency-invariant.md).
- **Single SQLite database per vault** — copy `.vault/vault` to back up.
- **Single static binary** — `cargo build --release` produces `rag` with no
  runtime deps beyond pandoc (optional, for DOCX/PDF).
- **JSON output everywhere** — every command accepts `--json` and emits a
  documented schema.

## The twelve commands

| Command | What it does |
|---|---|
| `rag init [dir]` | Create a new vault at `dir` (default cwd). |
| `rag add <paths>...` | Walk paths and register files as `pending`. |
| `rag rm <paths>...` | Deregister files. Cascade removes chunks. |
| `rag prune` | Delete registry rows in non-`indexed` states (default: `missing`). |
| `rag ls` | List registered files with their statuses. |
| `rag status` | Vault state report; detects on-disk drift since last index. |
| `rag index` | Process pending/modified files: extract → chunk → embed → write. |
| `rag search <query>` | Hybrid retrieval; print top-k passages. |
| `rag show <chunk-id-or-path>` | Display a chunk by ID, or a file's chunks by path. |
| `rag config get/set/unset/list` | Read or modify vault settings. |
| `rag info` | Vault metadata, counts, and (with `--check`) consistency checks. |

Run any command with `--help` for full flag documentation.

### Common flags (all commands)

- `--json` — emit JSON output to stdout
- `--vault <path>` — override the walk-up vault discovery
- `--quiet` / `--verbose` / `--color <when>`

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments or usage |
| 3 | No vault found |
| 4 | Vault corruption / schema mismatch |
| 5 | Configuration error |
| 6 | I/O error |
| 7 | Lock contention (`rag index --no-wait`) |
| 8 | Subprocess error (e.g., pandoc) |

---

## File-state lifecycle

A file in the registry has exactly one of these eight statuses:

| Status | Meaning | Has chunks? |
|---|---|---|
| `pending` | Registered, not yet indexed | No |
| `indexed` | Successfully indexed; content is in the index | **Yes** |
| `failed` | Last `rag index` errored | No |
| `unsupported` | Extension not in `files.supported_extensions` | No |
| `excluded` | Extension in `files.excluded_extensions` | No |
| `too_large` | File exceeds `files.size_cap_bytes` (default 50 MB) | No |
| `needs_ocr` | PDF (or similar) with no extractable text | No |
| `missing` | Registered, file no longer on disk | No |

Only `rag index` drives transitions on existing rows. `rag add` only inserts;
`rag rm` and `rag prune` only delete.

A ninth status, `stale`, is computed at `rag status` time (file mtime/size
differs from `last_mtime`/`last_size`). It's not persisted because `rag index`
immediately resolves it.

---

## Configuration

All configuration lives in the vault's `settings` table. There is no config
file you edit. Use `rag config`:

```sh
rag config get embedding.model
rag config set chunking.target_tokens 500
rag config unset chunking.target_tokens     # back to default
rag config list --modified                  # only keys you've changed
rag config list --defaults                  # built-in defaults
```

Notable keys (see `rag config list --defaults` for the full list):

| Key | Default | Mutability |
|---|---|---|
| `embedding.model` | `BAAI/bge-m3` | only when no chunks exist |
| `embedding.device` | `auto` | always (`auto` / `cpu` / `metal` / `cuda`) |
| `embedding.batch_size` | `64` | always (raise to 128+ on Metal/CUDA) |
| `chunking.target_tokens` | `400` | always |
| `chunking.max_tokens` | `800` | always |
| `chunking.overlap_tokens` | `50` | always |
| `files.supported_extensions` | `["md","markdown","docx","pdf","epub","txt"]` | always |
| `files.excluded_extensions` | `[]` | always |
| `files.size_cap_bytes` | `52428800` (50 MB) | always |
| `retrieval.default_k` | `10` | always |
| `retrieval.rrf_constant` | `60` | always |

---

## `.vaultignore`

A gitignore-shaped file at the vault root excludes paths during `rag add`.
Built-in defaults (always applied) cover `.git/`, `node_modules/`,
`__pycache__/`, `.DS_Store`, `*.pyc`, `.idea/`, `.vscode/`, `.vaultignore`
itself, and others.

```
# .vaultignore
drafts/
archive/
*.private.md
!important.private.md     # un-ignore a specific file
```

`rag add --no-ignore` bypasses `.vaultignore` (built-ins still apply).
`rag add --force` bypasses everything.

---

## On-disk layout

```
my-vault/
├── .vault/
│   ├── vault                        # the SQLite database (no extension)
│   ├── cache/models/<model-id>/     # downloaded model weights
│   ├── logs/                        # reserved for future use
│   └── index.lock                   # rag index file lock
├── .vaultignore                     # optional
└── ...                              # your content
```

---

## Database schema

The vault database has six tables and three triggers. Inspect with
`sqlite3 .vault/vault`.

```sql
schema_migrations(version PRIMARY KEY, applied_at)
vault_meta(id=1, vault_id, created_at, tool_version)
settings(key PRIMARY KEY, value JSON, updated_at)
files(id PRIMARY KEY, path UNIQUE, added_at, status, status_detail,
      status_note, last_mtime, last_size, last_hash, last_indexed,
      attempts, last_attempt)
chunks(id UUIDv7 PRIMARY KEY, file_id REFERENCES files ON DELETE CASCADE,
       ordinal, content, content_hash, heading_path, page_number,
       token_count, created_at)
chunk_vectors USING vec0(chunk_id PRIMARY KEY, embedding FLOAT[1024])
chunk_fts USING fts5(chunk_id UNINDEXED, content, heading_path,
                     tokenize='unicode61 remove_diacritics 2')
```

The `chunks → vectors/fts` cascade is enforced by trigger
`trg_chunks_after_delete`. See
[ADR 0006](docs/adr/0006-consistency-invariant.md) for why.

---

## Workflow recipes

### Start a vault, index a folder, search

```sh
cd ~/work/blueprint
rag init .
rag add docs/ adrs/
rag index
rag search "release cadence" --k 5
```

### Re-index after edits

```sh
rag status                  # shows 'modified: N' if files changed
rag index                   # only modified files are reprocessed
```

### Switch the embedding model

```sh
rag rm --all --yes
rag config set embedding.model "different-model"
rag add docs/
rag index
```

### Drop missing files

```sh
rag index                   # detects missing files (status='missing')
rag prune                   # removes them from the registry
```

### Inspect a single chunk

```sh
rag --json search "trunk-based" --k 1 | jq -r '.results[0].chunk_id' \
  | xargs rag show
```

### CI / scripting

```sh
# Index in CI; fail loudly on any error
rag index --json | jq -e '.summary.failed == 0'
# Verify invariants
rag info --check --json | jq -e '.checks | all(.)'
```

---

## Architecture

- `crates/rag-core` — library: indexing, retrieval, schema, configuration.
- `crates/rag-cli` — binary: `rag`, a thin frontend over the library.

The split is mandatory: the CLI consumes the library; the library knows
nothing about argument parsing, terminal output, or exit codes. A future
daemon, GUI, or alternate frontend can consume `rag-core` directly.

Architecture decisions are documented in
[`docs/adr/`](docs/adr/):

1. [Rust as the implementation language](docs/adr/0001-rust-language.md)
2. [Candle as the embedding backend](docs/adr/0002-candle-embedder.md)
3. [Pandoc subprocess for DOCX/PDF](docs/adr/0003-pandoc-extractor.md)
4. [A vault is a single SQLite database](docs/adr/0004-database-as-vault.md)
5. [Snapshot semantics for `rag add`](docs/adr/0005-snapshot-semantics.md)
6. [The consistency invariant](docs/adr/0006-consistency-invariant.md)

---

## Testing

```sh
cargo test                  # 48 unit + integration tests, runs in <1s
                            #   (uses a deterministic stub embedder)
```

The integration suite covers the full lifecycle, the consistency invariant
under failure conditions, all 7 reachable status transitions, re-index
semantics, prune variants, search modes, the file lock under concurrency,
and `.vaultignore` behavior. See `crates/rag-core/tests/`.

---

## License

MIT. See `LICENSE`.
