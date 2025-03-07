// src/crawler.rs
use chrono::Utc;
use futures::future::join_all;
use log::{debug, error, info, warn};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, Semaphore};
use url::Url;

use crate::config::CrawlerConfig;
use crate::error::{CrawlerError, Result};
use crate::page::Page;
use crate::robots::RobotsChecker;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlStats {
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: chrono::DateTime<chrono::Utc>,
    pub duration_secs: f64,
    pub success_count: usize,
    pub error_count: usize,
    pub avg_page_size: usize,
}

#[derive(Debug, Clone)]
pub struct CrawlResult {
    pub pages: Vec<Page>,
    pub graph: HashMap<String, Vec<String>>,
    pub total_links: usize,
    pub stats: CrawlStats,
}

pub struct Crawler {
    visited: Arc<Mutex<HashSet<String>>>,
    graph: Arc<Mutex<HashMap<String, Vec<String>>>>,
    pages: Arc<Mutex<Vec<Page>>>,
    config: CrawlerConfig,
    client: Client,
    limiter: Arc<Semaphore>,
    robots_checker: RobotsChecker,
    domain_counters: Arc<Mutex<HashMap<String, usize>>>,
    stats: Arc<Mutex<CrawlStats>>,
}

impl Crawler {
    pub fn new(config: CrawlerConfig) -> Result<Self> {
        // Create HTTP client with proper settings
        let client = Client::builder()
            .user_agent(&config.user_agent)
            .timeout(config.request_timeout)
            .redirect(if config.follow_redirects {
                reqwest::redirect::Policy::limited(10)
            } else {
                reqwest::redirect::Policy::none()
            })
            .build()
            .map_err(CrawlerError::RequestError)?;

        // Store the concurrent_tasks value before moving config
        let concurrent_tasks = config.concurrent_tasks;

        // Initialize robots.txt checker with the same client
        let robots_checker = RobotsChecker::new(client.clone());

        // Initialize stats
        let stats = Arc::new(Mutex::new(CrawlStats {
            started_at: Utc::now(),
            finished_at: Utc::now(),
            duration_secs: 0.0,
            success_count: 0,
            error_count: 0,
            avg_page_size: 0,
        }));

        Ok(Crawler {
            visited: Arc::new(Mutex::new(HashSet::new())),
            graph: Arc::new(Mutex::new(HashMap::new())),
            pages: Arc::new(Mutex::new(Vec::new())),
            config: config.clone(), // Clone the config here
            client,
            limiter: Arc::new(Semaphore::new(concurrent_tasks)),
            robots_checker,
            domain_counters: Arc::new(Mutex::new(HashMap::new())),
            stats,
        })
    }

