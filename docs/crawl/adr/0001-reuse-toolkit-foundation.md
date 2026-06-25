# ADR 0001: Reuse the toolkit's Rust + vault-core foundation

## Status

Accepted. 2026-06-24.

## Context

`crawl` is a new tool in the same family as `rag` (index & search) and `md`
(convert). Those share a foundation crate, `vault-core`, that provides SQLite
connection setup, a migrations runner, walk-up state-directory discovery, file
locking, and a gitignore-style matcher. They also share a set of conventions: a
vault is a directory with a `.<tool>/` state dir holding a single SQLite
manifest, every command emits `--json`, configuration lives in a `settings`
table rather than a file, and the binary is small and static.

We could have written `crawl` in another language or invented its own
plumbing.

## Decision

Build `crawl` in Rust (edition 2021, MSRV 1.75) as a `*-core` / `*-cli` pair
that depends on the **unmodified** `vault-core` crate, mirroring `rag` and `md`
exactly.

## Consequences

**Why:** a user who knows `rag` already knows `crawl` — same vault discovery,
same `--json` everywhere, same exit-code scheme, same config model, same
`.<tool>ignore` story. Reusing `vault-core` means the lifecycle plumbing is
already battle-tested, and `crawl` can be dropped straight into the
`mario-vanhecke/tools` workspace by deleting its vendored copy of `vault-core`
and pointing at the workspace one. The only change made to `vault-core` was
adding `.crawl/` and `.crawlignore` to the built-in ignore defaults — a natural
extension of "state directories of the tools in this distribution."

**Cost:** `crawl` lives in the same workspace as `rag`/`md` and shares their
`vault-core` directly, so the lifecycle plumbing stays in lockstep across all
three tools.
