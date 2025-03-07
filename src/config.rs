// src/config.rs
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::error::{CrawlerError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlerConfig {
    pub max_depth: u32,
    pub concurrent_tasks: usize,
    #[serde(with = "duration_serde")]
    pub request_timeout: Duration,
    #[serde(with = "duration_serde")]
    pub crawl_timeout: Duration,
    #[serde(with = "duration_serde")]
    pub delay_between_requests: Duration,
    pub user_agent: String,
    pub respect_robots_txt: bool,
    pub follow_redirects: bool,
    pub allowed_domains: Vec<String>,
    pub excluded_paths: Vec<String>,
    pub max_urls_per_domain: Option<usize>,
    pub max_total_urls: Option<usize>,
}

impl Default for CrawlerConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            concurrent_tasks: 8,
            request_timeout: Duration::from_secs(10),
            crawl_timeout: Duration::from_secs(120),
            delay_between_requests: Duration::from_millis(100),
            user_agent: "RustCrawler/1.0 (https://example.com/bot)".to_string(),
            respect_robots_txt: true,
            follow_redirects: true,
            allowed_domains: Vec::new(),
            excluded_paths: Vec::new(),
            max_urls_per_domain: None,
            max_total_urls: None,
        }
    }
}

// Helper module for serializing Duration
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let millis = duration.as_millis();
        millis.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<CrawlerConfig> {
    // Fix: Use path.as_ref() to avoid moving the path
    let content = fs::read_to_string(path.as_ref())
        .map_err(|e| CrawlerError::ConfigError(format!("Failed to read config file: {}", e)))?;

    // Now path hasn't been moved
    let extension = path
        .as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    match extension {
        "toml" => toml::from_str(&content)
            .map_err(|e| CrawlerError::ConfigError(format!("Failed to parse TOML: {}", e))),
        "json" => serde_json::from_str(&content)
            .map_err(|e| CrawlerError::ConfigError(format!("Failed to parse JSON: {}", e))),
        "yaml" | "yml" => serde_yaml::from_str(&content)
            .map_err(|e| CrawlerError::ConfigError(format!("Failed to parse YAML: {}", e))),
        _ => Err(CrawlerError::ConfigError(
            "Unsupported config file format".to_string(),
        )),
    }
}

pub fn create_example_config<P: AsRef<Path>>(path: P) -> Result<()> {
    let config = CrawlerConfig::default();
    let extension = path
        .as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    let content = match extension {
        "toml" => toml::to_string_pretty(&config).map_err(|e| {
            CrawlerError::ConfigError(format!("Failed to serialize to TOML: {}", e))
        })?,
        "json" => serde_json::to_string_pretty(&config).map_err(|e| {
            CrawlerError::ConfigError(format!("Failed to serialize to JSON: {}", e))
        })?,
        "yaml" | "yml" => serde_yaml::to_string(&config).map_err(|e| {
            CrawlerError::ConfigError(format!("Failed to serialize to YAML: {}", e))
        })?,
        _ => {
            return Err(CrawlerError::ConfigError(
                "Unsupported config file format".to_string(),
            ))
        }
    };

    fs::write(path, content)
        .map_err(|e| CrawlerError::ConfigError(format!("Failed to write config file: {}", e)))?;

    Ok(())
}

