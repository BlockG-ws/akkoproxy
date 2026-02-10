use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;

/// Deserialize a size that can be either a number (bytes) or a human-readable string like "10M", "1G"
fn deserialize_size<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SizeValue {
        Numeric(u64),
        String(String),
    }

    match SizeValue::deserialize(deserializer)? {
        SizeValue::Numeric(n) => Ok(n),
        SizeValue::String(s) => {
            // Try to parse as a human-readable size using bytesize
            s.parse::<bytesize::ByteSize>()
                .map(|bs| bs.as_u64())
                .map_err(serde::de::Error::custom)
        }
    }
}

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
    #[serde(default = "default_true")]
    pub preserve_upstream_headers: bool,

    /// Enable Cloudflare Free plan compatibility mode
    /// When enabled, the proxy will look for a 'format' query parameter
    /// and use it to determine output format (avif/webp), then strip it
    /// from the upstream request
    #[serde(default)]
    pub behind_cloudflare_free: bool,

    /// Enable forwarding of X-Forwarded-* headers to upstream
    /// When disabled, X-Forwarded-* headers from clients are ignored
    #[serde(default)]
    pub forward_headers_enabled: bool,

    /// List of trusted proxy IP addresses or CIDR ranges
    /// Only requests from these IPs will have their X-Forwarded-* headers honored
    /// If empty, no headers will be forwarded (secure default)
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
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
    #[serde(default = "default_max_item_size", deserialize_with = "deserialize_size")]
    pub max_item_size: u64,

    /// Enable disk-based cache (default: false)
    #[serde(default)]
    pub disk_cache_enabled: bool,

    /// Path to disk cache directory (default: ./cache)
    #[serde(default = "default_disk_cache_path")]
    pub disk_cache_path: String,

    /// Maximum disk cache size in bytes (default: 1GB)
    #[serde(default = "default_disk_cache_max_size", deserialize_with = "deserialize_size")]
    pub disk_cache_max_size: u64,
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
    "0.0.0.0:3000"
        .parse()
        .expect("Failed to parse default bind address")
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

fn default_disk_cache_path() -> String {
    "./cache".to_string()
}

fn default_disk_cache_max_size() -> u64 {
    1024 * 1024 * 1024 // 1GB
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind_address(),
            via_header: default_via_header(),
            preserve_upstream_headers: true,
            behind_cloudflare_free: false,
            forward_headers_enabled: false,
            trusted_proxies: Vec::new(),
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_capacity: default_max_capacity(),
            ttl: default_ttl(),
            max_item_size: default_max_item_size(),
            disk_cache_enabled: false,
            disk_cache_path: default_disk_cache_path(),
            disk_cache_max_size: default_disk_cache_max_size(),
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
        let contents = fs::read_to_string(path).context("Failed to read configuration file")?;

        let config: Config =
            toml::from_str(&contents).context("Failed to parse configuration file")?;

        config.validate()?;
        Ok(config)
    }

    /// Create a default configuration with a given upstream URL
    #[cfg(test)]
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

    /// Create a default configuration with empty upstream (to be filled later)
    pub fn default_without_upstream() -> Self {
        Self {
            server: ServerConfig::default(),
            upstream: UpstreamConfig {
                url: String::new(),
                timeout: default_timeout(),
            },
            cache: CacheConfig::default(),
            image: ImageConfig::default(),
        }
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        // Validate upstream URL
        url::Url::parse(&self.upstream.url).context("Invalid upstream URL")?;

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

    #[test]
    fn test_parse_size_numeric() {
        let toml = r#"
            [upstream]
            url = "https://example.com"
            
            [cache]
            max_item_size = 10485760
            disk_cache_max_size = 1073741824
        "#;
        
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.cache.max_item_size, 10485760);
        assert_eq!(config.cache.disk_cache_max_size, 1073741824);
    }

    #[test]
    fn test_parse_size_human_readable() {
        let toml = r#"
            [upstream]
            url = "https://example.com"
            
            [cache]
            max_item_size = "10MiB"
            disk_cache_max_size = "1GiB"
        "#;
        
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.cache.max_item_size, 10 * 1024 * 1024);
        assert_eq!(config.cache.disk_cache_max_size, 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_various_formats() {
        let toml = r#"
            [upstream]
            url = "https://example.com"
            
            [cache]
            max_item_size = "5MB"
            disk_cache_max_size = "2GB"
        "#;
        
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.cache.max_item_size, 5 * 1000 * 1000);
        assert_eq!(config.cache.disk_cache_max_size, 2 * 1000 * 1000 * 1000);
    }

    #[test]
    fn test_parse_size_kilobytes() {
        let toml = r#"
            [upstream]
            url = "https://example.com"
            
            [cache]
            max_item_size = "512KiB"
            disk_cache_max_size = "100KB"
        "#;
        
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.cache.max_item_size, 512 * 1024);
        assert_eq!(config.cache.disk_cache_max_size, 100 * 1000);
    }
}
