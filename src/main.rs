mod cache;
mod config;
mod image;
mod proxy;

use anyhow::{Context, Result};
use axum::{
    routing::get,
    Router,
};
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Config;
use crate::proxy::{health_handler, metrics_handler, proxy_handler, AppState};

#[derive(Parser, Debug)]
#[command(name = "akkoproxy")]
#[command(about = "A fast caching and optimization media proxy for Akkoma/Pleroma", long_about = None)]
#[command(version)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Upstream server URL (e.g., https://akkoma.example.com)
    #[arg(short, long, value_name = "URL")]
    upstream: Option<String>,

    /// Address to bind the server to (e.g., 0.0.0.0:3000)
    #[arg(short, long, value_name = "ADDR")]
    bind: Option<SocketAddr>,

    /// Enable AVIF conversion
    #[arg(long)]
    enable_avif: bool,

    /// Disable AVIF conversion
    #[arg(long, conflicts_with = "enable_avif")]
    disable_avif: bool,

    /// Enable WebP conversion
    #[arg(long)]
    enable_webp: bool,

    /// Disable WebP conversion
    #[arg(long, conflicts_with = "enable_webp")]
    disable_webp: bool,

    /// Preserve all headers from upstream when responding
    #[arg(long)]
    preserve_headers: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "akkoproxy=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Akkoproxy v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = load_config(&cli)?;
    
    info!("Configuration loaded:");
    info!("  Bind address: {}", config.server.bind);
    info!("  Upstream URL: {}", config.upstream.url);
    info!("  Cache max capacity: {}", config.cache.max_capacity);
    info!("  AVIF conversion: {}", config.image.enable_avif);
    info!("  WebP conversion: {}", config.image.enable_webp);
    info!("  Preserve upstream headers: {}", config.server.preserve_upstream_headers);

    // Create application state
    let state = AppState::new(config.clone());

    // Build router
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .fallback(proxy_handler)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind(&config.server.bind)
        .await
        .with_context(|| format!("Failed to bind to {}", config.server.bind))?;

    info!("Server listening on {}", config.server.bind);
    
    axum::serve(listener, app)
        .await
        .context("Server error")?;

    Ok(())
}

/// Load configuration with priority: env > cmdline options > config file
fn load_config(cli: &Cli) -> Result<Config> {
    // Priority 3 (lowest): Load from config file if it exists
    let mut config = if let Some(config_path) = &cli.config {
        // Use specified config file
        info!("Loading configuration from: {}", config_path.display());
        Config::from_file(config_path)?
    } else {
        // Try default config file path
        let config_path = PathBuf::from("config.toml");
        if config_path.exists() {
            info!("Loading configuration from: {}", config_path.display());
            Config::from_file(&config_path)?
        } else {
            // No config file, start with defaults (will need upstream from env or CLI)
            Config::with_upstream(String::new()) // Placeholder, will be overridden
        }
    };

    // Priority 2 (medium): Apply command-line options
    if let Some(upstream_url) = &cli.upstream {
        info!("Overriding upstream URL from command line: {}", upstream_url);
        config.upstream.url = upstream_url.clone();
    }
    
    if let Some(bind) = cli.bind {
        config.server.bind = bind;
    }

    if cli.enable_avif {
        config.image.enable_avif = true;
    } else if cli.disable_avif {
        config.image.enable_avif = false;
    }

    if cli.enable_webp {
        config.image.enable_webp = true;
    } else if cli.disable_webp {
        config.image.enable_webp = false;
    }

    if cli.preserve_headers {
        config.server.preserve_upstream_headers = true;
    }

    // Priority 1 (highest): Apply environment variables
    if let Ok(upstream_url) = std::env::var("UPSTREAM_URL") {
        info!("Overriding upstream URL from environment: {}", upstream_url);
        config.upstream.url = upstream_url;
    }
    
    if let Ok(bind_str) = std::env::var("BIND_ADDRESS") {
        if let Ok(bind) = bind_str.parse() {
            info!("Overriding bind address from environment: {}", bind);
            config.server.bind = bind;
        }
    }
    
    if let Ok(preserve) = std::env::var("PRESERVE_HEADERS") {
        if let Ok(value) = preserve.parse::<bool>() {
            info!("Overriding preserve_headers from environment: {}", value);
            config.server.preserve_upstream_headers = value;
        }
    }

    // Validate that we have an upstream URL
    if config.upstream.url.is_empty() {
        anyhow::bail!(
            "No upstream URL configured!\n\
            Use --upstream to specify upstream URL via command line,\n\
            set UPSTREAM_URL environment variable, or add it to config.toml file."
        )
    }

    config.validate()?;
    Ok(config)
}
