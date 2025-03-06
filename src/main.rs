use error_chain::error_chain;
use futures::future::join_all;
use reqwest::Client;
use scraper::{Html, Selector};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};
use url::Url;

// Custom error type to handle different errors
error_chain! {
    foreign_links {
        ReqError(reqwest::Error);
        UrlParseError(url::ParseError);
        IoError(std::io::Error);
        JoinError(tokio::task::JoinError);
    }
}

// Page struct representing a single webpage
#[derive(Debug, Clone)]
struct Page {
    url: String,
    links: Vec<String>,
    depth: u32,
}

// Configuration for crawler behavior
#[derive(Debug, Clone)]
struct CrawlerConfig {
    max_depth: u32,
    concurrent_tasks: usize,
    request_timeout: Duration,
    crawl_timeout: Duration,
    delay_between_requests: Duration,
    user_agent: String,
    respect_robots_txt: bool,
}

impl Default for CrawlerConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            concurrent_tasks: 4,
            request_timeout: Duration::from_secs(10),
            crawl_timeout: Duration::from_secs(60),
            delay_between_requests: Duration::from_millis(100),
            user_agent: "RustCrawler/1.0".to_string(),
            respect_robots_txt: true,
        }
    }
}

// Crawler struct - control center of the program
#[derive(Clone)]
struct Crawler {
    visited: Arc<Mutex<HashSet<String>>>,
    graph: Arc<Mutex<HashMap<String, Vec<String>>>>,
    config: CrawlerConfig,
    client: Client,
    limiter: Arc<Semaphore>,
}

impl Crawler {
    // Constructor with configurable parameters
    fn new(config: CrawlerConfig) -> Result<Self> {
        // Create a custom client with proper settings
        let client = Client::builder()
            .user_agent(&config.user_agent)
            .timeout(config.request_timeout)
            .build()?;

        // Store the concurrent_tasks value before moving config
        let concurrent_tasks = config.concurrent_tasks;

        Ok(Crawler {
            visited: Arc::new(Mutex::new(HashSet::new())),
            graph: Arc::new(Mutex::new(HashMap::new())),
            config,
            client,
            limiter: Arc::new(Semaphore::new(concurrent_tasks)),
        })
    }

