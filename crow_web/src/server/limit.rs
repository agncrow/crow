//! A route's rate limit, `route['/api', h, limit = 60]`: so many requests a
//! minute from each client, a token bucket per client refilled as the
//! minute passes. A client is its address — a proxy's on this machine (the
//! address is a loopback one) is the first address its `X-Forwarded-For`
//! names. The buckets of clients long gone are dropped, so a flood of
//! addresses cannot grow them without end.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Requests a minute, per client.
#[derive(Clone)]
pub struct Limit {
    per_minute: u64,
    buckets: Arc<Mutex<HashMap<String, (f64, Instant)>>>,
}

impl Limit {
    pub fn new(per_minute: u64) -> Self {
        Limit { per_minute: per_minute.max(1), buckets: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Whether `client` may make a request now, or the seconds until it may.
    pub fn check(&self, client: &str) -> Result<(), u64> {
        let rate = self.per_minute as f64 / 60.0;
        let max = self.per_minute as f64;
        let now = Instant::now();
        let mut buckets = crow_runtime::lock(&self.buckets);
        // a bucket a minute untouched is full again: forget it
        if buckets.len() > 10_000 {
            buckets.retain(|_, (_, at)| now.duration_since(*at) < Duration::from_secs(60));
        }
        let (tokens, at) = buckets.entry(client.to_string()).or_insert((max, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * rate).min(max);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - *tokens) / rate).ceil().max(1.0) as u64)
        }
    }
}

/// The client a request comes from: its peer's address, or, when the peer
/// is this machine (a proxy in front of the app), the first address the
/// proxy's `X-Forwarded-For` names.
pub fn client_of(peer: Option<std::net::SocketAddr>, forwarded: Option<&str>) -> String {
    match peer {
        Some(addr) if !addr.ip().is_loopback() => addr.ip().to_string(),
        _ => forwarded.and_then(|f| f.split(',').next()).map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).or_else(|| peer.map(|a| a.ip().to_string())).unwrap_or_else(|| "unknown".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_over_its_limit_waits() {
        let l = Limit::new(2);
        assert_eq!(l.check("a"), Ok(()));
        assert_eq!(l.check("a"), Ok(()));
        let wait = l.check("a").err().unwrap_or(0);
        assert!((1..=30).contains(&wait), "{}", wait);
        // another client has its own bucket
        assert_eq!(l.check("b"), Ok(()));
    }

    #[test]
    fn a_client_is_its_address_or_its_proxys_word() {
        let far: std::net::SocketAddr = "203.0.113.9:5000".parse().unwrap_or_else(|_| unreachable!());
        let near: std::net::SocketAddr = "127.0.0.1:5000".parse().unwrap_or_else(|_| unreachable!());
        assert_eq!(client_of(Some(far), Some("198.51.100.1")), "203.0.113.9");
        assert_eq!(client_of(Some(near), Some("198.51.100.1, 10.0.0.2")), "198.51.100.1");
        assert_eq!(client_of(Some(near), None), "127.0.0.1");
    }
}
