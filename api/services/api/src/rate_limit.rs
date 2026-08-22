//! Minimal in-process per-IP rate limiter for unauthenticated endpoints.
//!
//! Not a distributed limiter -- fine for a single-replica-class concern like
//! deterring casual abuse of a public prospect-quote endpoint, not for
//! defending against a real DDoS (that's the ingress/WAF's job). Sliding
//! window per key, with lazy cleanup of stale keys on each check so memory
//! doesn't grow unbounded.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    max_requests: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true if `key` is still under the limit (and records this
    /// attempt); false if the limit is already reached for this window.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut hits = self.hits.lock().expect("rate limiter mutex poisoned");

        // Lazy cleanup: drop keys whose entire window has expired, so a
        // long-running process doesn't accumulate one entry per distinct
        // IP forever.
        hits.retain(|_, timestamps| {
            timestamps
                .back()
                .is_some_and(|latest| now.duration_since(*latest) < self.window)
        });

        let timestamps = hits.entry(key.to_string()).or_default();
        while timestamps
            .front()
            .is_some_and(|oldest| now.duration_since(*oldest) >= self.window)
        {
            timestamps.pop_front();
        }

        if timestamps.len() >= self.max_requests {
            return false;
        }

        timestamps.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_requests_under_the_limit() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        assert!(limiter.check("1.2.3.4"));
        assert!(limiter.check("1.2.3.4"));
        assert!(limiter.check("1.2.3.4"));
    }

    #[test]
    fn rejects_requests_over_the_limit_within_the_window() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("5.6.7.8"));
        assert!(limiter.check("5.6.7.8"));
        assert!(!limiter.check("5.6.7.8"));
    }

    #[test]
    fn tracks_keys_independently() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check("a"));
        assert!(limiter.check("b"));
        assert!(!limiter.check("a"));
        assert!(!limiter.check("b"));
    }

    #[test]
    fn allows_again_once_the_window_has_fully_elapsed() {
        let limiter = RateLimiter::new(1, Duration::from_millis(20));
        assert!(limiter.check("c"));
        assert!(!limiter.check("c"));
        std::thread::sleep(Duration::from_millis(30));
        assert!(limiter.check("c"));
    }
}