    // Main crawl method implementing producer-consumer pattern
    async fn crawl(&self, start_url: &str) -> Result<()> {
        println!("\n🚀 Starting crawler at: {}", start_url);

        // Create a channel for communication between workers
        let (tx, mut rx) = mpsc::channel(100);

        // Create the starting point
        let start = Page {
            url: start_url.to_string(),
            links: Vec::new(),
            depth: 0,
        };

        // Mark the start URL as visited right away
        {
            let mut visited = self.visited.lock().unwrap();
            visited.insert(start_url.to_string());
        }

        tx.send(start).await.unwrap();

        // Set up worker tasks to process URLs
        let mut handles = vec![];

        // Main processing loop
        loop {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                // We received a page to process
                Ok(Some(page)) => {
                    // Skip if we've reached max depth
                    if page.depth >= self.config.max_depth {
                        println!(
                            "🛑 Reached max depth ({}) for {}",
                            self.config.max_depth, page.url
                        );
                        continue;
                    }

                    // Clone what we need for the task
                    let crawler = self.clone();
                    let tx = tx.clone();

                    // Spawn a new task to process this page
                    let handle = tokio::spawn(async move {
                        // Acquire a permit from the semaphore to limit concurrency
                        let _permit = crawler.limiter.acquire().await.unwrap();

                        println!(
                            "\n📊 Processing {} at depth {}/{}",
                            page.url, page.depth, crawler.config.max_depth
                        );

                        // Process the page and handle any links found
                        match crawler.process_page(&page.url).await {
                            Ok(links) => {
                                // Update the graph with new links
                                {
                                    let mut graph = crawler.graph.lock().unwrap();
                                    graph.insert(page.url.clone(), links.clone());
                                }

                                // Queue up new pages for processing
                                for link in links {
                                    // Check if we've already visited this URL
                                    let should_queue = {
                                        let mut visited = crawler.visited.lock().unwrap();
                                        if !visited.contains(&link) {
                                            // Mark as visited preemptively
                                            visited.insert(link.clone());
                                            true
                                        } else {
                                            false
                                        }
                                    };

                                    if should_queue {
                                        let new_page = Page {
                                            url: link.clone(),
                                            links: Vec::new(),
                                            depth: page.depth + 1,
                                        };

                                        println!(
                                            "➡️  Queueing {} (at depth {})",
                                            link, new_page.depth
                                        );

                                        if tx.send(new_page).await.is_err() {
                                            println!("❌ Channel closed, exiting");
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(e) => println!("⚠️  Error processing {}: {}", page.url, e),
                        }

                        // Delay to be polite to servers
                        tokio::time::sleep(crawler.config.delay_between_requests).await;

                        Ok::<(), Error>(())
                    });

                    handles.push(handle);

                    // Clean up completed handles
                    handles.retain(|h| !h.is_finished());
                }
                // Channel is empty for now, but might get more messages later
                Ok(None) => {
                    // If all tasks are done and channel is empty, we're finished
                    if handles.is_empty() {
                        break;
                    }

                    // Otherwise wait for tasks to complete
                    if let Some(handle) = join_all(handles.iter_mut()).await.pop() {
                        if let Err(e) = handle {
                            println!("⚠️  A worker task failed: {}", e);
                        }
                    }
                }
                // Timeout reached while waiting for new messages
                Err(_) => {
                    // Check if any tasks are still running
                    if handles.is_empty() {
                        break;
                    }

                    // Clean up completed handles again
                    handles.retain(|h| !h.is_finished());
                }
            }
        }

        // Set a timeout for the entire crawl operation
        match tokio::time::timeout(self.config.crawl_timeout, join_all(handles)).await {
            Ok(results) => {
                for result in results {
                    if let Err(e) = result {
                        println!("⚠️  A worker task failed: {}", e);
                    }
                }
                println!("\n✅ Crawl completed successfully!");
            }
            Err(_) => println!(
                "\n⚠️  Crawl timed out after {} seconds!",
                self.config.crawl_timeout.as_secs()
            ),
        }

        // Print final statistics
        self.print_statistics().await;

        Ok(())
    }

    // Process a single page - fetch and extract links
    async fn process_page(&self, url: &str) -> Result<Vec<String>> {
        println!(
            "\n📄 Crawling page: {} (Total visited: {})",
            url,
            self.visited.lock().unwrap().len()
        );

        // Make an HTTP request with our configured client
        let response = self.client.get(url).send().await?;

        // Check for successful response
        if !response.status().is_success() {
            println!(
                "⚠️  Failed to download page: {} (status: {})",
                url,
                response.status()
            );
            return Ok(Vec::new());
        }

        println!(
            "⬇️  Downloaded page: {} (status: {})",
            url,
            response.status()
        );

        let text = response.text().await?;

        // Parse HTML and extract links
        let document = Html::parse_document(&text);
        let selector = Selector::parse("a[href]").unwrap();
        let base_url = Url::parse(url)?;

        // Extract and validate links
        let mut links = Vec::new();
        println!("🔍 Found links:");

        for element in document.select(&selector) {
            if let Some(href) = element.value().attr("href") {
                // Convert relative URLs to absolute
                if let Ok(absolute_url) = base_url.join(href) {
                    // Only accept HTTP(S) links
                    if absolute_url.scheme() == "http" || absolute_url.scheme() == "https" {
                        // Normalize the URL to avoid duplicates
                        let normalized_url = self.normalize_url(&absolute_url);

                        println!("  → {}", normalized_url);
                        links.push(normalized_url);
                    }
                }
            }
        }

        println!("✨ Found {} valid links on this page", links.len());
        Ok(links)
    }

    // Normalize URLs to prevent duplicates (removing trailing slashes, fragments, etc.)
    fn normalize_url(&self, url: &Url) -> String {
        let mut url = url.clone();

        // Remove fragments (anchors)
        url.set_fragment(None);

        // Remove query parameters if needed
        // url.set_query(None);

        // Convert to string and remove trailing slash if present
        let mut url_str = url.to_string();
        if url_str.ends_with('/') {
            url_str.pop();
        }

        url_str
    }

    // Print statistics about the crawl
    async fn print_statistics(&self) {
        let visited = self.visited.lock().unwrap();
        let graph = self.graph.lock().unwrap();

        let total_links = graph.values().map(|v| v.len()).sum::<usize>();
        let avg_links_per_page = if !graph.is_empty() {
            total_links as f64 / graph.len() as f64
        } else {
            0.0
        };

        println!("\n📊 Final Statistics:");
        println!("   Pages crawled: {}", visited.len());
        println!("   Total links found: {}", total_links);
        println!("   Average links per page: {:.2}", avg_links_per_page);

        // Find page with most outgoing links
        if let Some((url, links)) = graph.iter().max_by_key(|(_, links)| links.len()) {
            println!("   Most linked page: {} with {} links", url, links.len());
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::init();

    // Configure the crawler
    let config = CrawlerConfig {
        max_depth: 2,
        concurrent_tasks: 8,
        request_timeout: Duration::from_secs(10),
        crawl_timeout: Duration::from_secs(120),
        delay_between_requests: Duration::from_millis(100),
        user_agent: "RustCrawler/1.0 (https://example.com/bot)".to_string(),
        respect_robots_txt: true,
    };

    println!("\n🚀 Starting crawler with configuration:");
    println!("   Max depth: {}", config.max_depth);
    println!("   Concurrent tasks: {}", config.concurrent_tasks);
    println!(
        "   Request timeout: {} seconds",
        config.request_timeout.as_secs()
    );
    println!(
        "   Crawl timeout: {} seconds",
        config.crawl_timeout.as_secs()
    );

    let crawler = Crawler::new(config)?;
    crawler.crawl("https://www.rust-lang.org").await?;

    Ok(())
}

