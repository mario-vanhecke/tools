# ADR 0003: Pluggable per-source crawl strategies

## Status

Accepted. 2026-06-24.

## Context

The brief for `crawl` is that it must "find documents on its own … apply some
strategies to crawl different sources." Different sources warrant different
traversal policies: a small project folder wants an exhaustive walk; a giant,
slow-changing network share wants a cheap delta; a noisy tree wants to surface
only PDFs. And the *kind* of source (local filesystem vs. Microsoft Graph)
dictates *how* enumeration physically happens.

These are two orthogonal axes — **how physically** (per kind) and **how
thoroughly** (per strategy) — and conflating them would produce a combinatorial
mess.

## Decision

Separate the two axes:

- A **`Crawler` trait**, one implementation per source *kind* (`local`, `smb`,
  `sharepoint`), responsible for the physical enumeration. This mirrors the
  `Extractor` trait in the toolkit's `extract-core`.
- A **`Strategy`** enum (`recursive` / `shallow` / `incremental` / `targeted`)
  that resolves to a small `StrategyParams` value (depth cap, since-cutoff,
  include/exclude globs). Crawlers consult the params; the orchestrator applies
  the document filter (extension + globs) uniformly on top.

The orchestrator owns the lifecycle: it enumerates via the crawler, upserts each
recorded document, and — critically — only an **exhaustive** pass (no
since-cutoff, no depth cap) is permitted to mark unseen documents `gone`.
Partial strategies see a subset of the tree, so absence there means "not
visited," not "deleted."

## Consequences

**Why:** adding a new source kind is one trait impl; adding a new strategy is one
enum variant plus its param resolution. The `gone`-only-on-full-enumeration rule
prevents the most dangerous failure mode of a discovery tool — a cheap
incremental crawl wrongly declaring half your documents deleted. It is covered
by integration tests (`strategies.rs`).

**Cost:** strategy params are interpreted partly by the crawler (depth, since)
and partly by the orchestrator (globs, extensions). The division is documented
on `DocFilter` and `StrategyParams`, but it is a seam a contributor must learn.
