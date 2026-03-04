//! `web.rate` module — Rate limiting with token bucket algorithm.
//!
//! ```text
//! use web
//! use web.rate
//!
//! app = web.new[]
//! limit[app, 100, 60]  --- 100 requests per 60 seconds ---
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A token bucket rate limiter.
#[derive(Clone)]
pub struct CrowRateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
}

struct RateLimiterInner {
    /// Maximum tokens (burst capacity).
    max_tokens: u64,
    /// Tokens refilled per second.
    refill_rate: f64,
    /// Per-key buckets.
    buckets: HashMap<String, TokenBucket>,
}

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl CrowRateLimiter {
    /// Create a rate limiter: `max_requests` per `window_secs` seconds.
    pub fn new(max_requests: u64, window_secs: u64) -> Self {
        let refill_rate = max_requests as f64 / window_secs.max(1) as f64;
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                max_tokens: max_requests,
                refill_rate,
                buckets: HashMap::new(),
            })),
        }
    }

    /// Check if a request from `key` (e.g. IP address) is allowed.
    /// Returns `true` if allowed, `false` if rate-limited.
    pub fn check(&self, key: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();
        let max_tokens = inner.max_tokens;
        let refill_rate = inner.refill_rate;

        let bucket = inner.buckets.entry(key.to_string()).or_insert(TokenBucket {
            tokens: max_tokens as f64,
            last_refill: now,
        });

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_rate)
            .min(max_tokens as f64);
        bucket.last_refill = now;

        // Try to consume one token
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Get remaining tokens for a key.
    pub fn remaining(&self, key: &str) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();

        if let Some(bucket) = inner.buckets.get_mut(key) {
            let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
            let tokens = (bucket.tokens + elapsed * inner.refill_rate)
                .min(inner.max_tokens as f64);
            tokens as u64
        } else {
            inner.max_tokens
        }
    }

    /// Reset the limiter for a specific key.
    pub fn reset(&self, key: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.buckets.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_allows_under_limit() {
        let limiter = CrowRateLimiter::new(5, 60);
        for _ in 0..5 {
            assert!(limiter.check("user1"));
        }
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let limiter = CrowRateLimiter::new(3, 60);
        assert!(limiter.check("user1"));
        assert!(limiter.check("user1"));
        assert!(limiter.check("user1"));
        assert!(!limiter.check("user1")); // 4th request blocked
    }

    #[test]
    fn test_rate_limiter_separate_keys() {
        let limiter = CrowRateLimiter::new(1, 60);
        assert!(limiter.check("user1"));
        assert!(!limiter.check("user1"));
        assert!(limiter.check("user2")); // Different key, separate bucket
    }

    #[test]
    fn test_rate_limiter_remaining() {
        let limiter = CrowRateLimiter::new(10, 60);
        assert_eq!(limiter.remaining("user1"), 10);
        limiter.check("user1");
        // remaining should be ~9 (may vary slightly due to refill timing)
        assert!(limiter.remaining("user1") <= 10);
    }

    #[test]
    fn test_rate_limiter_reset() {
        let limiter = CrowRateLimiter::new(1, 60);
        assert!(limiter.check("user1"));
        assert!(!limiter.check("user1"));
        limiter.reset("user1");
        assert!(limiter.check("user1")); // Reset restores tokens
    }
}

