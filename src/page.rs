// src/page.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub url: String,
    pub links: Vec<String>,
    pub depth: u32,
    pub title: Option<String>,
    pub content_type: Option<String>,
    pub status_code: Option<u16>,
    pub size: Option<usize>,
    pub crawled_at: Option<DateTime<Utc>>,
}

impl Page {
    pub fn new(url: String, depth: u32) -> Self {
        Self {
            url,
            links: Vec::new(),
            depth,
            title: None,
            content_type: None,
            status_code: None,
            size: None,
            crawled_at: None,
        }
    }

    pub fn with_links(mut self, links: Vec<String>) -> Self {
        self.links = links;
        self
    }

    pub fn with_title(mut self, title: String) -> Self {
        self.title = Some(title);
        self
    }

    pub fn with_content_type(mut self, content_type: String) -> Self {
        self.content_type = Some(content_type);
        self
    }

    pub fn with_status_code(mut self, status_code: u16) -> Self {
        self.status_code = Some(status_code);
        self
    }

    pub fn with_size(mut self, size: usize) -> Self {
        self.size = Some(size);
        self
    }

    pub fn mark_crawled(mut self) -> Self {
        self.crawled_at = Some(Utc::now());
        self
    }
}
