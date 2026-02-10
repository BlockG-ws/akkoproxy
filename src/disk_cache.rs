use anyhow::{Context, Result};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{debug, error, warn};
use walkdir::WalkDir;

use crate::cache::CacheKey;

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

        // Check if both cache file and metadata exist
        if !cache_path.exists() || !metadata_path.exists() {
            return None;
        }

        // Check TTL
        if let Ok(metadata) = fs::metadata(&cache_path) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = SystemTime::now().duration_since(modified) {
                    if duration.as_secs() > self.ttl {
                        // Cache entry expired, remove it
                        debug!("Cache entry expired: {:?}", cache_path);
                        let _ = fs::remove_file(&cache_path);
                        let _ = fs::remove_file(&metadata_path);
                        return None;
                    }
                }
            }
        }

        // Read the cached data
        let data = match fs::read(&cache_path) {
            Ok(data) => Bytes::from(data),
            Err(e) => {
                warn!("Failed to read cache file {:?}: {}", cache_path, e);
                return None;
            }
        };

        // Read the content type from metadata file
        let content_type = match fs::read_to_string(&metadata_path) {
            Ok(ct) => ct,
            Err(e) => {
                warn!("Failed to read metadata file {:?}: {}", metadata_path, e);
                return None;
            }
        };

        debug!("Cache hit from disk: {:?}", cache_path);
        Some((data, content_type))
    }

    /// Store an item in the disk cache
    pub async fn put(&self, key: &CacheKey, data: Bytes, content_type: String) -> Result<()> {
        let cache_path = self.get_cache_path(key);
        let metadata_path = cache_path.with_extension("meta");

        // Create subdirectory if it doesn't exist
        if let Some(parent) = cache_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create cache subdirectory: {:?}", parent))?;
            }
        }

        // Write data to a temporary file first (atomic write)
        let temp_path = cache_path.with_extension("data.tmp");
        fs::write(&temp_path, &data)
            .with_context(|| format!("Failed to write cache file: {:?}", temp_path))?;

        // Write metadata to a temporary file
        let temp_metadata_path = metadata_path.with_extension("meta.tmp");
        fs::write(&temp_metadata_path, &content_type)
            .with_context(|| format!("Failed to write metadata file: {:?}", temp_metadata_path))?;

        // Rename to final location (atomic operation)
        fs::rename(&temp_path, &cache_path)
            .with_context(|| format!("Failed to rename cache file: {:?}", cache_path))?;
        fs::rename(&temp_metadata_path, &metadata_path)
            .with_context(|| format!("Failed to rename metadata file: {:?}", metadata_path))?;

        debug!("Cache written to disk: {:?}", cache_path);

        // Check and enforce cache size limit
        self.enforce_size_limit().await;

        Ok(())
    }

    /// Enforce the cache size limit by removing oldest files
    async fn enforce_size_limit(&self) {
        let total_size = self.get_total_size();
        
        if total_size <= self.max_size {
            return;
        }

        debug!(
            "Cache size ({} bytes) exceeds limit ({} bytes), cleaning up...",
            total_size, self.max_size
        );

        // Collect all cache files with their access times
        let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
        
        for entry in WalkDir::new(&self.cache_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| !e.path().extension().map_or(false, |ext| ext == "meta" || ext == "tmp"))
        {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(accessed) = metadata.modified() {
                    files.push((entry.path().to_path_buf(), accessed));
                }
            }
        }

        // Sort by access time (oldest first)
        files.sort_by_key(|(_, time)| *time);

        // Remove files until we're under the limit
        let mut current_size = total_size;
        for (file_path, _) in files {
            if current_size <= self.max_size {
                break;
            }

            if let Ok(metadata) = fs::metadata(&file_path) {
                let file_size = metadata.len();
                
                // Remove cache file and its metadata
                let metadata_path = file_path.with_extension("meta");
                if let Err(e) = fs::remove_file(&file_path) {
                    error!("Failed to remove cache file {:?}: {}", file_path, e);
                } else {
                    current_size = current_size.saturating_sub(file_size);
                    debug!("Removed cache file: {:?}", file_path);
                }
                
                let _ = fs::remove_file(&metadata_path);
            }
        }

        debug!("Cache cleanup complete. New size: {} bytes", current_size);
    }

    /// Get the total size of the cache directory
    fn get_total_size(&self) -> u64 {
        WalkDir::new(&self.cache_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum()
    }

    /// Get cache statistics
    pub fn stats(&self) -> DiskCacheStats {
        let mut file_count = 0u64;
        let mut total_size = 0u64;

        for entry in WalkDir::new(&self.cache_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| !e.path().extension().map_or(false, |ext| ext == "meta" || ext == "tmp"))
        {
            file_count += 1;
            if let Ok(metadata) = entry.metadata() {
                total_size += metadata.len();
            }
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
}