    pub async fn crawl(&self, start_url: &str) -> Result<CrawlResult> {
        let start_time = Instant::now();
        info!("🚀 Starting crawler at: {}", start_url);

        // Update start time in stats
        {
            let mut stats = self.stats.lock().await;
            stats.started_at = Utc::now();
        }

        // Create a channel for communication between workers
        let (tx, mut rx) = mpsc::channel(100);

        // Create the starting point
        let start = Page::new(start_url.to_string(), 0);

        // Mark the start URL as visited right away
        {
            let mut visited = self.visited.lock().await;
            visited.insert(start_url.to_string());
        }

        tx.send(start).await.map_err(|e| {
            CrawlerError::ConfigError(format!("Failed to send initial page: {}", e))
        })?;

        // Set up worker tasks to process URLs
        let mut handles = vec![];

        // Main processing loop
        loop {
            tokio::select! {
                // Try to receive a message with timeout
                maybe_page = tokio::time::timeout(Duration::from_millis(100), rx.recv()) => {
                    match maybe_page {
                        // We received a page to process
                        Ok(Some(page)) => {
                            // Skip if we've reached max depth
                            if page.depth >= self.config.max_depth {
                                debug!("🛑 Reached max depth ({}) for {}", self.config.max_depth, page.url);
                                continue;
                            }

                            // Check max URLs per domain limit
                            if let Some(max_per_domain) = self.config.max_urls_per_domain {
                                let domain = self.extract_domain(&page.url).unwrap_or_default();
                                let mut domain_counters = self.domain_counters.lock().await;
                                let count = domain_counters.entry(domain.clone()).or_insert(0);
                                if *count >= max_per_domain {
                                    debug!("Reached max URLs for domain {}: {}", domain, max_per_domain);
                                    continue;
                                }
                                *count += 1;
                            }

                            // Check max total URLs limit
                            if let Some(max_total) = self.config.max_total_urls {
                                let visited_count = self.visited.lock().await.len();
                                if visited_count >= max_total {
                                    debug!("Reached max total URLs: {}", max_total);
                                    break;
                                }
                            }

                            // Clone what we need for the task
                            let crawler = self.clone();
                            let page_url = page.url.clone();
                            let page_depth = page.depth;
                            let tx = tx.clone();
                            let page_clone = page.clone();

                            // Spawn a new task to process this page
                            let handle = tokio::spawn(async move {
                                // Acquire a permit from the semaphore to limit concurrency
                                let _permit = crawler.limiter.acquire().await.unwrap();

                                info!("📊 Processing {} at depth {}/{}",
                                    page_url, page_depth, crawler.config.max_depth);

                                // Process the page and handle any links found
                                match crawler.process_page(&page_clone).await {
                                    Ok((processed_page, links)) => {
                                        // Save the processed page
                                        {
                                            let mut pages = crawler.pages.lock().await;
                                            pages.push(processed_page);
                                        }

                                        // Update the graph with new links
                                        {
                                            let mut graph = crawler.graph.lock().await;
                                            graph.insert(page_url.clone(), links.clone());
                                        }

                                        // Update success stats
                                        {
                                            let mut stats = crawler.stats.lock().await;
                                            stats.success_count += 1;
                                        }

                                        // Queue up new pages for processing
                                        for link in links {
                                            // Check if we've already visited this URL
                                            let should_queue = {
                                                let mut visited = crawler.visited.lock().await;
                                                if !visited.contains(&link) {
                                                    // Mark as visited preemptively
                                                    visited.insert(link.clone());
                                                    true
                                                } else {
                                                    false
                                                }
                                            };

                                            // Check domain/path filtering
                                            let allowed_domain = if !crawler.config.allowed_domains.is_empty() {
                                                let domain = crawler.extract_domain(&link).unwrap_or_default();
                                                crawler.config.allowed_domains.iter().any(|d| domain.contains(d))
                                            } else {
                                                true
                                            };

                                            let excluded_path = if !crawler.config.excluded_paths.is_empty() {
                                                crawler.config.excluded_paths.iter().any(|p| link.contains(p))
                                            } else {
                                                false
                                            };

                                            if should_queue && allowed_domain && !excluded_path {
                                                let new_page = Page::new(link.clone(), page_depth + 1);
                                                debug!("➡️  Queueing {} (at depth {})", link, new_page.depth);

                                                if tx.send(new_page).await.is_err() {
                                                    warn!("❌ Channel closed, exiting");
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("⚠️  Error processing {}: {}", page_url, e);

                                        // Update error stats
                                        {
                                            let mut stats = crawler.stats.lock().await;
                                            stats.error_count += 1;
                                        }
                                    }
                                }
                            });

                            handles.push(handle);

                            // Clean up completed handles
                            handles.retain(|h| !h.is_finished());
                        },
                        // Channel is empty for now, but might get more messages later
                        Ok(None) => {
                            // If all tasks are done and channel is empty, we're finished
                            if handles.is_empty() {
                                break;
                            }

                            // Otherwise wait a bit
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        },
                        // Timeout reached while waiting for new messages
                        Err(_) => {
                            // Check if any tasks are still running
                            if handles.is_empty() {
                                break;
                            }

                            // Clean up completed handles
                            handles.retain(|h| !h.is_finished());
                        }
                    }
                },

                // Check for a timeout on the entire crawl operation
                _ = tokio::time::sleep(self.config.crawl_timeout) => {
                    info!("⚠️  Crawl timed out after {} seconds!", self.config.crawl_timeout.as_secs());
                    break;
                }
            }
        }

        // Wait for remaining tasks to complete with timeout
        if !handles.is_empty() {
            match tokio::time::timeout(Duration::from_secs(5), join_all(handles)).await {
                Ok(results) => {
                    for result in results {
                        if let Err(e) = result {
                            error!("⚠️  A worker task failed: {}", e);
                        }
                    }
                }
                Err(_) => {
                    warn!("Some tasks did not complete in time");
                }
            }
        }

        info!("✅ Crawl completed successfully!");

        // Update final stats
        {
            let mut stats = self.stats.lock().await;
            stats.finished_at = Utc::now();
            stats.duration_secs = start_time.elapsed().as_secs_f64();
        }

        // Build the result
        let pages = self.pages.lock().await.clone();
        let graph = self.graph.lock().await.clone();
        let stats = self.stats.lock().await.clone();

        let total_links = graph.values().map(|v| v.len()).sum();

        self.print_statistics().await;

        Ok(CrawlResult {
            pages,
            graph,
            total_links,
            stats,
        })
    }

    async fn process_page(&self, page: &Page) -> Result<(Page, Vec<String>)> {
        debug!("📄 Crawling page: {}", page.url);

        // Check robots.txt before processing
        if self.config.respect_robots_txt {
            if !self.should_crawl_url(&page.url).await {
                return Ok((
                    Page::new(page.url.clone(), page.depth)
                        .with_status_code(403)
                        .mark_crawled(),
                    Vec::new(),
                ));
            }
        }

        // Make an HTTP request
        let response = self.client.get(&page.url).send().await?;
        let status = response.status();
        let status_code = status.as_u16();

        // Check for successful response
        if !status.is_success() {
            warn!(
                "⚠️  Failed to download page: {} (status: {})",
                page.url, status
            );
            return Ok((
                Page::new(page.url.clone(), page.depth)
                    .with_status_code(status_code)
                    .mark_crawled(),
                Vec::new(),
            ));
        }

        // Get content type
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();

        // Skip non-HTML content
        if !content_type.contains("text/html") {
            debug!("Skipping non-HTML content: {} ({})", page.url, content_type);
            return Ok((
                Page::new(page.url.clone(), page.depth)
                    .with_status_code(status_code)
                    .with_content_type(content_type)
                    .mark_crawled(),
                Vec::new(),
            ));
        }

        // Get the response text first
        let bytes = response.bytes().await?;
        let text = String::from_utf8_lossy(&bytes);
        let size = bytes.len();

        // Extract links without any async operations in between
        // NOTE: This is the key fix for the Send issue
        let (links, title) = self.extract_links_and_title(&text, &page.url)?;

        // Get delay for the domain if needed
        if let Ok(domain) = self.extract_domain(&page.url) {
            if let Some(delay) = self
                .robots_checker
                .get_crawl_delay(&domain, &self.config.user_agent)
                .await
            {
                // Use the larger of robots.txt delay and our configured delay
                let configured_delay = self.config.delay_between_requests;
                let actual_delay = if delay > configured_delay {
                    delay
                } else {
                    configured_delay
                };

                debug!(
                    "Sleeping for {}ms (robots.txt crawl-delay)",
                    actual_delay.as_millis()
                );
                tokio::time::sleep(actual_delay).await;
            } else {
                // Use our configured delay
                tokio::time::sleep(self.config.delay_between_requests).await;
            }
        }

        // Create the updated page with all information
        let processed_page = Page::new(page.url.clone(), page.depth)
            .with_links(links.clone())
            .with_status_code(status_code)
            .with_content_type(content_type)
            .with_size(size)
            .mark_crawled();

        if let Some(t) = title {
            Ok((processed_page.with_title(t), links))
        } else {
            Ok((processed_page, links))
        }
    }

    // New helper method to extract links without async calls
    // This ensures we don't have `Html` across an await point
    fn extract_links_and_title(
        &self,
        html_text: &str,
        base_url_str: &str,
    ) -> Result<(Vec<String>, Option<String>)> {
        // Parse HTML and extract links
        let document = Html::parse_document(html_text);

        // Extract page title
        let title = document
            .select(&Selector::parse("title").unwrap())
            .next()
            .and_then(|el| el.text().next())
            .map(|s| s.to_string());

        let base_url = Url::parse(base_url_str)?;
        let selector = Selector::parse("a[href]").unwrap();

        // Extract and validate links
        let mut links = Vec::new();

        for element in document.select(&selector) {
            if let Some(href) = element.value().attr("href") {
                // Convert relative URLs to absolute
                if let Ok(absolute_url) = base_url.join(href) {
                    // Only accept HTTP(S) links
                    if absolute_url.scheme() == "http" || absolute_url.scheme() == "https" {
                        // Normalize the URL to avoid duplicates
                        let normalized_url = self.normalize_url(&absolute_url);
                        links.push(normalized_url);
                    }
                }
            }
        }

        debug!("✨ Found {} valid links on {}", links.len(), base_url_str);
        Ok((links, title))
    }

    async fn should_crawl_url(&self, url: &str) -> bool {
        match self
            .robots_checker
            .is_allowed(url, &self.config.user_agent)
            .await
        {
            Ok(allowed) => {
                if !allowed {
                    info!("🚫 URL disallowed by robots.txt: {}", url);
                }
                allowed
            }
            Err(e) => {
                warn!("⚠️ Error checking robots.txt for {}: {}", url, e);
                true // Proceed if we can't check robots.txt
            }
        }
    }

    fn extract_domain(&self, url: &str) -> Result<String> {
        let parsed = Url::parse(url)?;
        Ok(parsed.host_str().unwrap_or("").to_string())
    }

    fn normalize_url(&self, url: &Url) -> String {
        let mut url = url.clone();

        // Remove fragments (anchors)
        url.set_fragment(None);

        // Remove query parameters if desired
        // url.set_query(None);

        // Convert to string and remove trailing slash if present
        let mut url_str = url.to_string();
        if url_str.ends_with('/') {
            url_str.pop();
        }

        url_str
    }

    async fn print_statistics(&self) {
        let visited = self.visited.lock().await;
        let graph = self.graph.lock().await;
        let stats = self.stats.lock().await;

        let total_links = graph.values().map(|v| v.len()).sum::<usize>();
        let avg_links_per_page = if !graph.is_empty() {
            total_links as f64 / graph.len() as f64
        } else {
            0.0
        };

        info!("📊 Final Statistics:");
        info!("   Pages crawled: {}", visited.len());
        info!("   Total links found: {}", total_links);
        info!("   Average links per page: {:.2}", avg_links_per_page);
        info!("   Crawl duration: {:.2}s", stats.duration_secs);
        info!("   Successful requests: {}", stats.success_count);
        info!("   Failed requests: {}", stats.error_count);

        // Find page with most outgoing links
        if let Some((url, links)) = graph.iter().max_by_key(|(_, links)| links.len()) {
            info!("   Most linked page: {} with {} links", url, links.len());
        }

        // Domain distribution
        let domain_counts = visited
            .iter()
            .filter_map(|url| self.extract_domain(url).ok())
            .fold(HashMap::new(), |mut acc, domain| {
                *acc.entry(domain).or_insert(0) += 1;
                acc
            });

        info!("   Top domains:");
        let mut domain_list: Vec<_> = domain_counts.iter().collect();
        domain_list.sort_by(|(_, a), (_, b)| b.cmp(a));
        for (domain, count) in domain_list.iter().take(5) {
            info!("     - {}: {} pages", domain, count);
        }
    }
}

impl Clone for Crawler {
    fn clone(&self) -> Self {
        Self {
            visited: Arc::clone(&self.visited),
            graph: Arc::clone(&self.graph),
            pages: Arc::clone(&self.pages),
            config: self.config.clone(),
            client: self.client.clone(),
            limiter: Arc::clone(&self.limiter),
            robots_checker: self.robots_checker.clone(),
            domain_counters: Arc::clone(&self.domain_counters),
            stats: Arc::clone(&self.stats),
        }
    }
}

