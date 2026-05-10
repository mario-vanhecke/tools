pub mod keys;

use crate::error::{Error, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub use keys::{KeyDef, Mutability, KEYS};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub dir: String,
    pub annotate: bool,
    pub collision_aware_naming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesConfig {
    pub supported_extensions: Vec<String>,
    pub excluded_extensions: Vec<String>,
    pub size_cap_bytes: u64,
    pub respect_mdignore: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertConfig {
    pub concurrency: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub vault_name: String,
    pub output: OutputConfig,
    pub files: FilesConfig,
    pub convert: ConvertConfig,
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
            output: OutputConfig {
                dir: g(keys::OUTPUT_DIR)
                    .as_str()
                    .unwrap_or("converted")
                    .to_string(),
                annotate: g(keys::OUTPUT_ANNOTATE).as_bool().unwrap_or(true),
                collision_aware_naming: g(keys::OUTPUT_COLLISION_AWARE).as_bool().unwrap_or(true),
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
                size_cap_bytes: g(keys::FILES_SIZE_CAP_BYTES)
                    .as_u64()
                    .unwrap_or(104_857_600),
                respect_mdignore: g(keys::FILES_RESPECT_MDIGNORE).as_bool().unwrap_or(true),
            },
            convert: ConvertConfig {
                concurrency: g(keys::CONVERT_CONCURRENCY).as_u64().unwrap_or(3) as u32,
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
