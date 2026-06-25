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

    #[error("no source named '{0}'")]
    NoSuchSource(String),

    #[error("a source named '{0}' already exists")]
    DuplicateSource(String),

    #[error("unknown source kind '{0}' (expected: local, smb, sharepoint)")]
    UnknownKind(String),

    #[error("unknown crawl strategy '{0}' (expected: recursive, shallow, incremental, targeted)")]
    UnknownStrategy(String),

    /// A crawl source could not be reached (mount missing, auth failed, ...).
    #[error("source '{name}' unreachable: {detail}")]
    Unreachable { name: String, detail: String },

    /// SharePoint / Graph auth or transport failure.
    #[error("sharepoint: {0}")]
    SharePoint(String),

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
    pub fn unreachable<S: Into<String>, D: Into<String>>(name: S, detail: D) -> Self {
        Self::Unreachable {
            name: name.into(),
            detail: detail.into(),
        }
    }
}
