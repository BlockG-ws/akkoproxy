use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use anyhow::{Context, Result};

/// Application configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Server configuration
    #[serde(default)]
    pub server: ServerConfig,
    
    /// Upstream configuration
    pub upstream: UpstreamConfig,
    
    /// Cache configuration
    #[serde(default)]
    pub cache: CacheConfig,
    
    /// Image processing configuration
    #[serde(default)]
    pub image: ImageConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    /// Address to bind to
    #[serde(default = "default_bind_address")]
    pub bind: SocketAddr,
    
    /// Custom Via header value
    #[serde(default = "default_via_header")]
    pub via_header: String,
    
    /// Preserve all headers from upstream
    #[serde(default)]
    pub preserve_upstream_headers: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpstreamConfig {
    /// Upstream server URL (e.g., "https://akkoma.example.com")
    pub url: String,
    
    /// Timeout for upstream requests in seconds
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheConfig {
    /// Maximum number of cached items
    #[serde(default = "default_max_capacity")]
    pub max_capacity: u64,
    
    /// Time to live for cached items in seconds
    #[serde(default = "default_ttl")]
    pub ttl: u64,
    
    /// Maximum size of a cached item in bytes
    #[serde(default = "default_max_item_size")]
    pub max_item_size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageConfig {
    /// Enable AVIF conversion
    #[serde(default = "default_true")]
    pub enable_avif: bool,
    
    /// Enable WebP conversion
    #[serde(default = "default_true")]
    pub enable_webp: bool,
    
    /// JPEG quality for conversions (1-100)
    #[serde(default = "default_quality")]
    pub quality: u8,
    
    /// Maximum image dimensions for processing
    #[serde(default = "default_max_dimension")]
    pub max_dimension: u32,
}

// Default value functions
fn default_bind_address() -> SocketAddr {
    "0.0.0.0:3000".parse().unwrap()
}

fn default_via_header() -> String {
    format!("akkoproxy/{}", env!("CARGO_PKG_VERSION"))
}

fn default_timeout() -> u64 {
    30
}

fn default_max_capacity() -> u64 {
    10_000
}

fn default_ttl() -> u64 {
    3600 // 1 hour
}

fn default_max_item_size() -> u64 {
    10 * 1024 * 1024 // 10MB
}

fn default_true() -> bool {
    true
}

fn default_quality() -> u8 {
    85
}

fn default_max_dimension() -> u32 {
    4096
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind_address(),
            via_header: default_via_header(),
            preserve_upstream_headers: false,
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_capacity: default_max_capacity(),
            ttl: default_ttl(),
            max_item_size: default_max_item_size(),
        }
    }
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            enable_avif: default_true(),
            enable_webp: default_true(),
            quality: default_quality(),
            max_dimension: default_max_dimension(),
        }
    }
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .context("Failed to read configuration file")?;
        
        let config: Config = toml::from_str(&contents)
            .context("Failed to parse configuration file")?;
        
        config.validate()?;
        Ok(config)
    }
    
    /// Create a default configuration with a given upstream URL
    pub fn with_upstream(upstream_url: String) -> Self {
        Self {
            server: ServerConfig::default(),
            upstream: UpstreamConfig {
                url: upstream_url,
                timeout: default_timeout(),
            },
            cache: CacheConfig::default(),
            image: ImageConfig::default(),
        }
    }
    
    /// Validate configuration
    fn validate(&self) -> Result<()> {
        // Validate upstream URL
        url::Url::parse(&self.upstream.url)
            .context("Invalid upstream URL")?;
        
        // Validate quality
        if self.image.quality == 0 || self.image.quality > 100 {
            anyhow::bail!("Image quality must be between 1 and 100");
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::with_upstream("https://example.com".to_string());
        assert_eq!(config.upstream.url, "https://example.com");
        assert!(config.image.enable_avif);
        assert!(config.image.enable_webp);
    }
}
