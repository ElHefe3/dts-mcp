use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    AlreadyExists(String),
    #[error("{0}")]
    InUse(String),
    #[error("storage error: {0}")]
    Storage(String),
}

impl AppError {
    pub fn storage(error: impl std::fmt::Display) -> Self {
        Self::Storage(error.to_string())
    }

    pub fn is_client_error(&self) -> bool {
        !matches!(self, Self::Storage(_))
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
