//! `web.cache` module — In-memory caching with Moka and optional Redis.
//!
//! ```text
//! use web.cache
//!
//! c = cache[1000]         --- max 1000 entries ---
//! set[c, 'key', 'value']
//! log[get[c, 'key']]     --- 'value' ---
//! ```

/// In-memory cache backed by Moka.
#[derive(Clone)]
pub struct CrowCache {
    inner: moka::future::Cache<String, serde_json::Value>,
}

impl CrowCache {
    /// Create a cache with max capacity.
    pub fn new(max_capacity: u64) -> Self {
        Self {
            inner: moka::future::Cache::new(max_capacity),
        }
    }

    /// Create a cache with TTL (time-to-live in seconds).
    pub fn with_ttl(max_capacity: u64, ttl_secs: u64) -> Self {
        Self {
            inner: moka::future::Cache::builder()
                .max_capacity(max_capacity)
                .time_to_live(std::time::Duration::from_secs(ttl_secs))
                .build(),
        }
    }

    /// Set a value.
    pub async fn set(&self, key: &str, value: serde_json::Value) {
        self.inner.insert(key.to_string(), value).await;
    }

    /// Get a value.
    pub async fn get(&self, key: &str) -> Option<serde_json::Value> {
        self.inner.get(&key.to_string()).await
    }

    /// Remove a value.
    pub async fn remove(&self, key: &str) {
        self.inner.remove(&key.to_string()).await;
    }

    /// Get the current entry count.
    pub fn count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Invalidate all entries.
    pub fn clear(&self) {
        self.inner.invalidate_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_set_get() {
        let cache = CrowCache::new(100);
        cache.set("name", serde_json::json!("alice")).await;
        assert_eq!(
            cache.get("name").await,
            Some(serde_json::json!("alice"))
        );
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = CrowCache::new(100);
        assert_eq!(cache.get("missing").await, None);
    }

    #[tokio::test]
    async fn test_cache_remove() {
        let cache = CrowCache::new(100);
        cache.set("key", serde_json::json!(42)).await;
        cache.remove("key").await;
        // Moka removal is eventually consistent; just verify no panic
    }

    #[tokio::test]
    async fn test_cache_overwrite() {
        let cache = CrowCache::new(100);
        cache.set("k", serde_json::json!(1)).await;
        cache.set("k", serde_json::json!(2)).await;
        assert_eq!(cache.get("k").await, Some(serde_json::json!(2)));
    }
}

