//! Exponential-backoff retry for transient transport failures (Codex /
//! Geofabrik).
//!
//! The storage and Geofabrik clients talk to network endpoints that can drop
//! a connection, time out, or return a transient 5xx/429 under load. A
//! single-shot client surfaces those as hard failures; this module retries
//! the transient ones a bounded number of times with exponential backoff,
//! and surfaces a clear error only after exhaustion. Non-transient errors (a
//! 400 bad request, a genuinely-absent 404 after propagation retries) are
//! returned immediately — retrying them would only burn time.
//!
//! Deterministic schedule (no jitter): 200ms → 400ms → 800ms …, capped at
//! 5s, max 4 attempts. (Proven in the vault build; reused verbatim.)

use std::future::Future;
use std::time::Duration;

use anyhow::Result;

/// Max attempts (one try plus retries).
pub const MAX_ATTEMPTS: u32 = 4;
const BASE_DELAY: Duration = Duration::from_millis(200);
const CAP_DELAY: Duration = Duration::from_secs(5);

/// A classified error from a retried operation.
#[derive(Debug)]
pub enum RetryErr {
    /// A transient failure (connection reset, timeout, 5xx, 429) — retry.
    Transient(anyhow::Error),
    /// A permanent failure — return it now, do not retry.
    Fatal(anyhow::Error),
}

/// Run `op` up to [`MAX_ATTEMPTS`] times, retrying [`RetryErr::Transient`]
/// failures with exponential backoff. [`RetryErr::Fatal`] returns immediately.
pub async fn retry_transient<T, F, Fut>(mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, RetryErr>>,
{
    let mut delay = BASE_DELAY;
    let mut last: Option<anyhow::Error> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match op().await {
            Ok(value) => return Ok(value),
            Err(RetryErr::Fatal(e)) => return Err(e),
            Err(RetryErr::Transient(e)) => {
                last = Some(e);
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(CAP_DELAY);
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("retry exhausted with no error")))
}

/// Is an HTTP status worth retrying? 5xx, 408 (request timeout), 429 (rate
/// limit) are transient; other 4xx are not.
pub fn status_is_transient(code: u16) -> bool {
    code >= 500 || code == 408 || code == 429
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn succeeds_on_second_attempt() {
        let calls = AtomicU32::new(0);
        let res: Result<u32> = retry_transient(|| {
            let c = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if c == 0 {
                    Err(RetryErr::Transient(anyhow::anyhow!("boom")))
                } else {
                    Ok(42)
                }
            }
        })
        .await;
        assert_eq!(res.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn fatal_returns_immediately() {
        let calls = AtomicU32::new(0);
        let res: Result<u32> = retry_transient(|| {
            let c = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                let _ = c;
                Err(RetryErr::Fatal(anyhow::anyhow!("nope")))
            }
        })
        .await;
        assert!(res.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn exhausts_after_max_attempts() {
        let calls = AtomicU32::new(0);
        let res: Result<u32> = retry_transient(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(RetryErr::Transient(anyhow::anyhow!("boom"))) }
        })
        .await;
        assert!(res.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), MAX_ATTEMPTS);
    }

    #[test]
    fn status_classification() {
        assert!(status_is_transient(500));
        assert!(status_is_transient(503));
        assert!(status_is_transient(408));
        assert!(status_is_transient(429));
        assert!(!status_is_transient(400));
        assert!(!status_is_transient(403));
    }
}
