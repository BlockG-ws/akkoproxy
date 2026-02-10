use axum::http::HeaderMap;
use bytes::Bytes;
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use crate::disk_cache::DiskCache;

/// Cache key for storing responses
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct CacheKey {
    pub path: String,
    pub format: String,
}

impl CacheKey {
    pub fn new(path: String, format: String) -> Self {
        Self { path, format }
    }
}

/// Cached response data
#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub data: Bytes,
    pub content_type: String,
    pub upstream_headers: Option<HeaderMap>,
}

/// Response cache manager
#[derive(Clone)]
pub struct ResponseCache {
    cache: Cache<CacheKey, Arc<CachedResponse>>,
    disk_cache: Option<DiskCache>,
    max_item_size: u64,
}

impl ResponseCache {
    /// Create a new response cache
    pub fn new(max_capacity: u64, ttl: Duration, max_item_size: u64) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(ttl)
            .initial_capacity(100)
            .weigher(|_key, value: &Arc<CachedResponse>| -> u32 {
                // Use the size of the cached data as the weight
                // Cap at u32::MAX to avoid overflow
                value.data.len().min(u32::MAX as usize) as u32
            })
            .build();

        Self { 
            cache,
            disk_cache: None,
            max_item_size,
        }
    }

    /// Create a new response cache with disk cache support
    pub fn new_with_disk_cache(
        max_capacity: u64, 
        ttl: Duration, 
        max_item_size: u64,
        disk_cache: DiskCache
    ) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(ttl)
            .initial_capacity(100)
            .weigher(|_key, value: &Arc<CachedResponse>| -> u32 {
                // Use the size of the cached data as the weight
                // Cap at u32::MAX to avoid overflow
                value.data.len().min(u32::MAX as usize) as u32
            })
            .build();

        Self { 
            cache,
            disk_cache: Some(disk_cache),
            max_item_size,
        }
    }

    /// Get a cached response
    pub async fn get(&self, key: &CacheKey) -> Option<Arc<CachedResponse>> {
        // First, check memory cache
        if let Some(cached) = self.cache.get(key).await {
            return Some(cached);
        }

        // If not in memory and disk cache is enabled, check disk
        if let Some(disk_cache) = &self.disk_cache {
            if let Some((data, content_type)) = disk_cache.get(key).await {
                // Check if the item size exceeds the max_item_size limit
                let item_size = data.len() as u64;
                if item_size > self.max_item_size {
                    warn!(
                        "Disk cache item size ({} bytes) exceeds max_item_size ({} bytes), not promoting to memory",
                        item_size, self.max_item_size
                    );
                    // Still return the data from disk, just don't promote to memory
                    let response = CachedResponse {
                        data,
                        content_type,
                        upstream_headers: None,
                    };
                    return Some(Arc::new(response));
                }

                // Found in disk cache, promote to memory cache
                let response = CachedResponse {
                    data,
                    content_type,
                    upstream_headers: None, // Note: disk cache doesn't store headers
                };
                let arc_response = Arc::new(response);
                self.cache.insert(key.clone(), arc_response.clone()).await;
                return Some(arc_response);
            }
        }

        None
    }

    /// Store a response in the cache
    pub async fn put(&self, key: CacheKey, response: CachedResponse) {
        // Check if the item size exceeds the max_item_size limit
        let item_size = response.data.len() as u64;
        if item_size > self.max_item_size {
            warn!(
                "Item size ({} bytes) exceeds max_item_size ({} bytes), skipping cache",
                item_size, self.max_item_size
            );
            return;
        }

        // Store in memory cache
        self.cache.insert(key.clone(), Arc::new(response.clone())).await;

        // If disk cache is enabled, store there too
        if let Some(disk_cache) = &self.disk_cache {
            // Store to disk cache and log any errors
            if let Err(e) = disk_cache.put(&key, response.data, response.content_type).await {
                warn!("Failed to write to disk cache: {}", e);
            }
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entry_count: self.cache.entry_count(),
            weighted_size: self.cache.weighted_size(),
            disk_cache_enabled: self.disk_cache.is_some(),
            disk_stats: self.disk_cache.as_ref().map(|dc| dc.stats()),
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entry_count: u64,
    pub weighted_size: u64,
    pub disk_cache_enabled: bool,
    pub disk_stats: Option<crate::disk_cache::DiskCacheStats>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderName, HeaderValue};

    #[tokio::test]
    async fn test_cache_put_and_get() {
        let cache = ResponseCache::new(100, Duration::from_secs(60), 1024 * 1024);

        let key = CacheKey::new("/media/test.jpg".to_string(), "avif".to_string());
        let response = CachedResponse {
            data: Bytes::from("test data"),
            content_type: "image/avif".to_string(),
            upstream_headers: None,
        };

        cache.put(key.clone(), response.clone()).await;

        let cached = cache.get(&key).await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().content_type, "image/avif");
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = ResponseCache::new(100, Duration::from_secs(60), 1024 * 1024);

        let key = CacheKey::new("/media/nonexistent.jpg".to_string(), "webp".to_string());
        let cached = cache.get(&key).await;

        assert!(cached.is_none());
    }

    #[tokio::test]
    async fn test_cache_with_upstream_headers() {
        let cache = ResponseCache::new(100, Duration::from_secs(60), 1024 * 1024);

        // Create headers to cache
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-custom-header"),
            HeaderValue::from_static("test-value"),
        );

        let key = CacheKey::new("/media/test.jpg".to_string(), "avif".to_string());
        let response = CachedResponse {
            data: Bytes::from("test data"),
            content_type: "image/avif".to_string(),
            upstream_headers: Some(headers.clone()),
        };

        cache.put(key.clone(), response.clone()).await;

        let cached = cache.get(&key).await;
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert_eq!(cached.content_type, "image/avif");
        assert!(cached.upstream_headers.is_some());

        let cached_headers = cached.upstream_headers.as_ref().unwrap();
        assert_eq!(cached_headers.get("x-custom-header").unwrap(), "test-value");
    }

    #[tokio::test]
    async fn test_cache_ttl() {
        // Create cache with 1 second TTL
        let cache = ResponseCache::new(100, Duration::from_secs(1), 1024 * 1024);

        let key = CacheKey::new("/media/test.jpg".to_string(), "avif".to_string());
        let response = CachedResponse {
            data: Bytes::from("test data"),
            content_type: "image/avif".to_string(),
            upstream_headers: None,
        };

        cache.put(key.clone(), response.clone()).await;

        // Should be in cache immediately
        let cached = cache.get(&key).await;
        assert!(cached.is_some());

        // Wait for TTL to expire
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Should be gone after TTL
        let cached = cache.get(&key).await;
        assert!(cached.is_none());
    }
}
