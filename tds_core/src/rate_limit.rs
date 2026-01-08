//! A simple token bucket implementation for rate limiting.

use std::time::Instant;

/// A token bucket rate limiter.
///
/// This struct implements the token bucket algorithm to control the rate of operations.
#[derive(Clone)]
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_rate: f64, // tokens per second
    last_refill: Instant,
}

impl TokenBucket {
    /// Creates a new `TokenBucket`.
    ///
    /// # Arguments
    ///
    /// * `capacity` - The maximum number of tokens the bucket can hold.
    /// * `refill_rate` - The rate at which tokens are added to the bucket (tokens per second).
    pub fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Attempts to consume a specified number of tokens from the bucket.
    ///
    /// # Arguments
    ///
    /// * `amount` - The number of tokens to consume.
    ///
    /// # Returns
    ///
    /// `true` if the tokens were successfully consumed, `false` otherwise.
    pub fn consume(&mut self, amount: f64) -> bool {
        self.refill();
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        let new_tokens = elapsed * self.refill_rate;

        if new_tokens > 0.0 {
            self.tokens = (self.tokens + new_tokens).min(self.capacity);
            self.last_refill = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_token_bucket() {
        let mut bucket = TokenBucket::new(10.0, 1.0);
        
        // Initial capacity is full
        assert!(bucket.consume(10.0));
        // Should be empty now
        assert!(!bucket.consume(1.0));

        // Sleep for 1.1s to get ~1 token
        thread::sleep(Duration::from_millis(1100));
        assert!(bucket.consume(1.0));
    }
}
