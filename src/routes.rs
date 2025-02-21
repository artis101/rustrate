use crate::OutputFormat;
use crate::state::{AppEvent, AppState, RequestLog};
use anyhow::anyhow;
use axum::{
    extract::{OriginalUri, State},
    http::StatusCode,
    response::Response,
};
use serde_json::json;

use rand::Rng;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub(crate) struct DelayConfig {
    min: u64,
    max: u64,
}

impl DelayConfig {
    pub fn parse(delay_str: &str) -> anyhow::Result<Self> {
        if delay_str.contains('-') {
            let parts: Vec<&str> = delay_str.split('-').collect();
            if parts.len() != 2 {
                return Err(anyhow!("Invalid delay range format. Expected 'min-max'"));
            }
            let min = parts[0]
                .parse::<u64>()
                .map_err(|_| anyhow!("Invalid minimum delay value"))?;
            let max = parts[1]
                .parse::<u64>()
                .map_err(|_| anyhow!("Invalid maximum delay value"))?;
            if min >= max {
                return Err(anyhow!("Minimum delay must be less than maximum delay"));
            }
            Ok(Self { min, max })
        } else {
            let delay = delay_str
                .parse::<u64>()
                .map_err(|_| anyhow!("Invalid delay value"))?;
            Ok(Self {
                min: delay,
                max: delay,
            })
        }
    }

    pub fn get_delay(&self) -> u64 {
        if self.min == self.max {
            self.min
        } else {
            rand::rng().random_range(self.min..=self.max)
        }
    }
}

/// A fallback handler that catches all requests
pub async fn request_handler(
    State(state): State<AppState>,
    uri: OriginalUri,
    method: axum::http::Method,
) -> Response<String> {
    let start = std::time::Instant::now();
    let now = state.now_timestamp();
    state.increment_requests();

    // Get the configured delay
    let delay_ms = state.get_delay();
    if delay_ms > 0 {
        // Simulate delay
        sleep(Duration::from_millis(delay_ms)).await;
    }

    // Build a simple log record
    let elapsed = start.elapsed();
    let duration_ms = elapsed.as_secs_f64() * 1000.0;

    let log = RequestLog {
        path: uri.0.path().to_string(),
        method: method.to_string(),
        status: 200,
        timestamp: now,
        duration_ms,
    };

    // Send an event to the TUI
    let _ = state.tx.send(AppEvent::RequestReceived(log)).await;

    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Format response based on output format
    let response_body = match state.output_format {
        OutputFormat::Json => json!({
            "status": "success",
            "request": {
                "path": uri.0.path(),
                "method": method.to_string(),
                "timestamp": now
            },
            "timing": {
                "processing_time_ms": elapsed_ms,
                "simulated_delay_ms": delay_ms
            }
        })
        .to_string(),
        OutputFormat::Text => format!(
            "Request processed in {}ms (simulated delay: {}ms)",
            elapsed_ms, delay_ms
        ),
    };

    // Return 200 OK
    Response::builder()
        .status(StatusCode::OK)
        .header(
            "content-type",
            match state.output_format {
                OutputFormat::Json => "application/json",
                OutputFormat::Text => "text/plain",
            },
        )
        .body(response_body)
        .unwrap()
}
