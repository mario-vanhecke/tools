# ADR 0002: Sources and documents as the data model

## Status

Accepted. 2026-06-24.

## Context

`rag` and `md` each track a single kind of thing: files registered for indexing
or conversion. `crawl`'s job is different — it *discovers* documents in places
it is told about. A discovery tool needs to represent both "where to look" and
"what was found", and those have very different shapes and lifecycles. A place
is configured once and crawled many times; a document appears, changes, and
disappears.

## Decision

Model two first-class tables joined by a third:

- **`sources`** — where to look. One row per local dir / SMB share / SharePoint
  drive, with a `kind`, a root `uri`, a `strategy`, and a JSON `config` blob for
  kind- and strategy-specific parameters.
- **`documents`** — what was found. One row per discovered file, keyed uniquely
  by `(source_id, uri)`, carrying name/path/size/mtime/hash/metadata and a
  lifecycle `status`.
- **`runs`** — one crawl pass over one source, recording counts and timing, so
  every document has a provenance trail (`first_run_id`, `last_run_id`) and
  "what's new since the last run" is a cheap query.

`ON DELETE CASCADE` ties documents and runs to their source: removing a source
removes everything discovered through it.

## Consequences

**Why:** the split keeps configuration (rarely changing, human-authored) cleanly
separated from discoveries (high-volume, machine-written). Provenance via `runs`
makes the lifecycle observable — you can answer "what changed this crawl" and
"what did this source contribute" without extra bookkeeping. A unique
`(source_id, uri)` is the natural identity of a document and makes upsert on
re-crawl trivial.

**Cost:** two crawls of overlapping trees (e.g. a directory and its parent
registered as separate sources) record the same file twice, once per source.
We accept this: sources are intentionally independent, and de-duplication, if
ever wanted, belongs at export time, not in the registry.
