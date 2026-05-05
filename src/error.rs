use thiserror::Error;

pub type Result<T> = std::result::Result<T, BinateError>;

#[derive(Debug, Error)]
pub enum BinateError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Binary parse error: {0}")]
    ObjectParse(#[from] object::read::Error),

    #[error("DWARF error: {0}")]
    Dwarf(#[from] gimli::Error),

    #[error("Unsupported architecture: {0}")]
    UnsupportedArch(String),

    #[error("Section not found: {0}")]
    SectionNotFound(String),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}
