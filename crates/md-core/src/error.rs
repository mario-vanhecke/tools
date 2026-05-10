use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Vault(#[from] vault_core::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("config: {0}")]
    Config(String),

    #[error("path not in registry: {0}")]
    PathNotInRegistry(String),

    #[error("conflict: {path} — both source and output have changed since the last conversion")]
    Conflict { path: String },

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other<S: Into<String>>(s: S) -> Self {
        Self::Other(s.into())
    }
    pub fn config<S: Into<String>>(s: S) -> Self {
        Self::Config(s.into())
    }
}
