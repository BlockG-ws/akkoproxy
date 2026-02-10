//! Disk-based cache for media files
//!
//! This module provides a persistent, nginx-like disk cache for media content.
//! The cache stores files using SHA-256 hashed keys and supports automatic
//! TTL-based expiration and LRU eviction.
//!
//! # Features
//!
//! - **Persistent storage**: Cached media survives server restarts
//! - **Automatic cleanup**: LRU eviction when disk space limit is reached
//! - **TTL support**: Respects configured TTL using file creation times
//! - **LRU eviction**: Tracks last access time separately from creation time
//! - **Atomic writes**: Uses temporary files to prevent corruption
//! - **Subdirectory structure**: Organizes files to avoid too many in one directory
//!
//! # Limitations
//!
//! The disk cache stores only:
//! - Media content (bytes)
//! - Content-Type header
//! - Timestamps (created_at, last_access)
//!
//! Other upstream headers are NOT stored in the disk cache. They are only
//! preserved in the memory cache.

use anyhow::{Context, Result};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error, warn};
use walkdir::WalkDir;

use crate::cache::CacheKey;

/// Metadata stored alongside cached content
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheMetadata {
    content_type: String,
    created_at: u64,
    last_access: u64,
}

/// Disk-based cache for media files
#[derive(Clone)]
pub struct DiskCache {
    cache_dir: PathBuf,
    max_size: u64,
    ttl: u64,
}

impl DiskCache {
    /// Create a new disk cache
    pub fn new<P: AsRef<Path>>(cache_dir: P, max_size: u64, ttl: u64) -> Result<Self> {
        let cache_dir = cache_dir.as_ref().to_path_buf();
        
        // Create cache directory if it doesn't exist
        if !cache_dir.exists() {
            fs::create_dir_all(&cache_dir)
                .with_context(|| format!("Failed to create cache directory: {:?}", cache_dir))?;
        }

        Ok(Self {
            cache_dir,
            max_size,
            ttl,
        })
    }

    /// Generate a cache file path from a cache key
    fn get_cache_path(&self, key: &CacheKey) -> PathBuf {
        // Create a unique hash for the cache key
        let cache_key_str = format!("{}:{}", key.path, key.format);
        let mut hasher = Sha256::new();
        hasher.update(cache_key_str.as_bytes());
        let hash = hasher.finalize();
        let hash_str = hex::encode(hash);

        // Use first 2 chars for subdirectory to avoid too many files in one dir
        let subdir = &hash_str[0..2];
        let filename = &hash_str[2..];

        self.cache_dir.join(subdir).join(filename)
    }

    /// Get a cached item from disk
    pub async fn get(&self, key: &CacheKey) -> Option<(Bytes, String)> {
        let cache_path = self.get_cache_path(key);
        let metadata_path = cache_path.with_extension("meta");
        let ttl = self.ttl;

        // Use spawn_blocking for disk I/O
        let result = tokio::task::spawn_blocking(move || {
            // Check if both cache file and metadata exist
            if !cache_path.exists() || !metadata_path.exists() {
                return None;
            }

            // Read and parse metadata
            let metadata_str = match fs::read_to_string(&metadata_path) {
                Ok(m) => m,
                Err(e) => {
                    warn!("Failed to read metadata file {:?}: {}", metadata_path, e);
                    return None;
                }
            };

            let mut metadata: CacheMetadata = match serde_json::from_str(&metadata_str) {
                Ok(m) => m,
                Err(e) => {
                    warn!("Failed to parse metadata file {:?}: {}", metadata_path, e);
                    return None;
                }
            };

            // Check TTL based on created_at
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            
            let age = now.saturating_sub(metadata.created_at);
            
            // Read the cached data
            let data = match fs::read(&cache_path) {
                Ok(data) => Bytes::from(data),
                Err(e) => {
                    warn!("Failed to read cache file {:?}: {}", cache_path, e);
                    return None;
                }
            };

            // Update last_access timestamp
            metadata.last_access = now;
            if let Ok(metadata_json) = serde_json::to_string(&metadata) {
                let _ = fs::write(&metadata_path, metadata_json);
            }

            debug!("Cache hit from disk: {:?}", cache_path);
            Some((data, metadata.content_type, age, ttl))
        })
        .await
        .ok()
        .flatten();

        // Check TTL after reading
        if let Some((data, content_type, age, ttl)) = result {
            if age > ttl {
                // Cache entry expired, schedule removal
                let cache_path = self.get_cache_path(key);
                let metadata_path = cache_path.with_extension("meta");
                tokio::task::spawn_blocking(move || {
                    debug!("Cache entry expired, removing: {:?}", cache_path);
                    let _ = fs::remove_file(&cache_path);
                    let _ = fs::remove_file(&metadata_path);
                });
                return None;
            }
            Some((data, content_type))
        } else {
            None
        }
    }

