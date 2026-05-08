use serde_json::{json, Value};

pub const VAULT_NAME: &str = "vault.name";
pub const EMBEDDING_MODEL: &str = "embedding.model";
pub const EMBEDDING_DIMENSION: &str = "embedding.dimension";
pub const EMBEDDING_DEVICE: &str = "embedding.device";
pub const EMBEDDING_BATCH_SIZE: &str = "embedding.batch_size";
pub const CHUNKING_TARGET_TOKENS: &str = "chunking.target_tokens";
pub const CHUNKING_MAX_TOKENS: &str = "chunking.max_tokens";
pub const CHUNKING_OVERLAP_TOKENS: &str = "chunking.overlap_tokens";
pub const FILES_SUPPORTED_EXTENSIONS: &str = "files.supported_extensions";
pub const FILES_EXCLUDED_EXTENSIONS: &str = "files.excluded_extensions";
pub const FILES_SIZE_CAP_BYTES: &str = "files.size_cap_bytes";
pub const FILES_RESPECT_GITIGNORE: &str = "files.respect_gitignore";
pub const FILES_RESPECT_VAULTIGNORE: &str = "files.respect_vaultignore";
pub const INDEXING_EXTRACT_CONCURRENCY: &str = "indexing.extract_concurrency";
pub const RETRIEVAL_DEFAULT_K: &str = "retrieval.default_k";
pub const RETRIEVAL_RRF_CONSTANT: &str = "retrieval.rrf_constant";

pub type ValidatorFn = fn(&Value) -> Result<(), String>;

#[derive(Clone, Copy, Debug)]
pub enum Mutability {
    Always,
    OnlyWhenNoChunksExist,
    Derived,
}

