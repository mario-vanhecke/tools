use serde_json::{json, Value};

pub const VAULT_NAME: &str = "vault.name";
pub const DOCUMENTS_EXTENSIONS: &str = "documents.extensions";
pub const DOCUMENTS_EXCLUDED_EXTENSIONS: &str = "documents.excluded_extensions";
pub const DOCUMENTS_SIZE_CAP_BYTES: &str = "documents.size_cap_bytes";
pub const CRAWL_HASH: &str = "crawl.hash";
pub const CRAWL_FOLLOW_SYMLINKS: &str = "crawl.follow_symlinks";
pub const CRAWL_RESPECT_CRAWLIGNORE: &str = "crawl.respect_crawlignore";
pub const CRAWL_DEFAULT_STRATEGY: &str = "crawl.default_strategy";
pub const CRAWL_DEFAULT_MAX_DEPTH: &str = "crawl.default_max_depth";
pub const CRAWL_CONCURRENCY: &str = "crawl.concurrency";

pub type ValidatorFn = fn(&Value) -> Result<(), String>;

#[derive(Clone, Copy, Debug)]
pub enum Mutability {
    Always,
    Derived,
}

pub struct KeyDef {
    pub key: &'static str,
    pub mutability: Mutability,
    pub validator: ValidatorFn,
    pub description: &'static str,
}

fn validate_string(v: &Value) -> Result<(), String> {
    if v.is_string() {
        Ok(())
    } else {
        Err(format!("must be a string, got {v}"))
    }
}
fn validate_positive_int(v: &Value) -> Result<(), String> {
    match v.as_u64() {
        Some(n) if n >= 1 => Ok(()),
        _ => Err(format!("must be a positive integer, got {v}")),
    }
}
fn validate_non_negative_int(v: &Value) -> Result<(), String> {
    match v.as_u64() {
        Some(_) => Ok(()),
        None => Err(format!("must be a non-negative integer, got {v}")),
    }
}
fn validate_bool(v: &Value) -> Result<(), String> {
    if v.is_boolean() {
        Ok(())
    } else {
        Err(format!("must be a boolean, got {v}"))
    }
}
fn validate_string_array(v: &Value) -> Result<(), String> {
    match v.as_array() {
        Some(a) => {
            for x in a {
                if !x.is_string() {
                    return Err(format!("array must contain only strings, got {x}"));
                }
            }
            Ok(())
        }
        None => Err(format!("must be an array of strings, got {v}")),
    }
}
fn validate_strategy(v: &Value) -> Result<(), String> {
    match v.as_str() {
        Some("recursive" | "shallow" | "incremental" | "targeted") => Ok(()),
        _ => Err(format!(
            "must be one of recursive|shallow|incremental|targeted, got {v}"
        )),
    }
}

pub static KEYS: &[KeyDef] = &[
    KeyDef {
        key: VAULT_NAME,
        mutability: Mutability::Always,
        validator: validate_string,
        description: "Human-readable label for the discovery vault",
    },
    KeyDef {
        key: DOCUMENTS_EXTENSIONS,
        mutability: Mutability::Always,
        validator: validate_string_array,
        description:
            "Document extensions crawl records (lowercase, no dot). Empty = record every file.",
    },
    KeyDef {
        key: DOCUMENTS_EXCLUDED_EXTENSIONS,
        mutability: Mutability::Always,
        validator: validate_string_array,
        description: "Extensions to skip even if they match documents.extensions",
    },
    KeyDef {
        key: DOCUMENTS_SIZE_CAP_BYTES,
        mutability: Mutability::Always,
        validator: validate_non_negative_int,
        description:
            "Files larger than this are recorded as too_large and never hashed. 0 = no cap.",
    },
    KeyDef {
        key: CRAWL_HASH,
        mutability: Mutability::Always,
        validator: validate_bool,
        description:
            "Compute a sha256 content hash for local/smb documents (slower, exact change detection)",
    },
    KeyDef {
        key: CRAWL_FOLLOW_SYMLINKS,
        mutability: Mutability::Always,
        validator: validate_bool,
        description: "Follow symbolic links when walking local/smb sources",
    },
    KeyDef {
        key: CRAWL_RESPECT_CRAWLIGNORE,
        mutability: Mutability::Always,
        validator: validate_bool,
        description: "Honor a .crawlignore file at each local source root",
    },
    KeyDef {
        key: CRAWL_DEFAULT_STRATEGY,
        mutability: Mutability::Always,
        validator: validate_strategy,
        description: "Strategy applied to `source add` when --strategy is omitted",
    },
    KeyDef {
        key: CRAWL_DEFAULT_MAX_DEPTH,
        mutability: Mutability::Always,
        validator: validate_non_negative_int,
        description: "Default traversal depth when a source pins none. 0 = unlimited.",
    },
    KeyDef {
        key: CRAWL_CONCURRENCY,
        mutability: Mutability::Always,
        validator: validate_positive_int,
        description: "Reserved: parallel source crawls during `crawl run`",
    },
];

pub fn default_for(key: &str) -> Option<&'static Value> {
    use std::sync::OnceLock;
    static D_VAULT_NAME: OnceLock<Value> = OnceLock::new();
    static D_EXTENSIONS: OnceLock<Value> = OnceLock::new();
    static D_EXCLUDED: OnceLock<Value> = OnceLock::new();
    static D_SIZE_CAP: OnceLock<Value> = OnceLock::new();
    static D_HASH: OnceLock<Value> = OnceLock::new();
    static D_FOLLOW: OnceLock<Value> = OnceLock::new();
    static D_RESPECT: OnceLock<Value> = OnceLock::new();
    static D_STRATEGY: OnceLock<Value> = OnceLock::new();
    static D_MAX_DEPTH: OnceLock<Value> = OnceLock::new();
    static D_CONCURRENCY: OnceLock<Value> = OnceLock::new();

    Some(match key {
        VAULT_NAME => D_VAULT_NAME.get_or_init(|| Value::String(String::new())),
        DOCUMENTS_EXTENSIONS => D_EXTENSIONS.get_or_init(|| {
            json!([
                "pdf", "doc", "docx", "rtf", "odt", "pages", "txt", "md", "markdown", "xls",
                "xlsx", "ods", "numbers", "csv", "tsv", "ppt", "pptx", "odp", "key", "epub"
            ])
        }),
        DOCUMENTS_EXCLUDED_EXTENSIONS => D_EXCLUDED.get_or_init(|| json!([])),
        DOCUMENTS_SIZE_CAP_BYTES => D_SIZE_CAP.get_or_init(|| Value::from(0u64)),
        CRAWL_HASH => D_HASH.get_or_init(|| Value::Bool(false)),
        CRAWL_FOLLOW_SYMLINKS => D_FOLLOW.get_or_init(|| Value::Bool(false)),
        CRAWL_RESPECT_CRAWLIGNORE => D_RESPECT.get_or_init(|| Value::Bool(true)),
        CRAWL_DEFAULT_STRATEGY => D_STRATEGY.get_or_init(|| Value::String("recursive".to_string())),
        CRAWL_DEFAULT_MAX_DEPTH => D_MAX_DEPTH.get_or_init(|| Value::from(0u64)),
        CRAWL_CONCURRENCY => D_CONCURRENCY.get_or_init(|| Value::from(4u64)),
        _ => return None,
    })
}

pub fn lookup(key: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|k| k.key == key)
}