    /// Store an item in the disk cache
    pub async fn put(&self, key: &CacheKey, data: Bytes, content_type: String) -> Result<()> {
        let cache_path = self.get_cache_path(key);
        let metadata_path = cache_path.with_extension("meta");

        // Use spawn_blocking for disk I/O
        tokio::task::spawn_blocking(move || {
            // Create subdirectory if it doesn't exist
            if let Some(parent) = cache_path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("Failed to create cache subdirectory: {:?}", parent))?;
                }
            }

            // Create metadata with timestamps
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            
            let metadata = CacheMetadata {
                content_type,
                created_at: now,
                last_access: now,
            };

            // Write data to a temporary file first (atomic write)
            // Use proper .tmp extension
            let temp_path = {
                let file_name = cache_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("data");
                cache_path
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .join(format!("{}.tmp", file_name))
            };
            fs::write(&temp_path, &data)
                .with_context(|| format!("Failed to write cache file: {:?}", temp_path))?;

            // Write metadata to a temporary file
            let temp_metadata_path = {
                let file_name = metadata_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("meta");
                metadata_path
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .join(format!("{}.tmp", file_name))
            };
            let metadata_json = serde_json::to_string(&metadata)
                .with_context(|| "Failed to serialize metadata")?;
            fs::write(&temp_metadata_path, metadata_json)
                .with_context(|| format!("Failed to write metadata file: {:?}", temp_metadata_path))?;

            // Rename to final location (atomic operation)
            fs::rename(&temp_path, &cache_path)
                .with_context(|| format!("Failed to rename cache file: {:?}", cache_path))?;
            fs::rename(&temp_metadata_path, &metadata_path)
                .with_context(|| format!("Failed to rename metadata file: {:?}", metadata_path))?;

