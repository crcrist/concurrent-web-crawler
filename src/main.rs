use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use error_chain::error_chain;  // Import the macro properly

// Define our error chain
error_chain! {
    foreign_links {
        ReqError(reqwest::Error);
        UrlParseError(url::ParseError);
        IoError(std::io::Error);
        JoinError(tokio::task::JoinError);
    }
}

#[derive(Debug, Clone)]
struct Page {
    url: String,
    links: Vec<String>,
    depth: u32,
}

#[derive(Clone)]
struct Crawler {
    visited: Arc<Mutex<HashSet<String>>>,
    graph: Arc<Mutex<HashMap<String, Vec<String>>>>,
    max_depth: u32,
    concurrent_tasks: usize,
}

impl Crawler {
    fn new(max_depth: u32, concurrent_tasks: usize) -> Self {
        Crawler {
            visited: Arc::new(Mutex::new(HashSet::new())),
            graph: Arc::new(Mutex::new(HashMap::new())),
            max_depth,
            concurrent_tasks,
        }
    }

    async fn crawl(&self, start_url: &str) -> Result<()> {
        // Create a channel for passing pages between tasks
        let (tx, rx) = mpsc::channel(100);
        let rx = Arc::new(Mutex::new(rx));  // Share receiver between tasks

        // Send initial URL
        let start = Page {
            url: start_url.to_string(),
            links: Vec::new(),
            depth: 0,
        };
        tx.send(start).await.unwrap();

        // Spawn worker tasks
        let mut handles = vec![];
        for _ in 0..self.concurrent_tasks {
            let crawler = self.clone();
            let tx = tx.clone();
            let rx = Arc::clone(&rx);

            let handle = tokio::spawn(async move {
                loop {
                    // Get the next page to process
                    let page = {
                        let mut rx = rx.lock().unwrap();
                        match rx.try_recv() {
                            Ok(page) => page,
                            Err(_) => break,  // Channel is empty or closed
                        }
                    };

                    if page.depth >= crawler.max_depth {
                        continue;
                    }

                    // Process the page and get its links
                    if let Ok(links) = crawler.process_page(&page.url).await {
                        // Update the graph with new links
                        {
                            let mut graph = crawler.graph.lock().unwrap();
                            graph.insert(page.url.clone(), links.clone());
                        }

                        // Queue new pages
                        for link in links {
                            let new_page = Page {
                                url: link,
                                links: Vec::new(),
                                depth: page.depth + 1,
                            };
                            if tx.send(new_page).await.is_err() {
                                break;  // Channel closed
                            }
                        }
                    }
                }
                Ok::<(), Error>(())
            });
            handles.push(handle);
        }

        // Drop the original sender so the channel can close
        drop(tx);

        // Wait for all tasks to complete
        for handle in handles {
            handle.await??;
        }

        Ok(())
    }

    async fn process_page(&self, url: &str) -> Result<Vec<String>> {
        // Check if already visited
        {
            let visited = self.visited.lock().unwrap();
            if visited.contains(url) {
                return Ok(Vec::new());
            }
        }

        // Mark as visited
        {
            let mut visited = self.visited.lock().unwrap();
            visited.insert(url.to_string());
        }

        // Fetch and parse the page
        let response = reqwest::get(url).await?;
        let text = response.text().await?;

        // For now, return empty vector - we'll implement link extraction next
        Ok(Vec::new())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    
    let crawler = Crawler::new(3, 4);
    crawler.crawl("https://example.com").await?;
    
    Ok(())
}
