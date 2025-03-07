// src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CrawlerError {
    #[error("HTTP request failed: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("URL parsing failed: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Task join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),

    #[error("Robots.txt error: {0}")]
    RobotsError(String),

    #[error("Config error: {0}")]
    ConfigError(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Visualization error: {0}")]
    VisualizationError(String),
}

pub type Result<T> = std::result::Result<T, CrawlerError>;