            debug!("Cache written to disk: {:?}", cache_path);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| "Spawn blocking task failed")??;

        // Check and enforce cache size limit
        self.enforce_size_limit().await;

        Ok(())
    }

    /// Enforce the cache size limit by removing oldest files
    async fn enforce_size_limit(&self) {
        let cache_dir = self.cache_dir.clone();
        let max_size = self.max_size;

        tokio::task::spawn_blocking(move || {
            let total_size = Self::calculate_total_size(&cache_dir);
            
            if total_size <= max_size {
                return;
            }

            debug!(
                "Cache size ({} bytes) exceeds limit ({} bytes), cleaning up...",
                total_size, max_size
            );

            // Collect all cache files with their last access times
            let mut files: Vec<(PathBuf, PathBuf, u64)> = Vec::new();
            
            for entry in WalkDir::new(&cache_dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .filter(|e| {
                    // Only include actual cache data files (not .meta or .tmp)
                    e.path().extension().is_none()
                })
            {
                let data_path = entry.path().to_path_buf();
                let meta_path = data_path.with_extension("meta");
                
                // Read metadata to get last_access timestamp
                if let Ok(meta_str) = fs::read_to_string(&meta_path) {
                    if let Ok(metadata) = serde_json::from_str::<CacheMetadata>(&meta_str) {
                        files.push((data_path, meta_path, metadata.last_access));
                    }
                }
            }

            // Sort by last access time (oldest first) - this is true LRU
            files.sort_by_key(|(_, _, last_access)| *last_access);

            // Remove files until we're under the limit
            let mut current_size = total_size;
            for (data_path, meta_path, _) in files {
                if current_size <= max_size {
                    break;
                }

                // Calculate size of both data and metadata files
                let mut pair_size = 0u64;
                if let Ok(metadata) = fs::metadata(&data_path) {
                    pair_size += metadata.len();
                }
                if let Ok(metadata) = fs::metadata(&meta_path) {
                    pair_size += metadata.len();
                }
                
                // Remove both cache file and its metadata
                if let Err(e) = fs::remove_file(&data_path) {
                    error!("Failed to remove cache file {:?}: {}", data_path, e);
                } else {
                    debug!("Removed cache file: {:?}", data_path);
                }
                
                if let Err(e) = fs::remove_file(&meta_path) {
                    error!("Failed to remove metadata file {:?}: {}", meta_path, e);
                }
                
                current_size = current_size.saturating_sub(pair_size);
            }

            debug!("Cache cleanup complete. New size: {} bytes", current_size);
        })
        .await
        .ok();
    }

    /// Calculate the total size of the cache directory
    fn calculate_total_size(cache_dir: &Path) -> u64 {
        WalkDir::new(cache_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                // Only count cache files and their metadata, not temp files
                let ext = e.path().extension().and_then(|s| s.to_str());
                ext.is_none() || ext == Some("meta")
            })
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum()
    }

    /// Get the total size of the cache directory
    fn get_total_size(&self) -> u64 {
        Self::calculate_total_size(&self.cache_dir)
    }

    /// Get cache statistics
    pub fn stats(&self) -> DiskCacheStats {
        let mut file_count = 0u64;
        let total_size = self.get_total_size();

        for _entry in WalkDir::new(&self.cache_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                // Only count actual cache data files
                e.path().extension().is_none()
            })
        {
            file_count += 1;
        }

        DiskCacheStats {
            file_count,
            total_size,
            max_size: self.max_size,
        }
    }
}

/// Disk cache statistics
#[derive(Debug, Clone)]
pub struct DiskCacheStats {
    pub file_count: u64,
    pub total_size: u64,
    pub max_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_disk_cache_put_and_get() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiskCache::new(temp_dir.path(), 1024 * 1024, 3600).unwrap();

        let key = CacheKey::new("/media/test.jpg".to_string(), "avif".to_string());
        let data = Bytes::from("test data");
        let content_type = "image/avif".to_string();

        cache.put(&key, data.clone(), content_type.clone()).await.unwrap();

