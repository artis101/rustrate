use std::collections::VecDeque;
use std::io::stdout;
use std::time::{Duration, Instant};

use chrono::Utc;
use crossterm::event::KeyModifiers;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    symbols,
    text::Span,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph},
};

use tokio::sync::mpsc::Receiver;

use crate::state::{AppEvent, RequestLog};

/// Maximum number of logs to store
const MAX_LOGS: usize = 1000;

/// Data that the TUI thread holds locally
struct TuiData {
    /// Ring buffer of the most recent logs
    logs: VecDeque<RequestLog>,

    /// The last 60 seconds of RPS counts for calculation (index 0 = "now")
    rps_history: [u64; 60],

    /// The last 60 seconds of RPS counts for display.
    /// When a second ticks and no new request comes in, this holds the previous value
    /// so that the graph doesn't jump to 0.
    rps_display_history: [u64; 60],

    /// The timestamp (in seconds) for the last time we updated the RPS arrays
    last_rps_update: i64,

    /// The total number of requests
    total_requests: u64,

    /// The server's start time, for calculating uptime
    start_time: Instant,

    /// The server port, for display
    port: u16,

    /// Current request delay in milliseconds
    current_delay: f64,

    /// Minimum request delay seen in milliseconds
    min_delay: f64,

    /// Maximum request delay seen in milliseconds
    max_delay: f64,

    /// Total delay for calculating average
    total_delay: f64,

    /// Number of delay samples for calculating average
    delay_samples: u64,
}

impl TuiData {
    fn new(start_time: Instant, port: u16) -> Self {
        Self {
            logs: VecDeque::with_capacity(MAX_LOGS),
            rps_history: [0; 60],
            rps_display_history: [0; 60],
            last_rps_update: 0,
            total_requests: 0,
            start_time,
            port,
            current_delay: 0.0,
            min_delay: f64::MAX,
            max_delay: 0.0,
            total_delay: 0.0,
            delay_samples: 0,
        }
    }

    /// Add a new request log and update counters.
    fn push_log(&mut self, log: RequestLog) {
        self.total_requests += 1;
        if self.logs.len() == MAX_LOGS {
            self.logs.pop_front();
        }

        // Update delay statistics
        let delay = log.duration_ms;
        self.current_delay = delay;
        self.min_delay = self.min_delay.min(delay);
        self.max_delay = self.max_delay.max(delay);
        self.total_delay += delay;
        self.delay_samples += 1;

        self.logs.push_back(log);
    }

    /// Get the minimum request delay seen in milliseconds
    fn get_min_delay(&self) -> f64 {
        if self.min_delay == f64::MAX {
            0.0
        } else {
            self.min_delay
        }
    }

    /// Get the maximum request delay seen in milliseconds
    fn get_max_delay(&self) -> f64 {
        self.max_delay
    }

    /// Get the average request delay in milliseconds
    fn get_avg_delay(&self) -> f64 {
        if self.delay_samples == 0 {
            0.0
        } else {
            self.total_delay as f64 / self.delay_samples as f64
        }
    }

    /// Update the RPS data.
    ///
    /// For the calculation array we shift and clear new slots to 0.
    /// For the display array we shift and fill new slots with the last known value
    /// so that the graph doesn’t jump to 0.
    fn update_rps(&mut self, now: i64) {
        if self.last_rps_update == 0 {
            self.last_rps_update = now;
            return;
        }

        let diff = now - self.last_rps_update;
        if diff <= 0 {
            return;
        }

        let diff = diff.min(60) as usize;
        if diff >= 60 {
            self.rps_history = [0; 60];
            self.rps_display_history = [0; 60];
        } else {
            // For calculation: shift right and set new slots to 0.
            self.rps_history.copy_within(0..60 - diff, diff);
            for i in 0..diff {
                self.rps_history[i] = 0;
            }

            // For display: shift right and set new slots to 0.
            self.rps_display_history.copy_within(0..60 - diff, diff);
            for i in 0..diff {
                self.rps_display_history[i] = 0;
            }
        }

        self.last_rps_update = now;
    }

    /// Increment the RPS counter for the current second in both arrays.
    fn increment_rps(&mut self) {
        self.rps_history[0] += 1;
        self.rps_display_history[0] = self.rps_history[0];
    }

