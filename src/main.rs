use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use error_chain::error_chain;
use scraper::{Html, Selector};
use url::Url;
use futures::future::join_all;  // Add this import


// error chain macro, custom error type to handle a variety of different errors
error_chain! {
    foreign_links {
        ReqError(reqwest::Error);
        UrlParseError(url::ParseError);
        IoError(std::io::Error);
        JoinError(tokio::task::JoinError);
    }
}

// represents a single webpage
#[derive(Debug, Clone)]
struct Page {
    url: String,
    links: Vec<String>,
    depth: u32,
}

// control center of the program 
// arc mutex allows for the concurrency
// arc = ensure data isnt accidentally deleted 
// mutex = make sure only one part of our program can modify data at a time
#[derive(Clone)]
struct Crawler {
    visited: Arc<Mutex<HashSet<String>>>,
    graph: Arc<Mutex<HashMap<String, Vec<String>>>>,
    max_depth: u32,
    concurrent_tasks: usize,
}

// constructor function - intializing tools
// empty hashset for tracking visited pages
// empty hashmap for building out graph (connections between visted places)
// paramaeters - how deep to explore
impl Crawler {
    fn new(max_depth: u32, concurrent_tasks: usize) -> Self {
        Crawler {
            visited: Arc::new(Mutex::new(HashSet::new())),
            graph: Arc::new(Mutex::new(HashMap::new())),
            max_depth,
            concurrent_tasks,
        }
    }
    
    // implement producer-consumer pattern
    // tx (transmitter) - put new URLs we find (producer)
    // rx (receiver) - worker threads pick up URLs to process (consumers)
    // parameter 100 - can hold 100 links
    async fn crawl(&self, start_url: &str) -> Result<()> {
        println!("\n🚀 Starting crawler at: {}", start_url);
        
        let (tx, rx) = mpsc::channel(100);
        let rx = Arc::new(Mutex::new(rx));

        // create the starting point, this is our root page of depth 0
        let start = Page {
            url: start_url.to_string(),
            links: Vec::new(),
            depth: 0,
        };
        tx.send(start).await.unwrap();

        // setting up worker thrads, concurrent_tasks determins number of workers
        // each worker has: 
        // - copy of crawler 
        // - transmitter (to report new urls)
        // - access to the shared receiver (get new assignments)
        let mut handles = vec![];
        for worker_id in 0..self.concurrent_tasks {
            let crawler = self.clone();
            let tx = tx.clone();
            let rx = Arc::clone(&rx);
            
            // locks the receiver, tries to get a new page to process, 
            // if no new pages then print goodbye and exit
            // match statement works similarly to if/else, but accounts for all cases
            let handle = tokio::spawn(async move {
                loop {
                    let page = {
                        let mut rx = rx.lock().unwrap();
                        match rx.try_recv() {
                            Ok(page) => page,
                            Err(_) => {
                                println!("👋 Worker {} exiting - no more pages to process", worker_id);
                                break;
                            }
                        }
                    };
                    
                    // depth check happens here - "bounded depth traversal"
                    if page.depth >= crawler.max_depth {
                        println!("🛑 Worker {} - Reached max depth ({}) for {}", 
                            worker_id, crawler.max_depth, page.url);
                        continue;
                    }

                    println!("\n📊 Worker {} processing {} at depth {}/{}", 
                        worker_id, page.url, page.depth, crawler.max_depth);
                    
                    // the heart of the crawler
                    // calling process_page to fetch and analyze a page
                    // if successful
                    // - updates the graph with new links
                    // - for each link
                    // -- Create new page with increased depth
                    // -- sends it to the channel for other workers to process
                    // - errors are logges and crawler continues
                    // the use of locks (lock().unwrap() is crucial
                    // - prevents race condition
                    match crawler.process_page(&page.url).await {
                        Ok(links) => {
                                {
                                    let mut graph = crawler.graph.lock().unwrap();
                                    graph.insert(page.url.clone(), links.clone());   
                                }

                            for link in links {
                                let new_page = Page {
                                    url: link.clone(),
                                    links: Vec::new(),
                                    depth: page.depth + 1,
                                };
                                println!("➡️  Worker {} queueing {} (at depth {})", 
                                    worker_id, link, new_page.depth);
                                if tx.send(new_page).await.is_err() {
                                    println!("❌ Worker {} - Channel closed, exiting", worker_id);
                                    break;
                                }
                            }
                        }
                        Err(e) => println!("⚠️  Error processing {}: {}", page.url, e),
                    }
                }
                Ok::<(), Error>(())
            });
            handles.push(handle);
        }

        // Drop the original sender
        // shutdown of the crawler - stating "no new tasks will be created" 
        drop(tx);

        // Set a timeout for the entire crawl operation 
        let timeout = tokio::time::Duration::from_secs(60);  // 1 minute timeout
        // join_all(handles) is waiting for all workers to return, to ensure no lost progress
        match tokio::time::timeout(timeout, join_all(handles)).await {
            Ok(results) => {
                for result in results {
                    result??;
                }
                println!("\n✅ Crawl completed successfully!");
            }
            Err(_) => println!("\n⚠️  Crawl timed out after {} seconds!", timeout.as_secs()),
        }

        // Print final statistics
        let visited = self.visited.lock().unwrap();
        let graph = self.graph.lock().unwrap();
        println!("\n📊 Final Statistics:");
        println!("   Pages crawled: {}", visited.len());
        println!("   Total links found: {}", graph.values().map(|v| v.len()).sum::<usize>());

        Ok(())
    }