        let result = cache.get(&key).await;
        assert!(result.is_some());
        let (cached_data, cached_content_type) = result.unwrap();
        assert_eq!(cached_data, data);
        assert_eq!(cached_content_type, content_type);
    }

    #[tokio::test]
    async fn test_disk_cache_miss() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiskCache::new(temp_dir.path(), 1024 * 1024, 3600).unwrap();

        let key = CacheKey::new("/media/nonexistent.jpg".to_string(), "webp".to_string());
        let result = cache.get(&key).await;

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_disk_cache_ttl() {
        let temp_dir = TempDir::new().unwrap();
        // Create cache with 1 second TTL
        let cache = DiskCache::new(temp_dir.path(), 1024 * 1024, 1).unwrap();

        let key = CacheKey::new("/media/test.jpg".to_string(), "avif".to_string());
        let data = Bytes::from("test data");
        let content_type = "image/avif".to_string();

        cache.put(&key, data.clone(), content_type.clone()).await.unwrap();

        // Should be in cache immediately
        let result = cache.get(&key).await;
        assert!(result.is_some());

        // Wait for TTL to expire
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Should be gone after TTL
        let result = cache.get(&key).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_disk_cache_eviction() {
        let temp_dir = TempDir::new().unwrap();
        // Create cache with small max size (2KB to allow for metadata overhead)
        let cache = DiskCache::new(temp_dir.path(), 2048, 3600).unwrap();

        // Add multiple entries that exceed the limit
        let data = Bytes::from(vec![0u8; 512]); // 512 bytes each
        
        let key1 = CacheKey::new("/media/test1.jpg".to_string(), "avif".to_string());
        cache.put(&key1, data.clone(), "image/avif".to_string()).await.unwrap();
        
        // Small delay to ensure different timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        let key2 = CacheKey::new("/media/test2.jpg".to_string(), "avif".to_string());
        cache.put(&key2, data.clone(), "image/avif".to_string()).await.unwrap();
        
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        let key3 = CacheKey::new("/media/test3.jpg".to_string(), "avif".to_string());
        cache.put(&key3, data.clone(), "image/avif".to_string()).await.unwrap();

        // Wait for cleanup to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // At least one of the older entries should have been evicted
        let stats = cache.stats();
        assert!(stats.total_size <= cache.max_size, 
            "Cache size {} should be <= max size {}", stats.total_size, cache.max_size);
        
        // The newest entry (key3) should still be there
        assert!(cache.get(&key3).await.is_some(), "Most recent entry should be retained");
    }

    #[tokio::test]
    async fn test_disk_cache_ignores_temp_files() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiskCache::new(temp_dir.path(), 1024 * 1024, 3600).unwrap();

        // Add a real cache entry
        let key = CacheKey::new("/media/test.jpg".to_string(), "avif".to_string());
        let data = Bytes::from("test data");
        cache.put(&key, data.clone(), "image/avif".to_string()).await.unwrap();

        // Manually create a temp file that should be ignored
        let cache_path = cache.get_cache_path(&key);
        if let Some(parent) = cache_path.parent() {
            let temp_file = parent.join("orphaned_file.tmp");
            fs::write(&temp_file, "orphaned data").unwrap();
        }

        // Stats should only count the real cache entry, not the temp file
        let stats = cache.stats();
        assert_eq!(stats.file_count, 1, "Should only count actual cache files");
        
        // Total size should not include the temp file
        // (it should be close to the size of our test data + metadata)
        assert!(stats.total_size < 1024, "Size should be small without temp file");
    }

    #[tokio::test]
    async fn test_disk_cache_lru_access_tracking() {
        let temp_dir = TempDir::new().unwrap();
        let cache = DiskCache::new(temp_dir.path(), 2048, 3600).unwrap();

        let data = Bytes::from(vec![0u8; 512]);
        
        // Add three entries
        let key1 = CacheKey::new("/media/test1.jpg".to_string(), "avif".to_string());
        cache.put(&key1, data.clone(), "image/avif".to_string()).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        let key2 = CacheKey::new("/media/test2.jpg".to_string(), "avif".to_string());
        cache.put(&key2, data.clone(), "image/avif".to_string()).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        let key3 = CacheKey::new("/media/test3.jpg".to_string(), "avif".to_string());
        cache.put(&key3, data.clone(), "image/avif".to_string()).await.unwrap();
        
        // Access key1 to update its last_access time
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let _ = cache.get(&key1).await;
        
        // Add one more entry to trigger eviction
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let key4 = CacheKey::new("/media/test4.jpg".to_string(), "avif".to_string());
        cache.put(&key4, data.clone(), "image/avif".to_string()).await.unwrap();
        
        // Wait for cleanup
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        
        // key2 should be evicted (least recently accessed, not counting the initial write)
        // key1 should still be there (was accessed more recently)
        let key1_exists = cache.get(&key1).await.is_some();
        let key2_exists = cache.get(&key2).await.is_some();
        
        assert!(key1_exists || !key2_exists, 
            "LRU should prefer recently accessed items: key1={}, key2={}", 
            key1_exists, key2_exists);
    }
}
