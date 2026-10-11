//! Fixed-window rate limiter on outbound sends — a guardrail so an agent
//! stuck in a loop can't blast emails through your account.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<Mutex<State>>,
    window: Duration,
    max: u32,
}

struct State {
    window_start: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn new(window: Duration, max: u32) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                window_start: Instant::now(),
                count: 0,
            })),
            window,
            max,
        }
    }

    /// Ok(()) if this call is within the limit, Err(seconds until the window
    /// resets) once `max` calls happened inside the current window.
    pub async fn check(&self) -> Result<(), u64> {
        let mut s = self.state.lock().await;
        let now = Instant::now();
        let elapsed = now.duration_since(s.window_start);
        if elapsed >= self.window {
            s.window_start = now;
            s.count = 0;
        }
        if s.count >= self.max {
            return Err((self.window - elapsed).as_secs().max(1));
        }
        s.count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allows_up_to_max_then_blocks() {
        let rl = RateLimiter::new(Duration::from_secs(60), 3);
        assert!(rl.check().await.is_ok());
        assert!(rl.check().await.is_ok());
        assert!(rl.check().await.is_ok());
        let retry_in = rl.check().await.expect_err("4th call must be blocked");
        assert!((1..=60).contains(&retry_in));
    }

    #[tokio::test]
    async fn resets_after_window() {
        let rl = RateLimiter::new(Duration::from_millis(50), 1);
        assert!(rl.check().await.is_ok());
        assert!(rl.check().await.is_err());
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(rl.check().await.is_ok());
    }

    #[tokio::test]
    async fn clones_share_state() {
        let rl = RateLimiter::new(Duration::from_secs(60), 1);
        let clone = rl.clone();
        assert!(rl.check().await.is_ok());
        assert!(clone.check().await.is_err());
    }
}
