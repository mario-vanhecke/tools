use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("config: {0}")]
    Config(String),

    #[error("invalid config file: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("embedding endpoint: {0}")]
    Embed(String),

    #[error("http: {0}")]
    Http(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}
