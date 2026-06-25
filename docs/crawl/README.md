# `crawl` — find documents wherever they live

`crawl` is a small CLI in the same spirit as [`rag`](https://github.com/mario-vanhecke/tools)
(index & search) and `md` (convert to markdown). Where those operate on files
you already have, `crawl` goes and **finds** them: it registers crawl
**sources** — local directories, mounted network/SMB shares, and SharePoint
document libraries — applies a per-source **strategy** to enumerate each one,
and records every discovered **document** into a small SQLite registry.

The registry is the payoff. Once `crawl` knows where your documents are, you can
list them, search them by name, watch them change over time, and **export** a
clean file list to feed straight into `rag add` or `md add`.

```sh
crawl init .
crawl source add team-share   local      /Volumes/team/Documents
crawl source add field-laptop smb        '\\nas01\projects'
crawl source add marketing    sharepoint contoso \
      --set site_hostname=contoso.sharepoint.com --set site_path=/sites/Marketing
crawl run
crawl status
crawl export --format paths | xargs -I{} echo {}    # → feed rag / md
```

It shares the toolkit's conventions: a vault directory holds a SQLite manifest
under `.crawl/`, every command speaks `--json`, configuration lives in the
vault's `settings` table (no config file to edit), and it ships as a single
static binary.

---

## Install

### One-line installer (prebuilt binary)

**macOS / Linux:**

```sh
curl -fsSL https://github.com/mario-vanhecke/tools/raw/main/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://github.com/mario-vanhecke/tools/raw/main/install.ps1 | iex
```

The installer ships the whole toolkit (`rag`, `md`, `crawl`). For just crawl:
`RAG_TOOLS=crawl curl -fsSL https://github.com/mario-vanhecke/tools/raw/main/install.sh | sh`.

### From source (any platform with a Rust toolchain)

```sh
cargo install --git https://github.com/mario-vanhecke/tools crawl-cli
# or, in a clone:
cargo build --release        # produces target/release/crawl
```

`crawl` is pure Rust with no required runtime dependencies and ships as a single
static binary. Local and SMB crawling work out of the box; SMB shares must be
**mounted** first (see below). SharePoint works with your normal interactive
sign-in or browser session — **no app registration required** (see below).

### Verify

```sh
$ crawl --version
crawl 0.1.0
```

---

## Concepts

| Thing | What it is |
|---|---|
| **Vault** | A directory with a `.crawl/` state dir holding the SQLite manifest. Discovered by walking up from the cwd, like `rag`/`md`. |
| **Source** | A place `crawl` knows how to enumerate: `local`, `smb`, or `sharepoint`. Has a strategy and a JSON config. |
| **Strategy** | *How* to enumerate a source: `recursive`, `shallow`, `incremental`, or `targeted`. |
| **Document** | One discovered file, with a canonical URI, name, size, modified time, extension, and a lifecycle status. |
| **Run** | One crawl pass over one source. Gives every document a provenance trail and powers "what's new since last run". |

---

## The commands

| Command | What it does |
|---|---|
| `crawl init [dir]` | Create a new vault at `dir` (default cwd). |
| `crawl source add <name> <kind> <uri>` | Register a source. |
| `crawl source ls` / `show <name>` | List sources, or inspect one. |
| `crawl source rm <name>` | Remove a source and its discovered documents. |
| `crawl source enable/disable <name>` | Toggle whether `crawl run` includes it. |
| `crawl run` | Crawl sources; upsert discovered documents; mark vanished ones `gone`. |
| `crawl ls` | List discovered documents (`--status`, `--source`, `--ext`, `--limit`). |
| `crawl status` | Vault state: sources, document counts, what's new. |
| `crawl find <query>` | Search documents by name/URI substring. |
| `crawl rm <uris>...` | Deregister documents by URI. |
| `crawl prune` | Delete documents in a terminal status (default: `gone`). |
| `crawl export` | Emit documents as `paths` / `jsonl` / `csv`. |
| `crawl config get/set/unset/list` | Read or modify vault settings. |
| `crawl info [--check]` | Vault metadata, counts, and consistency checks. |

Every command accepts `--json`, `--vault <path>`, `--quiet`, and `--verbose`.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | General error (or a source hard-errored during `run`) |
| 2 | Invalid arguments (unknown source/kind/strategy) |
| 3 | No vault found |
| 4 | Vault corruption / schema mismatch |
| 5 | Configuration error |
| 6 | I/O error |
| 7 | Lock contention (`crawl run --no-wait`) |
| 10 | Source unreachable (share not mounted, SharePoint auth/network failure) |

---

## Strategies — how `crawl` decides what to enumerate

A strategy is the answer to *"find documents on its own."* Each source picks one
(`--strategy`, default `recursive`); `crawl run --strategy ...` overrides it for
one pass.

| Strategy | Traversal | Use it when |
|---|---|---|
| `recursive` | The entire tree, every level. The exhaustive default — and the only one that can mark deleted files `gone`. | You want a complete picture. |
| `shallow` | Only the top level of the root (depth 1). | Fast reconnaissance of a big tree. |
| `incremental` | Only items modified since the last successful crawl (a delta). | Cheap re-crawls of large, slow-changing sources. |
| `targeted` | Only files matching configured globs (e.g. `*.pdf`). | Hunting one class of document across a noisy tree. |

Strategy parameters live in the source config and can be set at `add` time:

```sh
crawl source add big local /data --strategy shallow
crawl source add pdfs local /data --strategy targeted --include '*.pdf' --include '*.docx'
crawl source add nas  smb  /Volumes/nas --strategy incremental
crawl source add deep local /data --max-depth 3
```

> **Only an exhaustive pass marks files `gone`.** `incremental` and `shallow`
> visit a *subset* of the tree, so a file they don't see means "not visited",
> not "deleted". Run a `recursive` crawl to reconcile deletions.

---

## Source kinds

### `local` — a directory on this machine

```sh
crawl source add docs local ./Documents
```

The path is stored absolute. A `.crawlignore` file at the **source root**
(gitignore syntax) prunes paths, on top of always-on built-in defaults
(`.git/`, `node_modules/`, `.DS_Store`, the `.crawl/` state dir, …).

### `smb` — a mounted network share

Portable userspace SMB is heavyweight; the standard, reliable approach is to
**mount** the share and let `crawl` walk it as a filesystem path. The crawler
accepts an already-mounted path, a `mount` config override, or a UNC/`smb://`
locator it resolves against common mount roots:

```sh
# macOS: mount, then crawl
mount_smbfs //user@nas01/projects /Volumes/projects
crawl source add proj smb /Volumes/projects

# Or give it the UNC and let it find the mount:
crawl source add proj smb '\\nas01\projects'

# Or pin the mount explicitly:
crawl source add proj smb '\\nas01\projects' --set mount=/Volumes/projects
```

If the share isn't mounted, `crawl run` fails with an actionable message (exit
code 10) instead of silently finding nothing.

