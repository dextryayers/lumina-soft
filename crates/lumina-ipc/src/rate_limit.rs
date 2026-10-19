//! Per-client command budget.
//!
//! A client may burst up to 120 commands and then refills one token every
//! 8.3 milliseconds. Exceeding the budget returns `RateLimited` and leaves the
//! connection open, so a script that accidentally loops cannot take the island
//! down or lock itself out permanently.

use std::time::Instant;

/// The documented default budget.
pub const DEFAULT_RATE_PER_SECOND: u32 = 120;

/// A token bucket with a one-second capacity.
#[derive(Debug)]
pub struct RateLimiter {
    capacity: f64,
    tokens: f64,
    refill_per_second: f64,
    last: Instant,
}

impl RateLimiter {
    /// Create a limiter that accepts `rate_per_second` commands after a burst.
    ///
    /// A rate of zero is raised to one rather than disabling the limiter, which
    /// would make the protection opt-out by accident.
    pub fn new(rate_per_second: u32) -> Self {
        let rate = rate_per_second.max(1);
        Self {
            capacity: f64::from(rate),
            tokens: f64::from(rate),
            refill_per_second: f64::from(rate),
            last: Instant::now(),
        }
    }

    /// The configured burst capacity.
    pub fn capacity(&self) -> u32 {
        self.capacity as u32
    }

    /// Try to spend one token.
    pub fn try_acquire(&mut self) -> bool {
        self.try_acquire_at(Instant::now())
    }

    pub(crate) fn try_acquire_at(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.refill_per_second).min(self.capacity);
            self.last = now;
        }
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_full_burst_is_allowed() {
        let mut limiter = RateLimiter::new(3);
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(!limiter.try_acquire());
    }

    #[test]
    fn tokens_refill_over_time() {
        let mut limiter = RateLimiter::new(2);
        let start = Instant::now();
        assert!(limiter.try_acquire_at(start));
        assert!(limiter.try_acquire_at(start));
        assert!(!limiter.try_acquire_at(start));
        assert!(limiter.try_acquire_at(start + Duration::from_millis(500)));
        assert!(!limiter.try_acquire_at(start + Duration::from_millis(500)));
    }

    #[test]
    fn refill_is_capped_at_the_burst_capacity() {
        let mut limiter = RateLimiter::new(2);
        let start = Instant::now();
        assert!(limiter.try_acquire_at(start));
        assert!(limiter.try_acquire_at(start));
        let later = start + Duration::from_secs(60);
        assert!(limiter.try_acquire_at(later));
        assert!(limiter.try_acquire_at(later));
        assert!(!limiter.try_acquire_at(later));
    }

    #[test]
    fn a_zero_rate_is_lifted_to_one() {
        let mut limiter = RateLimiter::new(0);
        assert_eq!(limiter.capacity(), 1);
        assert!(limiter.try_acquire());
        assert!(!limiter.try_acquire());
    }

    #[test]
    fn the_default_rate_matches_the_specification() {
        assert_eq!(DEFAULT_RATE_PER_SECOND, 120);
        let limiter = RateLimiter::new(DEFAULT_RATE_PER_SECOND);
        assert_eq!(limiter.capacity(), 120);
    }

    #[test]
    fn time_going_backwards_does_not_add_tokens() {
        let mut limiter = RateLimiter::new(1);
        let start = Instant::now();
        assert!(limiter.try_acquire_at(start));
        assert!(!limiter.try_acquire_at(start));
    }
}
