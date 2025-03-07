// src/robots.rs
use log::{debug, warn};
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use url::Url;

use crate::error::{CrawlerError, Result};

#[derive(Debug, Clone)]
pub struct RobotsChecker {
    client: Client,
    cache: Arc<RwLock<HashMap<String, RobotsData>>>,
}

#[derive(Debug, Clone)]
struct RobotsData {
    allow_patterns: Vec<String>,
    disallow_patterns: Vec<String>,
    crawl_delay: Option<f64>,
}

impl RobotsChecker {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn is_allowed(&self, url: &str, user_agent: &str) -> Result<bool> {
        let parsed_url = Url::parse(url)
            .map_err(|e| CrawlerError::RobotsError(format!("Failed to parse URL: {}", e)))?;

        let domain = format!(
            "{}://{}",
            parsed_url.scheme(),
            parsed_url.host_str().unwrap_or_default()
        );

        let robots_url = format!("{}/robots.txt", domain);
        let path = parsed_url.path();

        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(robots_data) = cache.get(&domain) {
                return Ok(self.check_path_allowed(robots_data, path));
            }
        }

        // Fetch and parse robots.txt
        debug!("Fetching robots.txt from {}", robots_url);
        let robots_data = match self.fetch_and_parse_robots(&robots_url, user_agent).await {
            Ok(data) => data,
            Err(e) => {
                warn!("Error fetching robots.txt: {}, assuming allowed", e);
                RobotsData {
                    allow_patterns: vec![],
                    disallow_patterns: vec![],
                    crawl_delay: None,
                }
            }
        };

        // Cache the result
        {
            let mut cache = self.cache.write().await;
            cache.insert(domain, robots_data.clone());
        }

        Ok(self.check_path_allowed(&robots_data, path))
    }

    fn check_path_allowed(&self, robots_data: &RobotsData, path: &str) -> bool {
        // Check if path matches any disallow pattern
        for pattern in &robots_data.disallow_patterns {
            if path.starts_with(pattern) {
                // Check if there's a more specific allow pattern
                for allow_pattern in &robots_data.allow_patterns {
                    if path.starts_with(allow_pattern) && allow_pattern.len() > pattern.len() {
                        return true;
                    }
                }
                return false;
            }
        }

        // If no disallow pattern matches, it's allowed
        true
    }

    async fn fetch_and_parse_robots(
        &self,
        robots_url: &str,
        user_agent: &str,
    ) -> Result<RobotsData> {
        let response =
            self.client.get(robots_url).send().await.map_err(|e| {
                CrawlerError::RobotsError(format!("Failed to fetch robots.txt: {}", e))
            })?;

        if !response.status().is_success() {
            // If robots.txt doesn't exist or can't be retrieved, everything is allowed
            return Ok(RobotsData {
                allow_patterns: vec![],
                disallow_patterns: vec![],
                crawl_delay: None,
            });
        }

        let content = response
            .text()
            .await
            .map_err(|e| CrawlerError::RobotsError(format!("Failed to read robots.txt: {}", e)))?;

        // Parse robots.txt
        let mut current_agent = String::new();
        let mut allow_patterns = Vec::new();
        let mut disallow_patterns = Vec::new();
        let mut crawl_delay = None;

        for line in content.lines() {
            let line = line.trim();

            // Skip comments and empty lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim().to_lowercase();
                let value = value.trim();

                match key.as_str() {
                    "user-agent" => {
                        current_agent = value.to_string();
                    }
                    "allow" => {
                        if current_agent == "*" || current_agent == user_agent {
                            allow_patterns.push(value.to_string());
                        }
                    }
                    "disallow" => {
                        if current_agent == "*" || current_agent == user_agent {
                            if !value.is_empty() {
                                disallow_patterns.push(value.to_string());
                            }
                        }
                    }
                    "crawl-delay" => {
                        if current_agent == "*" || current_agent == user_agent {
                            if let Ok(delay) = value.parse::<f64>() {
                                crawl_delay = Some(delay);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(RobotsData {
            allow_patterns,
            disallow_patterns,
            crawl_delay,
        })
    }

    // Fixed: Added underscore to unused parameter name
    pub async fn get_crawl_delay(&self, domain: &str, _user_agent: &str) -> Option<Duration> {
        // Return the crawl delay if available
        let cache = self.cache.read().await;
        if let Some(robots_data) = cache.get(domain) {
            if let Some(delay) = robots_data.crawl_delay {
                return Some(Duration::from_secs_f64(delay));
            }
        }

        None
    }
}

