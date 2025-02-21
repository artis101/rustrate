# rustrate 🚀

A high-performance HTTP client performance testing tool.

It mimics real-world request handling while tracking throughput in real time.
You can easily benchmark and stress-test systems handling heavy HTTP traffic.

This project follows semantic versioning.

![rustrate demo video](https://github.com/artis101/rustrate/blob/main/preview.gif?raw=true "demo of rustrate in action handling a lot of requests")

## Features

- Handles HTTP requests and simulates real-world response delays.
- Tracks requests per second (RPS), min/max/avg response times, and uptime.
- Provides a **TUI dashboard** to visualize performance in real-time.
- Supports **configurable delays** (`0`, `min-max`) to simulate latency.
- JSON and plaintext output formats.

## Why?

Sometimes you need to test your service that makes HTTP requests. rustrate is a tool to help you do that. It's like a **microbenchmark** for HTTP clients. Use it to test your service's performance under various conditions. How scalable your solution really is?

## Installation

You'll need **Rust** (duh). Then:

```sh
cargo install --path .
```

or just run it directly:

```sh
cargo run -- --run
```

## Usage

Run the server with:

```sh
rustrate -p 31337 -d 30-150 -f json --run
```

Options:

- -p, --port <PORT>: Set the port (default: 31337).
- -d, --delay <DELAY>: Simulate delay (e.g., 50 or 30-150 for range).
- -f, --format <FORMAT>: Output format (json, text).
- -r, --run: Start the server (otherwise, just prints help).

## Example

```sh
rustrate -p 31337 -d 30-150 -f json --run
```

```sh
curl -X POST curl -X POST http://localhost:31337
```

Load test it:

```sh
wrk -t8 -c100 -d90s http://localhost:31337
```

## Interactive TUI

- Live stats: RPS, min/max/avg delay, total requests.
- Real-time graph of the last 60 seconds of throughput.
- Logs of recent requests.
- Press 'q' to quit or send SIGINT(Ctrl+C) to exit.

## Internals

- Built with Axum for the HTTP server.
- Uses tokio for async processing.
- ratatui for the TUI dashboard.
- Request logs and stats are sent to a channel and rendered in real time.

## License

MIT. Use it, break it, improve it.