    // before requesting, make sure we have not seen the URL already
    // block seperated to release the lock ASAP - "minimal critical section"
    async fn process_page(&self, url: &str) -> Result<Vec<String>> {
        {
            let visited = self.visited.lock().unwrap();
            if visited.contains(url) {
                println!("🔄 Already visited {}, skipping", url);
                return Ok(Vec::new());
            }
        }

        // after confirming URL as new, mark as visited
        // the 100 seconds delay is to prevent overwhelming web servers
        // - Robots Exclusion Protocol (robots.txt)
        {
            let mut visited = self.visited.lock().unwrap();
            visited.insert(url.to_string());
        }

        // Add a small delay to be polite to the server
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        println!("\n📄 Crawling page: {} (Total visited: {})", 
                url, 
                self.visited.lock().unwrap().len());
        
        // make an HTTP request
        // parse the HTML document
        // use CSS selectors to find links 
        // await? is combining two modern Rust features
        // - async/await (for handling operations without blocking)
        // - the ? operator (for elegant error handling)
        let response = reqwest::get(url).await?;
        println!("⬇️  Downloaded page: {} (status: {})", url, response.status());
        let text = response.text().await?;
        
        let document = Html::parse_document(&text);
        let selector = Selector::parse("a[href]").unwrap();
        let base_url = Url::parse(url)?;
        
        // extracting and validating links
        // - convert relative URLs to absolute ones (using base URL as context)
        // - only accept HTTP(S) links (ignore mailto:links)
        // - handle malformed URLs gracefully
        let mut links = Vec::new();
        println!("🔍 Found links:");
        for element in document.select(&selector) {
            if let Some(href) = element.value().attr("href") {
                if let Ok(absolute_url) = base_url.join(href) {
                    if absolute_url.scheme() == "http" || absolute_url.scheme() == "https" {
                        println!("  → {}", absolute_url);
                        links.push(absolute_url.to_string());
                    }
                }
            }
        }
        println!("✨ Found {} links on this page", links.len());
        Ok(links)
    }
}

// telling Rust to set up an asynch runtime
#[tokio::main]
async fn main() -> Result<()> {
    // sets up logging 
    env_logger::init();
    
    let max_depth = 2;  // Reduced depth for clearer output
    let concurrent_tasks = 4;
    let crawler = Crawler::new(max_depth, concurrent_tasks);
    println!("\n🚀 Starting crawler:");
    println!("   Max depth: {}", max_depth);
    println!("   Concurrent tasks: {}", concurrent_tasks);
    
    crawler.crawl("https://www.rust-lang.org").await?;
    
    // signifies completion, () is unit type, like returning void, but explicit
    Ok(())
}
