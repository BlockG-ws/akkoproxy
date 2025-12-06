mod cache;
mod config;
mod image;
mod proxy;

use anyhow::{Context, Result};
use axum::{
    routing::get,
    Router,
};
use std::env;
use std::path::PathBuf;
use tower_http::trace::TraceLayer;
use tracing::{info, Level};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Config;
use crate::proxy::{health_handler, metrics_handler, proxy_handler, AppState};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "akkoma_media_proxy=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Akkoma Media Proxy v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = load_config()?;
    
    info!("Configuration loaded:");
    info!("  Bind address: {}", config.server.bind);
    info!("  Upstream URL: {}", config.upstream.url);
    info!("  Cache max capacity: {}", config.cache.max_capacity);
    info!("  AVIF conversion: {}", config.image.enable_avif);
    info!("  WebP conversion: {}", config.image.enable_webp);

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

/// Load configuration from file or environment
fn load_config() -> Result<Config> {
    // Check for config file path from environment or use default
    let config_path = env::var("CONFIG_PATH")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));

    // Try to load from file
    if config_path.exists() {
        info!("Loading configuration from: {}", config_path.display());
        return Config::from_file(&config_path);
    }

    // If no config file, try to get upstream URL from environment
    if let Ok(upstream_url) = env::var("UPSTREAM_URL") {
        info!("Using upstream URL from environment: {}", upstream_url);
        return Ok(Config::with_upstream(upstream_url));
    }

    // Generate example config and show error
    let example = Config::example();
    eprintln!("No configuration found!");
    eprintln!("Please create a config.toml file or set UPSTREAM_URL environment variable.");
    eprintln!("\nExample configuration:\n\n{}", example);
    
    anyhow::bail!("No configuration found")
}
