use crate::cli::{ConfigAction, ConfigCmd};
use crate::commands::open_vault;
use crate::output::emit_json;
use crawl_core::config::{keys, Config};
use serde_json::{json, Value};
use std::path::Path;

pub fn run(cmd: ConfigCmd, json_out: bool, vault_arg: Option<&Path>) -> anyhow::Result<i32> {
    let vault = open_vault(vault_arg)?;
    match cmd.action {
        ConfigAction::Get { key } => {
            let v = Config::get(&vault.conn, &key)?;
            if json_out {
                emit_json(&v)?;
            } else {
                println!("{}", v);
            }
        }
        ConfigAction::Set { key, value } => {
            let parsed = parse_value(&key, &value)?;
            Config::set(&vault.conn, &key, parsed.clone())?;
            if json_out {
                emit_json(&json!({"key": key, "value": parsed}))?;
            } else {
                println!("Updated: {} = {}", key, parsed);
            }
        }
        ConfigAction::Unset { key } => {
            Config::unset(&vault.conn, &key)?;
            if json_out {
                emit_json(&json!({"unset": key}))?;
            } else {
                println!("Unset {}", key);
            }
        }
        ConfigAction::List { modified, defaults } => {
            if defaults {
                let mut out = Vec::new();
                for d in keys::KEYS {
                    out.push(json!({
                        "key": d.key,
                        "default": keys::default_for(d.key),
                        "description": d.description,
                    }));
                }
                if json_out {
                    emit_json(&json!({"defaults": out}))?;
                } else {
                    for v in &out {
                        println!("{:<34} {}", v["key"].as_str().unwrap(), v["default"]);
                    }
                }
            } else {
                let entries = Config::list_all(&vault.conn)?;
                let filtered: Vec<_> = if modified {
                    entries.into_iter().filter(|e| !e.is_default).collect()
                } else {
                    entries
                };
                if json_out {
                    emit_json(&json!({"settings": filtered}))?;
                } else {
                    for e in &filtered {
                        let mark = if e.is_default { "  " } else { "* " };
                        println!("{}{:<34} {}", mark, e.key, e.value);
                    }
                }
            }
        }
    }
    Ok(0)
}

fn parse_value(key: &str, raw: &str) -> anyhow::Result<Value> {
    let _def = keys::lookup(key).ok_or_else(|| {
        anyhow::Error::from(crawl_core::Error::Config(format!(
            "unknown config key '{key}'"
        )))
    })?;
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        return Ok(v);
    }
    if let Some(d) = keys::default_for(key) {
        match d {
            Value::Bool(_) => match raw {
                "true" | "1" | "yes" | "on" => return Ok(Value::Bool(true)),
                "false" | "0" | "no" | "off" => return Ok(Value::Bool(false)),
                _ => {}
            },
            Value::Number(_) => {
                if let Ok(n) = raw.parse::<u64>() {
                    return Ok(Value::from(n));
                }
                if let Ok(n) = raw.parse::<i64>() {
                    return Ok(Value::from(n));
                }
            }
            Value::Array(_) => {
                let parts: Vec<Value> = raw
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| Value::String(s.to_string()))
                    .collect();
                return Ok(Value::Array(parts));
            }
            _ => {}
        }
    }
    Ok(Value::String(raw.to_string()))
}
