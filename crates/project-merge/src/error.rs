use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    /// A declared solution path escapes its root, or a component is a symlink.
    /// The student cannot be allowed to redirect a solution path elsewhere.
    #[error("unsafe solution path `{0}`: {1}")]
    UnsafePath(String, &'static str),

    /// A file the assignment requires the student to provide is absent.
    #[error("student did not provide required file `{0}`")]
    MissingSolution(String),

    /// The declared solution path exists but is a directory / not a regular file.
    #[error("solution path `{0}` is not a regular file")]
    NotAFile(String),

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl MergeError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        MergeError::Io {
            path: path.into(),
            source,
        }
    }
}