### `sharepoint` — a Microsoft Graph document library

`crawl` reaches SharePoint Online through either the Microsoft Graph API or
SharePoint's own REST API, with several authentication modes (config key
`auth`). Pick by how you want to sign in:

**Interactive sign-in that avoids the Graph admin wall (`browser_rest`).** If
`browser` mode signs you in but then 403s on the libraries, your tenant gates
Graph's SharePoint scopes behind admin consent. `browser_rest` sidesteps that: it
signs in interactively for the **SharePoint resource** (the way `az login` does)
and uses SharePoint's REST API directly — no admin consent, no `az` install, and
the token **auto-refreshes** (unlike `cookie`).

```sh
crawl source add coe sharepoint contoso.sharepoint.com/sites/Marketing \
  --set auth=browser_rest \
  --set site_hostname=contoso.sharepoint.com \
  --set site_path=/sites/Marketing
crawl run --source coe
```

It uses the Azure CLI's public client by default (broadly pre-approved in
tenants). If your tenant still prompts for consent, that prompt is usually
user-grantable — accept it.

---

The Graph-based modes (use these when your tenant doesn't gate Graph):

**Interactive browser sign-in (`browser`, recommended).** Log in as yourself in
a real browser — username / password / MFA, whatever your tenant requires.
`crawl` opens the browser, you sign in, and it catches the redirect on
`http://localhost`. No device-code screen (which Conditional Access policies
often block), no secret. The token is cached under `.crawl/` (with a refresh
token) so later crawls run unattended.

```sh
crawl source add mkt sharepoint contoso --set auth=browser \
  --set site_hostname=contoso.sharepoint.com \
  --set site_path=/sites/Marketing \
  --set tenant_id=<tenant-id-or-domain>
crawl run --source mkt
#   → Opening your browser to sign in to SharePoint…
```

Uses OAuth2 authorization code + PKCE. The default `client_id` is Microsoft's
first-party Graph PowerShell public client (delegated, no app registration); if
your tenant blocks unknown apps, register a **public client** app — no secret —
with redirect URI `http://localhost`, and pass `--set client_id=<id>`.

**Sign in with a code on another device (`device_code`, the default).** Like
`browser`, but `crawl` prints a code you enter at `microsoft.com/devicelogin`.
Good for headless/SSH where no browser can open locally. Note some tenants block
the device-code flow via Conditional Access — if sign-in stalls, use `browser`
or `azure_cli` instead.

```sh
crawl source add mkt sharepoint contoso \
  --set site_hostname=contoso.sharepoint.com --set site_path=/sites/Marketing \
  --set tenant_id=<tenant-id-or-domain>
crawl run --source mkt
#   → To sign in, open https://microsoft.com/devicelogin and enter CODE …
```

**Reuse an existing `az login` (`azure_cli`).** Zero setup if you already use the
Azure CLI — `crawl` shells out to `az account get-access-token`:

```sh
az login                                  # interactive, once
crawl source add mkt sharepoint contoso --set auth=azure_cli \
  --set site_hostname=contoso.sharepoint.com --set site_path=/sites/Marketing
crawl run --source mkt
```

**Browser session cookies (`cookie`).** The escape hatch when your tenant gates
Graph behind admin consent (you sign in fine but get 403 on libraries). This
talks to SharePoint's *own* REST API as your browser does — no Graph, no admin,
no app, no `az`. You copy two cookies from your logged-in browser:

```sh
# In your browser, open the SharePoint site, then DevTools → Application →
# Cookies → https://contoso.sharepoint.com, and copy the FedAuth and rtFa values:
export CRAWL_SHAREPOINT_COOKIE='FedAuth=<value>; rtFa=<value>'

crawl source add coe sharepoint contoso.sharepoint.com/sites/Marketing \
  --set auth=cookie \
  --set site_hostname=contoso.sharepoint.com \
  --set site_path=/sites/Marketing
crawl run --source coe
```

It lists the site's document libraries and walks each. Caveat: those cookies
**expire after a few hours** — when a crawl returns 401/403, re-copy them and
re-export. Great for pulling documents now; not for an unattended daily job
(use `client_credentials` for that).

**Unattended app-only (`client_credentials`).** Best for cron/CI: an Azure AD app
registration with `Sites.Read.All` / `Files.Read.All` **application** permission.
The **client secret is read from an environment variable** and never written to
the vault.

```sh
export CRAWL_SHAREPOINT_SECRET='<client-secret>'
crawl source add mkt sharepoint contoso --set auth=client_credentials \
  --set tenant_id=<tenant-id> --set client_id=<app-client-id> \
  --set site_hostname=contoso.sharepoint.com --set site_path=/sites/Marketing
crawl run --source mkt
```

| Config key | Meaning |
|---|---|
| `auth` | `browser_rest` · `browser` · `device_code` (default) · `azure_cli` · `client_credentials` · `cookie` |
| `tenant_id` | Azure AD tenant id or domain (required for `client_credentials`) |
| `client_id` | App (client) id (required for `client_credentials`; defaults to a public client for `browser`/`device_code`) |
| `secret_env` | Env var holding the client secret (`client_credentials`; default `CRAWL_SHAREPOINT_SECRET`) |
| `drive_id` | Crawl one specific library by id, **or** … |
| `site_hostname` + `site_path` | … resolve the site and crawl **all** its libraries |
| `folder_path` | Start under this folder, relative to the drive root |
| `az_path` | Path to the `az` binary (`azure_cli`; default `az`) |
| `scopes` | Delegated scopes for sign-in (default: `offline_access Sites.Read.All`; add `Files.Read.All` if a tenant needs it) |
| `cookie_env` | Env var holding the SharePoint session cookie (`cookie`; default `CRAWL_SHAREPOINT_COOKIE`) |
| `all_sites` | (cookie/browser_rest) discover **every** accessible site via Search and crawl each |
| `sites_query` / `sites_filter` / `max_sites` | Tune `all_sites`: the search query, a URL substring filter, and a safety cap (default 50) |
| `graph_base` / `oauth_base` | Endpoint overrides for sovereign/GCC-High clouds (and tests) |

**Tenant-wide crawl (`all_sites`).** With a REST auth mode (`cookie` /
`browser_rest`), `crawl` can enumerate every site you can access (via the
SharePoint Search API) and crawl each recursively — instead of one site. Each
document's path is prefixed with its site so they stay distinct.

```sh
# Crawl up to 20 sites whose URL contains "/sites/Engineering":
crawl source add tenant sharepoint contoso.sharepoint.com \
  --set auth=cookie --set site_hostname=contoso.sharepoint.com \
  --set all_sites=true --set sites_filter=/sites/Engineering --set max_sites=20
crawl run --source tenant
#   → Discovered N site(s) to crawl
```

Mind the scale: a corporate tenant can have hundreds of sites and *many*
thousands of documents. Start with `sites_filter` and a small `max_sites`, then
widen. Sites on a different host (e.g. your `-my` OneDrive) are skipped, since a
cookie/token is scoped to one host.

**Troubleshooting "0 documents" / 403 on a library.** If sign-in succeeds but a
crawl reports `could not list document libraries … 403`, the token lacks
SharePoint *content* access. Re-run with `crawl run --reauth` (discards the
cached token and signs in again, requesting `Sites.Read.All`/`Files.Read.All`).
If the sign-in then says **"need admin approval"**, your tenant gates those
delegated scopes — either have an admin consent them, or use `--set
auth=azure_cli` after `az login` (the Azure CLI app is usually already
approved). To target one library directly, pass `--set drive_id=<id>`.

Given a site (no `drive_id`), `crawl` lists **every** document library in the
site and crawls each — so a restricted or empty default "Documents" library
never hides the rest. With several libraries, paths are prefixed by the library
name. To target just one library, pass its `--set drive_id=<id>`.

Each document's URI is its SharePoint `webUrl`; change detection uses Graph's
`quickXorHash` (no download needed). `incremental` uses the Graph `/delta`
endpoint and stores a delta link on the source; it does not reconcile deletions
(a `recursive` crawl does).

---

## Document lifecycle

A document row has exactly one status:

| Status | Meaning |
|---|---|
| `present` | Seen in the most recent crawl; unchanged or first sighting. |
| `modified` | Seen, but size/mtime/hash differ from last time. Resolves to `present` on the next clean crawl. |
| `gone` | Previously discovered, absent from the most recent **full** crawl of its source. |
| `too_large` | Found, but over `documents.size_cap_bytes`; recorded, never hashed. |
| `error` | The crawler could not read or stat the item. |

Only `crawl run` drives transitions. `crawl rm` and `crawl prune` only delete.

---

## Configuration

All configuration lives in the vault's `settings` table — there is no config
file. Use `crawl config`:

```sh
crawl config set documents.extensions pdf,docx,xlsx,pptx
crawl config set crawl.hash true
crawl config list --defaults
```

| Key | Default | Meaning |
|---|---|---|
| `documents.extensions` | a broad office-doc set | Extensions recorded. Empty array = every file. |
| `documents.excluded_extensions` | `[]` | Extensions to skip. |
| `documents.size_cap_bytes` | `0` (no cap) | Files over this are `too_large`, never hashed. |
| `crawl.hash` | `false` | Compute sha256 for local/smb docs (exact change detection). |
| `crawl.follow_symlinks` | `false` | Follow symlinks when walking local/smb. |
| `crawl.respect_crawlignore` | `true` | Honor `.crawlignore` at each local source root. |
| `crawl.default_strategy` | `recursive` | Strategy for `source add` when `--strategy` omitted. |
| `crawl.default_max_depth` | `0` (unlimited) | Default depth when a source pins none. |

---

## Feeding the rest of the toolkit

`crawl export` is the bridge. By default it emits only live (`present` /
`modified`) documents:

```sh
# Index everything crawl found, with rag:
crawl export --format paths --ext pdf > /tmp/pdfs.txt
xargs -a /tmp/pdfs.txt rag add

# Or a structured manifest:
crawl export --format jsonl > manifest.jsonl
crawl export --format csv   > inventory.csv
```

(`paths` yields filesystem paths for `local`/`smb` sources and `webUrl`s for
SharePoint.)

---

## On-disk layout

```
my-vault/
├── .crawl/
│   ├── manifest          # the SQLite database
│   └── crawl.lock        # crawl run file lock
└── ...                   # your content (or sources may live elsewhere entirely)
```

Per-local-source ignore files live at each source root as `.crawlignore`.

## Database schema

Five tables: `vault_meta`, `settings`, `sources`, `runs`, `documents`. Inspect
with `sqlite3 .crawl/manifest`. See
[`crates/crawl-core/src/db/migrations/001_initial.sql`](crates/crawl-core/src/db/migrations/001_initial.sql).

---

## Architecture

- `crates/vault-core` — shared lifecycle plumbing (SQLite connection,
  migrations runner, walk-up discovery, file locking, gitignore matcher).
  Identical to the crate in the `rag`/`md` toolkit, so `crawl` can drop straight
  into that workspace.
- `crates/crawl-core` — the library: sources, documents, the `Crawler` trait and
  its three implementations, the strategy engine, the run orchestrator.
- `crates/crawl-cli` — the `crawl` binary, a thin frontend over the library.

The core/CLI split is mandatory: the library knows nothing about argument
parsing, terminal output, or exit codes.

Decisions are recorded in [`docs/adr/`](docs/adr/):

1. [Reuse the toolkit's Rust + vault-core foundation](docs/adr/0001-reuse-toolkit-foundation.md)
2. [Sources and documents as the data model](docs/adr/0002-sources-and-documents.md)
3. [Pluggable per-source crawl strategies](docs/adr/0003-pluggable-strategies.md)
4. [SharePoint via Microsoft Graph, secrets from the environment](docs/adr/0004-sharepoint-via-graph.md)
5. [SharePoint auth modes and the REST fallback](docs/adr/0005-sharepoint-auth-and-rest.md)

---

## Testing locally

**1. Run the test suite** (no network, no credentials):

```sh
cargo test                              # 28 unit + integration tests, <2s
cargo test --test sharepoint_mock       # just the SharePoint flows
```

The integration suite (`crates/crawl-core/tests/`) covers the full document
lifecycle (discover → modified → gone → prune), content-hash change detection,
dry-run safety, the four strategies (including the invariant that partial crawls
never falsely mark files `gone`), and all four SharePoint auth modes against a
mock Graph server.

**2. Crawl a real local directory** — the quickest hands-on check:

```sh
cargo build --release
BIN="$PWD/target/release/crawl"

mkdir -p /tmp/crawltest/docs && cd /tmp/crawltest
printf x > docs/report.pdf; printf y > docs/sheet.xlsx; printf z > docs/sub/notes.txt
"$BIN" init .
"$BIN" source add mine local ./docs
"$BIN" run
"$BIN" status
"$BIN" ls
"$BIN" find report --ext pdf
"$BIN" export --format paths        # feed `rag add` / `md add`
"$BIN" info --check
```

Try the strategies: `crawl run --strategy shallow`, edit/delete a file and
re-run to watch `modified` / `gone`, or add `--set include_globs` with the
`targeted` strategy.

**3. Crawl an SMB share** — mount it first, then point `crawl` at the mount (or
the UNC and let it resolve):

```sh
"$BIN" source add nas smb /Volumes/projects        # already-mounted path
"$BIN" source add nas smb '\\nas01\projects'        # or a UNC to resolve
```

**4. Test SharePoint with no Azure tenant** — a bundled fake Graph server:

```sh
./scripts/try-sharepoint-offline.sh    # builds, starts the mock, crawls it
```

`scripts/mock-graph.py` is a ~60-line stand-in for Microsoft Graph; the script
wires a SharePoint source to it and runs a crawl. Use it to exercise the
SharePoint path (auth, pagination, folder recursion) without credentials.

**5. Test SharePoint against your real tenant** — use `auth=browser` (see the
[`sharepoint`](#sharepoint--a-microsoft-graph-document-library) section) and run
`crawl run`; it opens your browser to sign in.

## License

MIT. See `LICENSE`.
