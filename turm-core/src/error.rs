use thiserror::Error;

#[derive(Debug, Error)]
pub enum TurmError {
    #[error("PTY error: {0}")]
    Pty(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Protocol error: {0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, TurmError>;