    /// Return the server uptime in seconds.
    fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Compute statistics for the RPS over the last 60 seconds.
    /// Returns (min, max, average, median, 95th percentile).
    fn compute_rps_stats(&self) -> (u64, u64, f64, u64, u64) {
        // Exclude the current second from the calculation
        let mut data: Vec<u64> = self.rps_history[1..].to_vec();
        data.sort_unstable();
        let greater_than_zero: Vec<u64> = data.iter().filter(|&&x| x > 0).cloned().collect();
        // min excludes 0 values
        let min = *greater_than_zero.first().unwrap_or(&0);
        let max = *data.last().unwrap();
        let sum: u64 = data.iter().sum();
        let count = data.len() as f64;
        let avg = sum as f64 / count;
        let median = if data.len() % 2 == 1 {
            data[data.len() / 2]
        } else {
            let mid = data.len() / 2;
            (data[mid - 1] + data[mid]) / 2
        };
        let idx_90 = ((data.len() as f64) * 0.90).ceil() as usize;
        let idx_90 = if idx_90 >= data.len() {
            data.len() - 1
        } else {
            idx_90
        };
        let p90 = data[idx_90];
        (min, max, avg, median, p90)
    }
}

/// Main TUI function (runs in a blocking thread)
///
/// Receives `AppEvent` messages on `rx` and updates the TUI accordingly.
pub fn run_tui(mut rx: Receiver<AppEvent>, port: u16) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let start_time = Instant::now();
    let mut data = TuiData::new(start_time, port);

    let tick_rate = Duration::from_millis(200);
    let mut last_tick = Instant::now();

    loop {
        let now_ts = Utc::now().timestamp();
        data.update_rps(now_ts);

        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::RequestReceived(log) => {
                    data.push_log(log);
                    data.increment_rps();
                }
                // disable warning for unreachable pattern
                #[allow(unreachable_patterns)]
                _ => (),
            }
        }

        terminal.draw(|frame| {
            draw_ui(frame, &data);
        })?;

        if crossterm::event::poll(Duration::from_millis(1))? {
            if let CEvent::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q')
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    break;
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

/// Draw the entire UI layout.
fn draw_ui<B: ratatui::backend::Backend>(frame: &mut ratatui::Frame<B>, data: &TuiData) {
    // Vertical layout: top (stats), middle (chart), bottom (logs)
    let vertical_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(40),
            Constraint::Percentage(30),
        ])
        .split(frame.size());

    // Top area split into three columns: RPS Stats, Delay Stats, and Server Stats.
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(34),
            Constraint::Percentage(33),
        ])
        .split(vertical_chunks[0]);

    // Left widget: Detailed RPS statistics.
    let (rps_min, rps_max, rps_avg, rps_median, rps_p90) = data.compute_rps_stats();
    let rps_stats_text = format!(
        "Min RPS: {}\nMax RPS: {}\nAvg RPS: {:.2}\nMedian RPS: {}\n90th Percentile: {}",
        rps_min, rps_max, rps_avg, rps_median, rps_p90
    );
    let rps_stats_paragraph = Paragraph::new(rps_stats_text)
        .block(Block::default().borders(Borders::ALL).title("RPS Stats"));
    frame.render_widget(rps_stats_paragraph, top_chunks[0]);

    // Middle widget: Delay statistics.
    let delay_stats_text = format!(
        "Min Delay: {:.3} ms\nMax Delay: {:.3} ms\nAvg Delay: {:.3} ms",
        data.get_min_delay(),
        data.get_max_delay(),
        data.get_avg_delay()
    );
    let delay_stats_paragraph = Paragraph::new(delay_stats_text)
        .block(Block::default().borders(Borders::ALL).title("Delay Stats"));
    frame.render_widget(delay_stats_paragraph, top_chunks[1]);

    // Right widget: General server stats.
    let server_stats_text = format!(
        "Uptime: {}s\nTotal Requests: {}\nURL: http://localhost:{}",
        data.uptime_seconds(),
        data.total_requests,
        data.port
    );
    let server_stats_paragraph = Paragraph::new(server_stats_text)
        .block(Block::default().borders(Borders::ALL).title("Server Stats"));
    frame.render_widget(server_stats_paragraph, top_chunks[2]);

    // RPS chart in the middle remains similar.
    let chart_data: Vec<(f64, f64)> = data.rps_display_history[1..]
        .iter()
        .enumerate()
        .map(|(i, &count)| (i as f64, count as f64))
        .collect();

    let datasets = vec![
        Dataset::default()
            .name("RPS")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Green))
            .data(&chart_data),
    ];

    let max_rps = chart_data.iter().map(|(_x, y)| *y).fold(0.0, f64::max);
    let y_max = ((max_rps * 1.2).ceil() / 10.0).ceil() * 10.0;
    let y_max = y_max.max(10.0);

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("RPS (Last 60s)"),
        )
        .x_axis(
            Axis::default()
                .title(Span::raw("Seconds Ago"))
                .style(Style::default().fg(Color::Gray))
                .bounds([0.0, 59.0])
                .labels(vec!["0".into(), "30".into(), "59".into()]),
        )
        .y_axis(
            Axis::default()
                .title(Span::raw("Count"))
                .style(Style::default().fg(Color::Gray))
                .bounds([0.0, y_max])
                .labels(vec![
                    "0".into(),
                    format!("{}", y_max / 2.0).into(),
                    format!("{}", y_max).into(),
                ]),
        );
    frame.render_widget(chart, vertical_chunks[1]);

    // Logs panel remains unchanged.
    let logs_text: String = data
        .logs
        .iter()
        .rev()
        .take(20)
        .map(|log| {
            let timestamp = chrono::DateTime::<Utc>::from_timestamp(log.timestamp, 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            let status_text = format!("[{}]", log.status);
            format!(
                "{} {} {} {} ({:.3} ms)",
                timestamp, status_text, log.method, log.path, log.duration_ms
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let logs_paragraph =
        Paragraph::new(logs_text).block(Block::default().borders(Borders::ALL).title("Logs"));
    frame.render_widget(logs_paragraph, vertical_chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RequestLog;
    use chrono::Utc;
    use std::time::Instant;

    #[test]
    fn test_push_log_updates_stats() {
        let start = Instant::now();
        let mut data = TuiData::new(start, 8080);
        let now = Utc::now().timestamp();
        let log = RequestLog {
            path: "/test".to_string(),
            method: "GET".to_string(),
            status: 200,
            timestamp: now,
            duration_ms: 120.0,
        };
        data.push_log(log);
        assert_eq!(data.total_requests, 1);
        assert_eq!(data.get_min_delay(), 120.0);
        assert_eq!(data.get_max_delay(), 120.0);
        assert_eq!(data.get_avg_delay(), 120.0);
        assert_eq!(data.logs.len(), 1);
    }

    #[test]
    fn test_update_rps_shifts_history() {
        let start = Instant::now();
        let mut data = TuiData::new(start, 8080);
        // Simulate some RPS count at the current second.
        data.rps_history[0] = 5;
        data.rps_display_history[0] = 5;
        // Simulate that 10 seconds have passed.
        let now = Utc::now().timestamp();
        data.last_rps_update = now - 10;
        data.update_rps(now);
        // After shifting, the previous count should now be at index 10.
        assert_eq!(data.rps_history[10], 5);
        // And the first 10 indices should be reset to 0.
        for i in 0..10 {
            assert_eq!(data.rps_history[i], 0);
        }
    }

    #[test]
    fn test_compute_rps_stats() {
        let start = Instant::now();
        let mut data = TuiData::new(start, 8080);
        // Manually set rps_history for indices 1.. (index 0 is the current second and ignored)
        // Here we simulate sample RPS counts; non-zero values: 5, 3, 8, 2, 7, 4, 6.
        data.rps_history = [
            0, 5, 3, 8, 2, 7, 1, 4, 4, 6, 8, 7, 2, 2, 3, 5, 4, 4, 7, 1, 7, 5, 9, 9, 8, 9, 5, 9, 2,
            7, 6, 8, 1, 1, 2, 8, 7, 4, 2, 7, 11, 6, 6, 5, 6, 2, 3, 2, 8, 7, 1, 5, 7, 3, 4, 5, 6, 5,
            5, 3,
        ];
        let (min, max, avg, median, p90) = data.compute_rps_stats();
        // After sorting non-zero values: 2, 3, 4, 5, 6, 7, 8.
        assert_eq!(min, 1);
        assert_eq!(max, 11);
        assert_eq!(median, 5);
        assert_eq!(avg.round() as u64, 5);
        assert_eq!(p90, 9);
        // The 90th percentile (p90) should lie between the median and max.
        assert!(p90 >= median && p90 <= max);
    }
}
