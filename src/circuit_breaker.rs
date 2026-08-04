use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const STATE_CLOSED: u8 = 0;
const STATE_OPEN: u8 = 1;
const STATE_HALF_OPEN: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Closed,
    Open,
    HalfOpen,
}

impl From<u8> for State {
    fn from(v: u8) -> Self {
        match v {
            STATE_CLOSED => State::Closed,
            STATE_OPEN => State::Open,
            STATE_HALF_OPEN => State::HalfOpen,
            _ => State::Closed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CircuitStats {
    pub state: State,
    pub failure_count: u64,
    pub success_count: u64,
    pub last_failure: Option<Instant>,
}

pub struct CircuitBreaker {
    state: AtomicU8,
    failure_count: AtomicU64,
    success_count: AtomicU64,
    last_failure: RwLock<Option<Instant>>,
    failure_threshold: u64,
    recovery_timeout: Duration,
    half_open_max: u64,
    half_open_remaining: AtomicU64,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum CircuitBreakerError {
    #[error("circuit breaker is open — requests rejected")]
    CircuitOpen,
    #[error("upstream error: {0}")]
    UpstreamError(String),
}

impl CircuitBreaker {
    pub fn new(
        failure_threshold: u64,
        recovery_timeout_secs: u64,
        half_open_max: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: AtomicU8::new(STATE_CLOSED),
            failure_count: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            last_failure: RwLock::new(None),
            failure_threshold,
            recovery_timeout: Duration::from_secs(recovery_timeout_secs),
            half_open_max,
            half_open_remaining: AtomicU64::new(half_open_max),
        })
    }

    pub fn state(&self) -> State {
        State::from(self.state.load(Ordering::Acquire))
    }

    pub fn stats(&self) -> CircuitStats {
        CircuitStats {
            state: self.state(),
            failure_count: self.failure_count.load(Ordering::Relaxed),
            success_count: self.success_count.load(Ordering::Relaxed),
            last_failure: self.last_failure.try_read().ok().and_then(|g| *g),
        }
    }

    /// Attempt to transition from Open to HalfOpen if recovery timeout has elapsed.
    async fn maybe_trippy_open_to_half_open(&self) {
        if self.state() != State::Open {
            return;
        }

        let last = *self.last_failure.read().await;
        if let Some(t) = last {
            if t.elapsed() >= self.recovery_timeout {
                self.state.store(STATE_HALF_OPEN, Ordering::Release);
                self.half_open_remaining
                    .store(self.half_open_max, Ordering::Relaxed);
                tracing::info!("circuit breaker: OPEN -> HALF_OPEN (recovery timeout elapsed)");
            }
        }
    }

    /// Execute a closure through the circuit breaker.
    pub async fn call<F, Fut, T>(&self, f: F) -> Result<T, CircuitBreakerError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, CircuitBreakerError>>,
    {
        self.maybe_trippy_open_to_half_open().await;

        match self.state() {
            State::Open => {
                return Err(CircuitBreakerError::CircuitOpen);
            }
            State::HalfOpen => {
                let remaining = self.half_open_remaining.fetch_sub(1, Ordering::SeqCst);
                if remaining == 0 {
                    self.half_open_remaining.store(0, Ordering::Relaxed);
                    return Err(CircuitBreakerError::CircuitOpen);
                }
            }
            State::Closed => {}
        }

        match f().await {
            Ok(val) => {
                self.on_success().await;
                Ok(val)
            }
            Err(e) => {
                self.on_failure().await;
                Err(e)
            }
        }
    }

    /// Record a success (used externally for TCP connect outcomes).
    pub async fn on_success(&self) {
        self.success_count.fetch_add(1, Ordering::Relaxed);

        match self.state() {
            State::HalfOpen => {
                tracing::info!("circuit breaker: HALF_OPEN -> CLOSED (success)");
                self.state.store(STATE_CLOSED, Ordering::Release);
                self.failure_count.store(0, Ordering::Relaxed);
            }
            State::Closed => {
                self.failure_count.store(0, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    /// Record a failure (used externally for TCP connect outcomes).
    pub async fn on_failure(&self) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        *self.last_failure.write().await = Some(Instant::now());

        let failures = self.failure_count.load(Ordering::Relaxed);

        match self.state() {
            State::Closed => {
                if failures >= self.failure_threshold {
                    self.state.store(STATE_OPEN, Ordering::Release);
                    tracing::warn!(
                        "circuit breaker: CLOSED -> OPEN (failures={}, threshold={})",
                        failures,
                        self.failure_threshold
                    );
                }
            }
            State::HalfOpen => {
                self.state.store(STATE_OPEN, Ordering::Release);
                tracing::warn!("circuit breaker: HALF_OPEN -> OPEN (failure during probe)");
            }
            _ => {}
        }
    }

    /// Reset the circuit breaker to closed state (manual override).
    pub async fn reset(&self) {
        self.state.store(STATE_CLOSED, Ordering::Release);
        self.failure_count.store(0, Ordering::Relaxed);
        self.success_count.store(0, Ordering::Relaxed);
        *self.last_failure.write().await = None;
        tracing::info!("circuit breaker: reset to CLOSED");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_closed_allows_requests() {
        let cb = CircuitBreaker::new(3, 10, 1);
        let result = cb.call(|| async { Ok::<_, CircuitBreakerError>(42) }).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_opens_after_threshold() {
        let cb = CircuitBreaker::new(2, 60, 1);

        let err = CircuitBreakerError::UpstreamError("test".into());
        let _ = cb.call(|| async { Err::<i32, _>(err.clone()) }).await;
        assert_eq!(cb.state(), State::Closed);

        let err = CircuitBreakerError::UpstreamError("test".into());
        let _ = cb.call(|| async { Err::<i32, _>(err.clone()) }).await;
        assert_eq!(cb.state(), State::Open);

        let result = cb.call(|| async { Ok::<_, CircuitBreakerError>(0) }).await;
        assert!(matches!(result, Err(CircuitBreakerError::CircuitOpen)));
    }

    #[tokio::test]
    async fn test_half_open_after_timeout() {
        let cb = CircuitBreaker::new(1, 0, 1);

        let err = CircuitBreakerError::UpstreamError("test".into());
        let _ = cb.call(|| async { Err::<i32, _>(err.clone()) }).await;
        assert_eq!(cb.state(), State::Open);

        // recovery_timeout_secs=0, so next call triggers half-open
        let result = cb.call(|| async { Ok::<_, CircuitBreakerError>(99) }).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 99);
        assert_eq!(cb.state(), State::Closed);
    }

    #[tokio::test]
    async fn test_half_open_failure_reopens() {
        let cb = CircuitBreaker::new(1, 0, 1);

        let err = CircuitBreakerError::UpstreamError("test".into());
        let _ = cb.call(|| async { Err::<i32, _>(err.clone()) }).await;

        // Enters half-open, then fails -> re-opens
        let err = CircuitBreakerError::UpstreamError("test".into());
        let _ = cb.call(|| async { Err::<i32, _>(err.clone()) }).await;
        assert_eq!(cb.state(), State::Open);
    }

    #[tokio::test]
    async fn test_stats() {
        let cb = CircuitBreaker::new(5, 30, 3);
        let s = cb.stats();
        assert_eq!(s.state, State::Closed);
        assert_eq!(s.failure_count, 0);
        assert_eq!(s.success_count, 0);
    }
}
