# `rag`

A CLI tool for indexing and searching a directory of documents. One vault, one
SQLite database, twelve commands. No daemon, no LLM, no agent.

`rag` does retrieval. Generation, agents, and chat are deliberately out of
scope — `rag search --json` is the boundary; higher-level tools build on top.

---

## Install

**macOS / Linux:**

```sh
curl -fsSL https://github.com/mario-vanhecke/rag/raw/main/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://github.com/mario-vanhecke/rag/raw/main/install.ps1 | iex
```

**From source** (any platform with Rust toolchain):

```sh
cargo install --git https://github.com/mario-vanhecke/rag rag-cli
```

Optional: install [pandoc](https://pandoc.org/installing.html) if you want
DOCX, PDF, and EPUB support (`brew install pandoc` / `apt install pandoc` /
`winget install pandoc`).

## Quickstart

```sh
cd ~/notes
rag init .                 # creates ./.vault/
rag add docs/              # registers each matching file as 'pending'
rag index                  # downloads bge-m3 on first use, ~2.2 GB
rag search "branching strategy"
```

---

## What you get

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
