// src/storage.rs
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::Path;

use crate::crawler::CrawlResult;
use crate::error::{CrawlerError, Result};

#[derive(Serialize, Deserialize)]
pub struct StoredCrawlResult {
    pub pages_count: usize,
    pub links_count: usize,
    pub crawl_duration_seconds: f64,
    pub success_count: usize,
    pub error_count: usize,
    pub pages: Vec<StoredPage>,
    pub graph: std::collections::HashMap<String, Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct StoredPage {
    pub url: String,
    pub title: Option<String>,
    pub depth: u32,
    pub status_code: Option<u16>,
    pub content_type: Option<String>,
    pub size_bytes: Option<usize>,
    pub links_count: usize,
    pub crawled_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub fn save_results<P: AsRef<Path>>(result: &CrawlResult, path: P) -> Result<()> {
    let extension = path
        .as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("json");

    // Create a serializable version of the results
    let stored_result = StoredCrawlResult {
        pages_count: result.pages.len(),
        links_count: result.total_links,
        crawl_duration_seconds: result.stats.duration_secs,
        success_count: result.stats.success_count,
        error_count: result.stats.error_count,
        pages: result
            .pages
            .iter()
            .map(|page| StoredPage {
                url: page.url.clone(),
                title: page.title.clone(),
                depth: page.depth,
                status_code: page.status_code,
                content_type: page.content_type.clone(),
                size_bytes: page.size,
                links_count: page.links.len(),
                crawled_at: page.crawled_at,
            })
            .collect(),
        graph: result.graph.clone(),
    };

    let file = File::create(path.as_ref())
        .map_err(|e| CrawlerError::StorageError(format!("Failed to create output file: {}", e)))?;

    match extension {
        "json" => {
            serde_json::to_writer_pretty(file, &stored_result)
                .map_err(|e| CrawlerError::StorageError(format!("Failed to write JSON: {}", e)))?;
        }
        "yaml" | "yml" => {
            serde_yaml::to_writer(file, &stored_result)
                .map_err(|e| CrawlerError::StorageError(format!("Failed to write YAML: {}", e)))?;
        }
        _ => {
            return Err(CrawlerError::StorageError(
                "Unsupported output file format. Use .json or .yaml".to_string(),
            ));
        }
    }

    Ok(())
}

// Add this line above the function
#[allow(dead_code)]
pub fn load_results<P: AsRef<Path>>(path: P) -> Result<StoredCrawlResult> {
    let extension = path
        .as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("json");

    let file = File::open(path.as_ref())
        .map_err(|e| CrawlerError::StorageError(format!("Failed to open file: {}", e)))?;

    match extension {
        "json" => serde_json::from_reader(file)
            .map_err(|e| CrawlerError::StorageError(format!("Failed to parse JSON: {}", e))),
        "yaml" | "yml" => serde_yaml::from_reader(file)
            .map_err(|e| CrawlerError::StorageError(format!("Failed to parse YAML: {}", e))),
        _ => Err(CrawlerError::StorageError(
            "Unsupported file format. Use .json or .yaml".to_string(),
        )),
    }
}
