use axum::http::HeaderMap;
use bytes::Bytes;
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

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
}

impl ResponseCache {
    /// Create a new response cache
    pub fn new(max_capacity: u64, ttl: Duration, _max_item_size: u64) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(ttl)
            .weigher(move |_key: &CacheKey, value: &Arc<CachedResponse>| {
                // Weight based on data size
                let size = value.data.len() as u32;
                std::cmp::max(1, size)
            })
            .initial_capacity(100)
            .build();
        
        Self { cache }
    }
    
    /// Get a cached response
    pub async fn get(&self, key: &CacheKey) -> Option<Arc<CachedResponse>> {
        self.cache.get(key).await
    }
    
    /// Store a response in the cache
    pub async fn put(&self, key: CacheKey, response: CachedResponse) {
        self.cache.insert(key, Arc::new(response)).await;
    }
    
    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entry_count: self.cache.entry_count(),
            weighted_size: self.cache.weighted_size(),
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entry_count: u64,
    pub weighted_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
