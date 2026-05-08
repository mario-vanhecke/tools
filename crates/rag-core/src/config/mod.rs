pub mod keys;

use crate::error::{Error, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub use keys::{KeyDef, Mutability, KEYS};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingDevice {
    Auto,
    Cpu,
    Metal,
    Cuda,
}

impl EmbeddingDevice {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "cpu" => Some(Self::Cpu),
            "metal" => Some(Self::Metal),
            "cuda" => Some(Self::Cuda),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Metal => "metal",
            Self::Cuda => "cuda",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub model: String,
    pub dimension: u32,
    pub device: EmbeddingDevice,
    pub batch_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkingConfig {
    pub target_tokens: u32,
    pub max_tokens: u32,
    pub overlap_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesConfig {
    pub supported_extensions: Vec<String>,
    pub excluded_extensions: Vec<String>,
    pub size_cap_bytes: u64,
    pub respect_gitignore: bool,
    pub respect_vaultignore: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingConfig {
    pub extract_concurrency: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalConfig {
    pub default_k: u32,
    pub rrf_constant: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub vault_name: String,
    pub embedding: EmbeddingConfig,
    pub chunking: ChunkingConfig,
    pub files: FilesConfig,
    pub indexing: IndexingConfig,
    pub retrieval: RetrievalConfig,
}

impl Config {
    /// Read all `settings` rows, layer over defaults, build a typed Config.
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut current: HashMap<String, Value> = HashMap::new();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (k, v) = row?;
            let parsed: Value = serde_json::from_str(&v).unwrap_or(Value::Null);
            current.insert(k, parsed);
        }

        let g = |k: &str| -> Value {
            if let Some(v) = current.get(k) {
                return v.clone();
            }
            keys::default_for(k).cloned().unwrap_or(Value::Null)
        };

        Ok(Self {
            vault_name: g(keys::VAULT_NAME).as_str().unwrap_or("").to_string(),
            embedding: EmbeddingConfig {
                model: g(keys::EMBEDDING_MODEL)
                    .as_str()
                    .unwrap_or("BAAI/bge-m3")
                    .to_string(),
                dimension: g(keys::EMBEDDING_DIMENSION).as_u64().unwrap_or(1024) as u32,
                device: EmbeddingDevice::parse(
                    g(keys::EMBEDDING_DEVICE).as_str().unwrap_or("auto"),
                )
                .unwrap_or(EmbeddingDevice::Auto),
                batch_size: g(keys::EMBEDDING_BATCH_SIZE).as_u64().unwrap_or(64) as u32,
            },
            chunking: ChunkingConfig {
                target_tokens: g(keys::CHUNKING_TARGET_TOKENS).as_u64().unwrap_or(400) as u32,
                max_tokens: g(keys::CHUNKING_MAX_TOKENS).as_u64().unwrap_or(800) as u32,
                overlap_tokens: g(keys::CHUNKING_OVERLAP_TOKENS).as_u64().unwrap_or(50) as u32,
            },
            files: FilesConfig {
                supported_extensions: as_string_array(&g(keys::FILES_SUPPORTED_EXTENSIONS))
                    .unwrap_or_else(|| {
                        vec!["md", "markdown", "docx", "pdf", "epub", "txt"]
                            .into_iter()
                            .map(String::from)
                            .collect()
                    }),
                excluded_extensions: as_string_array(&g(keys::FILES_EXCLUDED_EXTENSIONS))
                    .unwrap_or_default(),
                size_cap_bytes: g(keys::FILES_SIZE_CAP_BYTES).as_u64().unwrap_or(52_428_800),
                respect_gitignore: g(keys::FILES_RESPECT_GITIGNORE).as_bool().unwrap_or(false),
                respect_vaultignore: g(keys::FILES_RESPECT_VAULTIGNORE).as_bool().unwrap_or(true),
            },
            indexing: IndexingConfig {
                extract_concurrency: g(keys::INDEXING_EXTRACT_CONCURRENCY).as_u64().unwrap_or(3)
                    as u32,
            },
            retrieval: RetrievalConfig {
                default_k: g(keys::RETRIEVAL_DEFAULT_K).as_u64().unwrap_or(10) as u32,
                rrf_constant: g(keys::RETRIEVAL_RRF_CONSTANT).as_u64().unwrap_or(60) as u32,
            },
        })
    }

    /// Effective value of `key` (set or default).
    pub fn get(conn: &Connection, key: &str) -> Result<Value> {
        let row: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .ok();
        if let Some(s) = row {
            Ok(serde_json::from_str(&s)?)
        } else if let Some(d) = keys::default_for(key) {
            Ok(d.clone())
        } else {
            Err(Error::config(format!("unknown config key '{key}'")))
        }
    }

    /// Validate and persist `value` for `key`.
    pub fn set(conn: &Connection, key: &str, value: Value) -> Result<()> {
        let def = keys::lookup(key)
            .ok_or_else(|| Error::config(format!("unknown config key '{key}'")))?;
        match def.mutability {
            Mutability::Derived => {
                return Err(Error::config(format!(
                    "{key} is derived and cannot be set directly"
                )));
            }
            Mutability::OnlyWhenNoChunksExist => {
                let n: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
                if n > 0 {
                    return Err(Error::config(format!(
                        "cannot change {key} — vault has {n} indexed chunks. Run `rag rm --all` first."
                    )));
                }
            }
            Mutability::Always => {}
        }
        (def.validator)(&value).map_err(Error::config)?;

        let now = chrono::Utc::now().timestamp_millis();
        let value_str = serde_json::to_string(&value)?;
        conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value_str, now],
        )?;
        Ok(())
    }

    pub fn unset(conn: &Connection, key: &str) -> Result<()> {
        let _def = keys::lookup(key)
            .ok_or_else(|| Error::config(format!("unknown config key '{key}'")))?;
        conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        Ok(())
    }

    /// Enumerate all known keys with effective values + whether they came from defaults.
    pub fn list_all(conn: &Connection) -> Result<Vec<SettingEntry>> {
        let mut current: HashMap<String, (String, i64)> = HashMap::new();
        let mut stmt = conn.prepare("SELECT key, value, updated_at FROM settings")?;
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })? {
            let (k, v, ts) = row?;
            current.insert(k, (v, ts));
        }
        let mut out = Vec::with_capacity(KEYS.len());
        for def in KEYS {
            let (value, is_default, updated_at) = match current.get(def.key) {
                Some((v, ts)) => (
                    serde_json::from_str(v).unwrap_or(Value::Null),
                    false,
                    Some(*ts),
                ),
                None => (def.default.clone(), true, None),
            };
            out.push(SettingEntry {
                key: def.key.to_string(),
                value,
                is_default,
                updated_at,
                description: def.description.to_string(),
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingEntry {
    pub key: String,
    pub value: Value,
    pub is_default: bool,
    pub updated_at: Option<i64>,
    pub description: String,
}

fn as_string_array(v: &Value) -> Option<Vec<String>> {
    match v {
        Value::Array(a) => Some(
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
        ),
        _ => None,
    }
}
