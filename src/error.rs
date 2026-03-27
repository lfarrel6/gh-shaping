use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parse error in {file}: {source}")]
    Yaml {
        file: PathBuf,
        source: serde_yaml::Error,
    },
    #[error("git ls-remote failed for {owner}/{repo}: {message}")]
    Git {
        owner: String,
        repo: String,
        message: String,
    },
    #[error("ref not found: {ref_str} in {owner}/{repo}")]
    RefNotFound {
        owner: String,
        repo: String,
        ref_str: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
