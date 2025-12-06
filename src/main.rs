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

/// Load configuration from file or environment, with CLI overrides
fn load_config(cli: &Cli) -> Result<Config> {
    // Start with base config from file or upstream URL
    let mut config = if let Some(config_path) = &cli.config {
        // Use specified config file
        info!("Loading configuration from: {}", config_path.display());
        Config::from_file(config_path)?
    } else if let Some(upstream_url) = &cli.upstream {
        // Use upstream from CLI
        info!("Using upstream URL from command line: {}", upstream_url);
        Config::with_upstream(upstream_url.clone())
    } else {
        // Try environment variables or default config file
        let config_path = std::env::var("CONFIG_PATH")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("config.toml"));

        if config_path.exists() {
            info!("Loading configuration from: {}", config_path.display());
            Config::from_file(&config_path)?
        } else if let Ok(upstream_url) = std::env::var("UPSTREAM_URL") {
            info!("Using upstream URL from environment: {}", upstream_url);
            Config::with_upstream(upstream_url)
        } else {
            anyhow::bail!(
                "No configuration found!\n\
                Use --config to specify a config file, or --upstream to provide upstream URL.\n\
                Alternatively, set UPSTREAM_URL environment variable or create config.toml file."
            )
        }
    };

    // Apply CLI overrides
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

    Ok(config)
}
