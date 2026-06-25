pub mod keys;

use crate::error::{Error, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub use keys::{KeyDef, Mutability, KEYS};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentsConfig {
    pub extensions: Vec<String>,
    pub excluded_extensions: Vec<String>,
    pub size_cap_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlConfig {
    pub hash: bool,
    pub follow_symlinks: bool,
    pub respect_crawlignore: bool,
    pub default_strategy: String,
    pub default_max_depth: u64,
    pub concurrency: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub vault_name: String,
    pub documents: DocumentsConfig,
    pub crawl: CrawlConfig,
}

impl Config {
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut current: HashMap<String, Value> = HashMap::new();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
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
            documents: DocumentsConfig {
                extensions: as_string_array(&g(keys::DOCUMENTS_EXTENSIONS)).unwrap_or_default(),
                excluded_extensions: as_string_array(&g(keys::DOCUMENTS_EXCLUDED_EXTENSIONS))
                    .unwrap_or_default(),
                size_cap_bytes: g(keys::DOCUMENTS_SIZE_CAP_BYTES).as_u64().unwrap_or(0),
            },
            crawl: CrawlConfig {
                hash: g(keys::CRAWL_HASH).as_bool().unwrap_or(false),
                follow_symlinks: g(keys::CRAWL_FOLLOW_SYMLINKS).as_bool().unwrap_or(false),
                respect_crawlignore: g(keys::CRAWL_RESPECT_CRAWLIGNORE).as_bool().unwrap_or(true),
                default_strategy: g(keys::CRAWL_DEFAULT_STRATEGY)
                    .as_str()
                    .unwrap_or("recursive")
                    .to_string(),
                default_max_depth: g(keys::CRAWL_DEFAULT_MAX_DEPTH).as_u64().unwrap_or(0),
                concurrency: g(keys::CRAWL_CONCURRENCY).as_u64().unwrap_or(4) as u32,
            },
        })
    }

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

    pub fn set(conn: &Connection, key: &str, value: Value) -> Result<()> {
        let def = keys::lookup(key)
            .ok_or_else(|| Error::config(format!("unknown config key '{key}'")))?;
        if matches!(def.mutability, Mutability::Derived) {
            return Err(Error::config(format!(
                "{key} is derived and cannot be set directly"
            )));
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
                None => (
                    keys::default_for(def.key).cloned().unwrap_or(Value::Null),
                    true,
                    None,
                ),
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