pub struct KeyDef {
    pub key: &'static str,
    pub default: Value,
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
fn validate_device(v: &Value) -> Result<(), String> {
    match v.as_str() {
        Some("auto") | Some("cpu") | Some("metal") | Some("cuda") => Ok(()),
        _ => Err(format!("must be one of: auto, cpu, metal, cuda; got {v}")),
    }
}
fn reject_derived(_: &Value) -> Result<(), String> {
    Err("derived; cannot be set directly".to_string())
}

pub fn lazy_default_supported() -> Value {
    json!(["md", "markdown", "docx", "pdf", "epub", "txt"])
}

pub static KEYS: &[KeyDef] = &[
    KeyDef {
        key: VAULT_NAME,
        default: Value::String(String::new()),
        mutability: Mutability::Always,
        validator: validate_string,
        description: "Human-readable label for the vault",
    },
    KeyDef {
        key: EMBEDDING_MODEL,
        default: Value::Null,
        mutability: Mutability::OnlyWhenNoChunksExist,
        validator: validate_string,
        description: "Hugging Face model ID for the embedder",
    },
    KeyDef {
        key: EMBEDDING_DIMENSION,
        default: Value::Null,
        mutability: Mutability::Derived,
        validator: reject_derived,
        description: "Embedding dimension (derived from the model)",
    },
    KeyDef {
        key: EMBEDDING_DEVICE,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_device,
        description: "Device used by the embedder: auto | cpu | metal | cuda",
    },
    KeyDef {
        key: EMBEDDING_BATCH_SIZE,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_positive_int,
        description: "Batch size for the embedder",
    },
    KeyDef {
        key: CHUNKING_TARGET_TOKENS,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_positive_int,
        description: "Target chunk size in tokens",
    },
    KeyDef {
        key: CHUNKING_MAX_TOKENS,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_positive_int,
        description: "Hard ceiling on chunk size in tokens",
    },
    KeyDef {
        key: CHUNKING_OVERLAP_TOKENS,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_non_negative_int,
        description: "Overlap between adjacent chunks in tokens",
    },
    KeyDef {
        key: FILES_SUPPORTED_EXTENSIONS,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_string_array,
        description: "File extensions handled by `rag` (lowercase, no dot)",
    },
    KeyDef {
        key: FILES_EXCLUDED_EXTENSIONS,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_string_array,
        description: "Subset of supported extensions disabled in this vault",
    },
    KeyDef {
        key: FILES_SIZE_CAP_BYTES,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_non_negative_int,
        description: "Maximum file size in bytes",
    },
    KeyDef {
        key: FILES_RESPECT_GITIGNORE,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_bool,
        description: "Honor .gitignore when adding files",
    },
    KeyDef {
        key: FILES_RESPECT_VAULTIGNORE,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_bool,
        description: "Honor .vaultignore when adding files",
    },
    KeyDef {
        key: INDEXING_EXTRACT_CONCURRENCY,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_positive_int,
        description: "Parallel extract operations during indexing",
    },
    KeyDef {
        key: RETRIEVAL_DEFAULT_K,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_positive_int,
        description: "Default result count for `rag search`",
    },
    KeyDef {
        key: RETRIEVAL_RRF_CONSTANT,
        default: Value::Null,
        mutability: Mutability::Always,
        validator: validate_positive_int,
        description: "Reciprocal-rank-fusion constant",
    },
];

/// Return the canonical default value for a key.
///
/// Defaults that depend on runtime state (e.g. vault name = directory name) are
/// resolved by callers; this function returns the *intrinsic* default.
pub fn default_for(key: &str) -> Option<&'static Value> {
    static D_VAULT_NAME: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::String(String::new()));
    static D_EMBEDDING_MODEL: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::String("BAAI/bge-m3".to_string()));
    static D_EMBEDDING_DIMENSION: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::from(1024u64));
    static D_EMBEDDING_DEVICE: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::String("auto".to_string()));
    static D_EMBEDDING_BATCH: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::from(64u64));
    static D_CHUNK_TARGET: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::from(400u64));
    static D_CHUNK_MAX: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::from(800u64));
    static D_CHUNK_OVERLAP: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::from(50u64));
    static D_FILES_SUPPORTED: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| json!(["md", "markdown", "docx", "pdf", "epub", "txt"]));
    static D_FILES_EXCLUDED: once_cell_lite::Lazy<Value> = once_cell_lite::Lazy::new(|| json!([]));
    static D_FILES_SIZE_CAP: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::from(52_428_800u64));
    static D_FILES_RESPECT_GIT: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::Bool(false));
    static D_FILES_RESPECT_VAULT: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::Bool(true));
    static D_EXTRACT_CONC: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::from(3u64));
    static D_DEFAULT_K: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::from(10u64));
    static D_RRF_CONST: once_cell_lite::Lazy<Value> =
        once_cell_lite::Lazy::new(|| Value::from(60u64));

    Some(match key {
        VAULT_NAME => &*D_VAULT_NAME,
        EMBEDDING_MODEL => &*D_EMBEDDING_MODEL,
        EMBEDDING_DIMENSION => &*D_EMBEDDING_DIMENSION,
        EMBEDDING_DEVICE => &*D_EMBEDDING_DEVICE,
        EMBEDDING_BATCH_SIZE => &*D_EMBEDDING_BATCH,
        CHUNKING_TARGET_TOKENS => &*D_CHUNK_TARGET,
        CHUNKING_MAX_TOKENS => &*D_CHUNK_MAX,
        CHUNKING_OVERLAP_TOKENS => &*D_CHUNK_OVERLAP,
        FILES_SUPPORTED_EXTENSIONS => &*D_FILES_SUPPORTED,
        FILES_EXCLUDED_EXTENSIONS => &*D_FILES_EXCLUDED,
        FILES_SIZE_CAP_BYTES => &*D_FILES_SIZE_CAP,
        FILES_RESPECT_GITIGNORE => &*D_FILES_RESPECT_GIT,
        FILES_RESPECT_VAULTIGNORE => &*D_FILES_RESPECT_VAULT,
        INDEXING_EXTRACT_CONCURRENCY => &*D_EXTRACT_CONC,
        RETRIEVAL_DEFAULT_K => &*D_DEFAULT_K,
        RETRIEVAL_RRF_CONSTANT => &*D_RRF_CONST,
        _ => return None,
    })
}

pub fn lookup(key: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|k| k.key == key)
}

mod once_cell_lite {
    /// Tiny once-cell wrapper using std::sync::OnceLock so we don't need to add a dep.
    pub struct Lazy<T: 'static> {
        cell: std::sync::OnceLock<T>,
        init: fn() -> T,
    }
    impl<T: 'static> Lazy<T> {
        pub const fn new(init: fn() -> T) -> Self {
            Self {
                cell: std::sync::OnceLock::new(),
                init,
            }
        }
    }
    impl<T: 'static> std::ops::Deref for Lazy<T> {
        type Target = T;
        fn deref(&self) -> &T {
            self.cell.get_or_init(self.init)
        }
    }
}
