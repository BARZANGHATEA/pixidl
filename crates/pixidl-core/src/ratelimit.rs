//! Async token-bucket rate limiter with a live-adjustable rate.
//!
//! The HTTP engine acquires tokens from both the per-download limiter and the
//! global limiter before writing each chunk, so both limits are enforced.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;

pub struct RateLimiter {
    /// Bytes per second; 0 = unlimited.
    rate: AtomicU64,
    state: Mutex<Bucket>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new(rate_bps: Option<u64>) -> Self {
        Self {
            rate: AtomicU64::new(rate_bps.unwrap_or(0)),
            state: Mutex::new(Bucket { tokens: 0.0, last: Instant::now() }),
        }
    }

    pub fn set_rate(&self, rate_bps: Option<u64>) {
        self.rate.store(rate_bps.unwrap_or(0), Ordering::Relaxed);
        let mut s = self.state.lock();
        s.tokens = s.tokens.min(rate_bps.unwrap_or(0) as f64);
        s.last = Instant::now();
    }

    pub fn rate(&self) -> Option<u64> {
        match self.rate.load(Ordering::Relaxed) {
            0 => None,
            r => Some(r),
        }
    }

    /// Waits until `n` bytes may be transferred.
    pub async fn acquire(&self, n: u64) {
        let mut remaining = n as f64;
        loop {
            let rate = self.rate.load(Ordering::Relaxed);
            if rate == 0 {
                return;
            }
            let wait = {
                let mut s = self.state.lock();
                let now = Instant::now();
                let elapsed = now.duration_since(s.last).as_secs_f64();
                s.last = now;
                // Allow a burst of at most 1/4 second worth of data.
                let cap = (rate as f64 / 4.0).max(16.0 * 1024.0);
                s.tokens = (s.tokens + elapsed * rate as f64).min(cap);
                if s.tokens >= remaining {
                    s.tokens -= remaining;
                    return;
                }
                remaining -= s.tokens.max(0.0);
                s.tokens = 0.0;
                Duration::from_secs_f64((remaining / rate as f64).min(0.25))
            };
            tokio::time::sleep(wait).await;
        }
    }
}

/// Exponential backoff schedule for automatic retries.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl RetryPolicy {
    pub fn new(max_retries: u32, base_delay_secs: u32) -> Self {
        Self { max_retries, base_delay: Duration::from_secs(base_delay_secs as u64), max_delay: Duration::from_secs(120) }
    }

    /// Delay before retry number `attempt` (1-based), or `None` when exhausted.
    pub fn delay_for(&self, attempt: u32) -> Option<Duration> {
        if attempt == 0 || attempt > self.max_retries {
            return None;
        }
        let factor = 2u32.saturating_pow(attempt - 1);
        Some(self.base_delay.saturating_mul(factor).min(self.max_delay))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_backoff() {
        let p = RetryPolicy::new(4, 2);
        assert_eq!(p.delay_for(0), None);
        assert_eq!(p.delay_for(1), Some(Duration::from_secs(2)));
        assert_eq!(p.delay_for(2), Some(Duration::from_secs(4)));
        assert_eq!(p.delay_for(3), Some(Duration::from_secs(8)));
        assert_eq!(p.delay_for(4), Some(Duration::from_secs(16)));
        assert_eq!(p.delay_for(5), None);
        let p = RetryPolicy::new(20, 30);
        assert_eq!(p.delay_for(10), Some(Duration::from_secs(120)));
        assert_eq!(RetryPolicy::new(0, 1).delay_for(1), None);
    }

    #[tokio::test(start_paused = true)]
    async fn unlimited_is_instant() {
        let l = RateLimiter::new(None);
        let t = Instant::now();
        l.acquire(100_000_000).await;
        assert!(t.elapsed() < Duration::from_millis(1));
    }

    #[tokio::test(start_paused = true)]
    async fn limits_throughput() {
        let l = RateLimiter::new(Some(100_000));
        let t = Instant::now();
        for _ in 0..50 {
            l.acquire(10_000).await; // 500 KB total at 100 KB/s ≈ 5 s
        }
        let e = t.elapsed().as_secs_f64();
        assert!((4.5..=5.5).contains(&e), "elapsed {e}");
    }

    #[tokio::test(start_paused = true)]
    async fn rate_can_change_live() {
        let l = RateLimiter::new(Some(1_000));
        l.set_rate(None);
        let t = Instant::now();
        l.acquire(1_000_000).await;
        assert!(t.elapsed() < Duration::from_millis(1));
        assert_eq!(l.rate(), None);
    }
}
