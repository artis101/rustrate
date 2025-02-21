use crate::OutputFormat;
use crate::routes::DelayConfig;
use chrono::Utc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc::Sender;

/// A log of one request
#[derive(Debug, Clone)]
pub struct RequestLog {
    pub path: String,
    pub method: String,
    pub status: u16,
    pub timestamp: i64,   // Unix timestamp
    pub duration_ms: f64, // Request duration in milliseconds with nanosecond precision
}

/// Events that the server sends to the TUI
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A new request was received
    RequestReceived(RequestLog),
}

/// Shared application state for Axum
#[derive(Clone)]
pub struct AppState {
    pub total_requests: Arc<AtomicU64>,
    pub tx: Sender<AppEvent>,
    delay_config: Arc<DelayConfig>,
    pub output_format: OutputFormat,
}

impl AppState {
    pub fn new(
        tx: Sender<AppEvent>,
        delay_str: &str,
        output_format: OutputFormat,
    ) -> anyhow::Result<Self> {
        let delay_config = DelayConfig::parse(delay_str)?;
        Ok(Self {
            total_requests: Arc::new(AtomicU64::new(0)),
            tx,
            delay_config: Arc::new(delay_config),
            output_format,
        })
    }

    /// Increment the request counter
    pub fn increment_requests(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Return the current Unix timestamp
    pub fn now_timestamp(&self) -> i64 {
        Utc::now().timestamp()
    }

    /// Get the delay for the current request
    pub fn get_delay(&self) -> u64 {
        self.delay_config.get_delay()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OutputFormat;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_app_state_new() {
        let (tx, _rx) = mpsc::channel(10);
        let state = AppState::new(tx, "100", OutputFormat::Json).unwrap();
        // Verify that the total_requests counter starts at 0.
        assert_eq!(
            state
                .total_requests
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn test_now_timestamp() {
        let (tx, _rx) = mpsc::channel(10);
        let state = AppState::new(tx, "100", OutputFormat::Json).unwrap();
        let now = state.now_timestamp();
        // Check that the timestamp is reasonably close to the current UTC time.
        let current = chrono::Utc::now().timestamp();
        assert!((now - current).abs() < 2);
    }
}
