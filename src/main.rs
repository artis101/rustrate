use anyhow::Result;
use clap::Parser;
use std::net::SocketAddr;
use tokio::signal;
use tokio::sync::mpsc;

mod routes;
mod state;
mod tui;

use crate::routes::request_handler;
use crate::state::{AppEvent, AppState};
use crate::tui::run_tui;

// ASCII banner
const BANNER: &str = r#"
                _             _       
               | |           | |      
 _ __ _   _ ___| |_ _ __ __ _| |_ ___ 
| '__| | | / __| __| '__/ _` | __/ _ \
| |  | |_| \__ \ |_| | | (_| | ||  __/
|_|   \__,_|___/\__|_|  \__,_|\__\___|

"#;

// Updated long description to match the README
const LONG_ABOUT: &str = r#"A high-performance HTTP server performance testing tool.
It mimics real-world request handling while tracking throughput in real time.
You can easily benchmark and stress-test systems handling heavy HTTP traffic."#;

const USAGE: &str = r#"Interactive TUI dashboard shows key stats — like requests per second, plus statistics
like avg/min/max/median throughput.

Press 'q' in the TUI to quit or send SIGINT (Ctrl+C) to quit.

Usage:
    rustrate [OPTIONS]
Options:
    -p, --port <PORT>      The port number to listen on (default: 31337)
    -d, --delay <DELAY>    The delay in milliseconds for each request (default: 0)
                           You can specify a range using 'min-max' format (e.g., 30-150)
    -f, --format <FORMAT>  The HTTP response output format (default: json)
                           Valid formats: json, text
    -r, --run              Run the server (if not set, only shows help)
    -h, --help             Print help information
    -V, --version          Print version information
"#;

/// Command-line arguments
#[derive(Parser, Debug)]
#[command(
    author = "artiscode",
    version = "1.0.0",
    // Updated short description to match the README
    about = "rustrate is a high-performance HTTP client performance testing tool written in Rust.",
    long_about = LONG_ABOUT,
)]
struct Args {
    /// The port to listen on
    #[arg(
        short,
        long,
        default_value_t = 31337,
        help = "The port number to listen on (default: 31337)"
    )]
    port: u16,

    /// The delay in milliseconds for each request
    #[arg(
        short,
        long,
        default_value = "0",
        help = "The delay in milliseconds for each request (default: 0). You can specify a range using 'min-max' format (e.g., 30-150)"
    )]
    delay: String,

    /// The output format for HTTP responses
    #[arg(
        short,
        long,
        default_value = "json",
        help = "The HTTP response output format (default: json). Valid formats: json, text"
    )]
    format: OutputFormat,

    /// Run the server (if not set, only shows help)
    #[arg(short, long)]
    run: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Json,
    Text,
}

impl std::str::FromStr for OutputFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "json" => Ok(OutputFormat::Json),
            "text" => Ok(OutputFormat::Text),
            _ => Err(anyhow::anyhow!("Invalid format. Valid formats: json, text")),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if !args.run {
        println!("{}", BANNER);
        println!("{}", LONG_ABOUT);
        println!("{}", USAGE);
        return Ok(());
    }

    let port = args.port;

    // Create a channel for sending request events to the TUI
    let (tx, rx) = mpsc::channel::<AppEvent>(1024);

    // Build our shared (atomic) state
    let state = AppState::new(tx.clone(), &args.delay, args.format)?;

    // Build our Axum router
    let app = axum::Router::new()
        // Catch all paths, any method
        .fallback(request_handler)
        .with_state(state.clone());

    // Prepare server
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let server = axum::Server::bind(&addr).serve(app.into_make_service());

    println!(
        "Server listening on http://{} (press 'q' in TUI or Ctrl+C to quit)",
        addr
    );

    // Graceful shutdown signal
    let shutdown_signal = async {
        // Wait for Ctrl+C
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
        println!("Received Ctrl+C, shutting down...");
    };

    // Run server with shutdown
    let server_handle = tokio::spawn(async move {
        if let Err(err) = server.with_graceful_shutdown(shutdown_signal).await {
            eprintln!("Server error: {}", err);
        }
    });

    // Spawn the TUI in a blocking thread via tokio
    let tui_handle = tokio::spawn(async move {
        // We'll run the TUI in a blocking context
        // because crossterm + ratatui are synchronous
        tokio::task::spawn_blocking(move || run_tui(rx, port))
            .await
            .expect("Failed to run TUI blocking task")?;
        Ok::<(), anyhow::Error>(())
    });

    // If either task finishes, we exit
    tokio::select! {
        _ = server_handle => { /* server finished or crashed */ }
        _ = tui_handle => { /* TUI finished */ }
    }

    Ok(())
}
