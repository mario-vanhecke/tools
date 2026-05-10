-- md migration 001: initial schema for the conversion vault.

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

-- Each row tracks one input file and its conversion state.
CREATE TABLE outputs (
  id              INTEGER PRIMARY KEY,
  input_path      TEXT NOT NULL UNIQUE,    -- vault-relative (forward slashes)
  output_path     TEXT,                     -- vault-relative path under output_dir; NULL until converted
  added_at        INTEGER NOT NULL,
  status          TEXT NOT NULL,
  status_detail   TEXT,
  status_note     TEXT,
  last_src_mtime  INTEGER,
  last_src_size   INTEGER,
  last_src_hash   TEXT,
  last_out_hash   TEXT,
  last_converted  INTEGER,
  extractor       TEXT,                     -- which extractor ran last
  attempts        INTEGER NOT NULL DEFAULT 0,
  last_attempt    INTEGER
);

CREATE INDEX idx_outputs_status      ON outputs(status);
CREATE UNIQUE INDEX idx_outputs_out  ON outputs(output_path) WHERE output_path IS NOT NULL;
