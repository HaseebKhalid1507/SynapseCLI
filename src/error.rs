use thiserror::Error;

#[derive(Error, Debug)]
pub enum RuntimeError {
    #[error("API error: {0}")]
    Api(#[from] reqwest::Error),
    #[error("Tool execution failed: {0}")]
    Tool(String),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;
