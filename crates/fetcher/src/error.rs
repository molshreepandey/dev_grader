use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("not a valid github repository url: `{0}`")]
    InvalidUrl(String),

    #[error("unsupported host in url `{0}` (only github.com is allowed)")]
    UnsupportedHost(String),

    #[error("http error fetching repo: {0}")]
    Http(String),

    /// The archive exceeded the configured total/per-file byte budget (tar-bomb guard).
    #[error("archive too large (limit {limit} bytes)")]
    ArchiveTooLarge { limit: u64 },

    /// The archive contained more entries than allowed.
    #[error("archive has too many files (limit {limit})")]
    TooManyFiles { limit: usize },

    #[error("malformed archive: {0}")]
    Archive(String),

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl FetchError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        FetchError::Io {
            path: path.into(),
            source,
        }
    }
}
