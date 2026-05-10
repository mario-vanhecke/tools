use serde_json::{json, Value};

pub const VAULT_NAME: &str = "vault.name";
pub const OUTPUT_DIR: &str = "output.dir";
pub const OUTPUT_ANNOTATE: &str = "output.annotate";
pub const OUTPUT_COLLISION_AWARE: &str = "output.collision_aware_naming";
pub const FILES_SUPPORTED_EXTENSIONS: &str = "files.supported_extensions";
pub const FILES_EXCLUDED_EXTENSIONS: &str = "files.excluded_extensions";
pub const FILES_SIZE_CAP_BYTES: &str = "files.size_cap_bytes";
pub const FILES_RESPECT_MDIGNORE: &str = "files.respect_mdignore";
pub const CONVERT_CONCURRENCY: &str = "convert.concurrency";

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

pub static KEYS: &[KeyDef] = &[
    KeyDef {
        key: VAULT_NAME,
        mutability: Mutability::Always,
        validator: validate_string,
        description: "Human-readable label for the conversion vault",
    },
    KeyDef {
        key: OUTPUT_DIR,
        mutability: Mutability::Always,
        validator: validate_string,
        description: "Where converted .md files are written. Vault-relative or absolute.",
    },
    KeyDef {
        key: OUTPUT_ANNOTATE,
        mutability: Mutability::Always,
        validator: validate_bool,
        description: "Emit an HTML-comment lineage marker at the top of every converted file",
    },
    KeyDef {
        key: OUTPUT_COLLISION_AWARE,
        mutability: Mutability::Always,
        validator: validate_bool,
        description: "When two inputs would produce the same output filename, include the source extension (foo.pdf.md vs foo.epub.md)",
    },
    KeyDef {
        key: FILES_SUPPORTED_EXTENSIONS,
        mutability: Mutability::Always,
        validator: validate_string_array,
        description: "Extensions handled by `md` (lowercase, no dot)",
    },
    KeyDef {
        key: FILES_EXCLUDED_EXTENSIONS,
        mutability: Mutability::Always,
        validator: validate_string_array,
        description: "Subset of supported extensions disabled in this vault",
    },
    KeyDef {
        key: FILES_SIZE_CAP_BYTES,
        mutability: Mutability::Always,
        validator: validate_non_negative_int,
        description: "Maximum input file size in bytes",
    },
    KeyDef {
        key: FILES_RESPECT_MDIGNORE,
        mutability: Mutability::Always,
        validator: validate_bool,
        description: "Honor .mdignore when adding files",
    },
    KeyDef {
        key: CONVERT_CONCURRENCY,
        mutability: Mutability::Always,
        validator: validate_positive_int,
        description: "Parallel extract operations during conversion",
    },
];

pub fn default_for(key: &str) -> Option<&'static Value> {
    static D_VAULT_NAME: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static D_OUTPUT_DIR: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static D_OUTPUT_ANNOTATE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static D_OUTPUT_COLLISION: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static D_SUPPORTED: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static D_EXCLUDED: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static D_SIZE_CAP: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static D_RESPECT_MDIGNORE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    static D_CONCURRENCY: std::sync::OnceLock<Value> = std::sync::OnceLock::new();

    Some(match key {
        VAULT_NAME => D_VAULT_NAME.get_or_init(|| Value::String(String::new())),
        OUTPUT_DIR => D_OUTPUT_DIR.get_or_init(|| Value::String("converted".to_string())),
        OUTPUT_ANNOTATE => D_OUTPUT_ANNOTATE.get_or_init(|| Value::Bool(true)),
        OUTPUT_COLLISION_AWARE => D_OUTPUT_COLLISION.get_or_init(|| Value::Bool(true)),
        FILES_SUPPORTED_EXTENSIONS => {
            D_SUPPORTED.get_or_init(|| json!(["md", "markdown", "docx", "pdf", "epub", "txt"]))
        }
        FILES_EXCLUDED_EXTENSIONS => D_EXCLUDED.get_or_init(|| json!([])),
        FILES_SIZE_CAP_BYTES => D_SIZE_CAP.get_or_init(|| Value::from(104_857_600u64)), // 100 MB
        FILES_RESPECT_MDIGNORE => D_RESPECT_MDIGNORE.get_or_init(|| Value::Bool(true)),
        CONVERT_CONCURRENCY => D_CONCURRENCY.get_or_init(|| Value::from(3u64)),
        _ => return None,
    })
}

pub fn lookup(key: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|k| k.key == key)
}
