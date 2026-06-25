-- crawl migration 001: initial schema for the discovery vault.
--
-- Where `rag` stores chunks and `md` stores conversion outputs, `crawl`
-- stores two things: the *sources* it knows how to crawl, and the
-- *documents* it has discovered in them across one or more crawl runs.

CREATE TABLE IF NOT EXISTS schema_migrations (
  version    INTEGER PRIMARY KEY,
  applied_at INTEGER NOT NULL
);

CREATE TABLE vault_meta (
  id           INTEGER PRIMARY KEY CHECK (id = 1),
  vault_id     TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  tool_version TEXT NOT NULL
);

CREATE TABLE settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

-- A source is a place crawl knows how to enumerate: a local directory, a
-- mounted network/SMB share, or a SharePoint drive. `config` holds the
-- kind- and strategy-specific parameters as JSON (auth never lives here —
-- SharePoint secrets are read from an environment variable at run time).
CREATE TABLE sources (
  id           INTEGER PRIMARY KEY,
  name         TEXT NOT NULL UNIQUE,     -- short handle, e.g. "team-share"
  kind         TEXT NOT NULL,            -- local | smb | sharepoint
  uri          TEXT NOT NULL,            -- root locator (path, UNC, drive selector)
  strategy     TEXT NOT NULL,            -- recursive | shallow | incremental | targeted
  config       TEXT NOT NULL DEFAULT '{}',
  enabled      INTEGER NOT NULL DEFAULT 1,
  added_at     INTEGER NOT NULL,
  last_crawled INTEGER,
  last_run_id  INTEGER,
  last_status  TEXT,                     -- ok | error | partial
  last_error   TEXT
);

-- One crawl pass over one source. Gives every document a provenance trail and
-- powers "what's new since the last run".
CREATE TABLE runs (
  id          INTEGER PRIMARY KEY,
  source_id   INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
  strategy    TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  finished_at INTEGER,
  discovered  INTEGER NOT NULL DEFAULT 0,  -- rows first seen this run
  updated     INTEGER NOT NULL DEFAULT 0,  -- rows that changed this run
  gone        INTEGER NOT NULL DEFAULT 0,  -- rows that vanished this run
  skipped     INTEGER NOT NULL DEFAULT 0,  -- items filtered out (ext/ignore/glob)
  errors      INTEGER NOT NULL DEFAULT 0,
  status      TEXT NOT NULL,               -- ok | error | partial
  note        TEXT
);

-- Each row is one discovered document. `uri` is the canonical, stable locator
-- (absolute path for local/smb, webUrl for SharePoint) and is unique per source.
CREATE TABLE documents (
  id            INTEGER PRIMARY KEY,
  source_id     INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
  uri           TEXT NOT NULL,
  name          TEXT NOT NULL,
  rel_path      TEXT,                     -- path under the source root (forward slashes)
  extension     TEXT,                     -- lowercase, no dot; NULL if none
  size          INTEGER,
  modified_ms   INTEGER,                  -- source-reported last-modified (epoch ms)
  content_hash  TEXT,                     -- sha256 (local --hash) or provider hash; NULL otherwise
  metadata      TEXT NOT NULL DEFAULT '{}',
  status        TEXT NOT NULL,            -- present | modified | gone | too_large | error
  status_note   TEXT,
  discovered_at INTEGER NOT NULL,
  first_run_id  INTEGER,
  last_seen     INTEGER NOT NULL,
  last_run_id   INTEGER,
  UNIQUE (source_id, uri)
);

CREATE INDEX idx_documents_status    ON documents(status);
CREATE INDEX idx_documents_source    ON documents(source_id);
CREATE INDEX idx_documents_extension ON documents(extension);
CREATE INDEX idx_documents_name      ON documents(name);
CREATE INDEX idx_runs_source         ON runs(source_id);
