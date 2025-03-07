// src/main.rs
mod config;
mod crawler;
mod error;
mod page;
mod robots;
mod storage;
mod visualization;

use clap::Parser;
use config::CrawlerConfig;
use crawler::Crawler;
use error::Result;
use log::{info, LevelFilter};

#[derive(Parser)]
#[command(
    author,
    version,
    about = "A high-performance web crawler written in Rust"
)]
struct Args {
    /// URL to start crawling from
    #[arg(short, long)]
    url: String,

    /// Maximum depth to crawl
    #[arg(short, long, default_value = "2")]
    depth: u32,

    /// Number of concurrent crawling tasks
    #[arg(short = 'n', long, default_value = "8")] // Changed from 'c' to 'n'
    concurrency: usize,

    /// Path to configuration file
    #[arg(short = 'f', long)] // Changed from 'c' to 'f'
    config_file: Option<String>,

    /// Whether to respect robots.txt
    #[arg(long, default_value = "true")]
    respect_robots: bool,

    /// Delay between requests in milliseconds
    #[arg(long, default_value = "100")]
    delay: u64,

    /// Custom user agent
    #[arg(long)]
    user_agent: Option<String>,

    /// Output file for crawl results
    #[arg(short, long)]
    output: Option<String>,

    /// Export graph in DOT format
    #[arg(long)]
    dot_output: Option<String>,

    /// Export interactive HTML visualization
    #[arg(long)]
    html_output: Option<String>,

    /// Generate example configuration file
    #[arg(long)]
    generate_config: Option<String>,

    /// Verbosity level (0-3)
    #[arg(short, long, default_value = "1")]
    verbose: u8,
}
#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = Args::parse();

    // Initialize logger with appropriate level
    let log_level = match args.verbose {
        0 => LevelFilter::Error,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    env_logger::Builder::new()
        .filter_level(log_level)
        .format_timestamp_millis()
        .init();

    // Check if we need to generate a config file
    if let Some(config_path) = args.generate_config {
        config::create_example_config(&config_path)?;
        info!("Example configuration file created at: {}", config_path);
        return Ok(());
    }

    // Load configuration (from file or defaults)
    let mut config = if let Some(config_file) = args.config_file {
        config::load_from_file(&config_file)?
    } else {
        CrawlerConfig::default()
    };

    // Override config with command line arguments
    config.max_depth = args.depth;
    config.concurrent_tasks = args.concurrency;
    config.respect_robots_txt = args.respect_robots;
    config.delay_between_requests = std::time::Duration::from_millis(args.delay);

    if let Some(user_agent) = args.user_agent {
        config.user_agent = user_agent;
    }

    // Display configuration
    info!("Starting crawler with configuration:");
    info!("   URL: {}", args.url);
    info!("   Max depth: {}", config.max_depth);
    info!("   Concurrent tasks: {}", config.concurrent_tasks);
    info!(
        "   Request timeout: {} seconds",
        config.request_timeout.as_secs()
    );
    info!("   Respect robots.txt: {}", config.respect_robots_txt);

    // Initialize the crawler
    let crawler = Crawler::new(config)?;

    // Start crawling
    let result = crawler.crawl(&args.url).await?;

    info!("Crawl completed: {} pages processed", result.pages.len());

    // Save results if output file specified
    if let Some(output_file) = args.output {
        storage::save_results(&result, &output_file)?;
        info!("Results saved to: {}", output_file);
    }

    // Generate visualizations if requested
    if let Some(dot_path) = args.dot_output {
        let mut visualizer = visualization::GraphVisualizer::new();
        visualizer.build_from_crawler_graph(&result.graph);
        visualizer.export_dot(&dot_path)?;
        info!("Graph visualization exported to DOT format: {}", dot_path);
    }

    if let Some(html_path) = args.html_output {
        let mut visualizer = visualization::GraphVisualizer::new();
        visualizer.build_from_crawler_graph(&result.graph);
        visualizer.export_html(&html_path)?;
        info!("Interactive visualization exported to HTML: {}", html_path);
    }

    Ok(())
}

