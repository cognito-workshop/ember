# Ember

**A high-performance Wisp v1/v2 proxy server written in Rust.**

## Features

- Wisp v1 + v2 protocol (auto-detected)
- TCP and UDP proxying over WebSocket
- Thread-per-core runtime (Linux with SO_REUSEPORT)
- Adaptive buffer sizing (tunes to traffic patterns)
- Plugin system with built-in plugins
- Prometheus /metrics endpoint
- Auth extensions (password + Ed25519 key)
- Hot reload config (SIGHUP)
- Graceful shutdown (SIGINT/SIGTERM)

## Quick Start

```bash
# Build
cargo build --release

# Run (multi-thread mode)
./target/release/ember -p 8080

# Run (thread-per-core, Linux)
./target/release/ember --thread-per-core -p 8080

# Or with Docker
docker build -t ember .
docker run -p 8080:8080 -p 9090:9090 ember
```

## Configuration

Create an `ember.toml`:

```toml
[server]
host = "0.0.0.0"
port = 8080
max_connections = 10000
metrics_port = 9090

[buffer]
initial_size = 128
min_size = 32
max_size = 1024

[extensions]
udp = true
motd = "Welcome to Ember"

[plugins]
logger = true

[plugins.rate_limiter]
max_connections_per_ip = 100
window_secs = 60

[auth]
# password = "secret"
# public_keys = ["base64-ed25519-key..."]
```

CLI flags override config file values.

## CLI Options

```
ember [OPTIONS]
  -c, --config <FILE>     Config file path
  -h, --host <HOST>       Listen address
  -p, --port <PORT>       Listen port
      --thread-per-core   Use thread-per-core runtime (Linux)
      --tls               Enable TLS
      --cert <FILE>       TLS certificate
      --key <FILE>       TLS private key
  -v, --verbose           Debug logging
```

## Plugin System

Ember has a plugin system with lifecycle hooks:
- `ConnectionOpen` -- new WebSocket connection
- `StreamOpen` -- new TCP/UDP stream
- `DataTransfer` -- bytes flowing
- `StreamClose` / `ConnectionClose` -- cleanup
- `Shutdown` -- server stopping

### Built-in Plugins

| Plugin | Description |
|--------|-------------|
| RateLimiter | Per-IP connection limiting |
| ConnectionLimiter | Global max connections |
| Metrics | Atomic counters for monitoring |
| Logger | Logs all plugin events |

## Metrics

```bash
curl http://localhost:9090/metrics
```

Returns Prometheus-format metrics:
- `ember_connections_active`
- `ember_connections_total`
- `ember_streams_active`
- `ember_streams_total`
- `ember_bytes_received_total`
- `ember_bytes_sent_total`

## Benchmarks

Tested on 2-core Linux VPS:
- 1 stream: ~500 MiB/s (flood benchmark)
- 10 streams: ~480 MiB/s
- Latency P50: ~46 us

---

# WispMark

WispMark is a benchmarking tool for Wisp protocol implementations.

## Installation

To run this repository, install the Xonsh shell, and all the dependencies for the Wisp implementations.

You need:
- Git
- iftop
- net-tools
- NodeJS
- CPython
- Rust Nightly
- GCC
- Go

You must also be on a recent Linux distribution. Debian 13 and Arch Linux have been tested to work.

Run `./wispmark.xsh` to start the tests. If you don't already have Xonsh installed, run `./wispmark.sh` which is a wrapper that will install Xonsh in a Python virtual environment.

Note: If you want to rebuild all of the server and client implementations to run a clean test, you can run: `git clean -ffXd`

## Ember Benchmarking

This repository includes ember in the WispMark benchmark suite. The `EmberWispServer` and `EmberThreadPerCoreWispServer` classes in `server.xsh` will automatically build ember from source and include it in benchmarks.

To test ember standalone (without full wispmark suite):

```bash
# Build ember
cargo build --release

# Run ember server
./target/release/ember -p 8080
```

## Methodology

This program pairs each Wisp server with each Wisp client, with a TCP echo server running on port 6002. The amount of traffic passing through that port is used to calculate the bandwidth that was achieved with each configuration.

### Implementations Tested

Server:
- [wisp-server-python](https://github.com/MercuryWorkshop/wisp-server-python)
- [wisp-js/server](https://github.com/MercuryWorkshop/wisp-js/blob/master/src/server)
- [epoxy-server](https://github.com/MercuryWorkshop/epoxy-tls/tree/multiplexed/server) (Rust)
- [mrrowisp](https://github.com/starlightdevgroup/mrrowisp) (Go)
- **ember** (Rust) - high-performance Wisp server

Client:
- [wisp-js/client](https://github.com/MercuryWorkshop/wisp-js/blob/master/src/client)
- [wisp-mux](https://github.com/MercuryWorkshop/epoxy-tls/tree/multiplexed/simple-wisp-client) (Rust)

## Usage

```
./wispmark.xsh [-h] [--duration DURATION] [--output OUTPUT] [--print-md]

A benchmarking tool for Wisp protocol implementations.

options:
  -h, --help           show this help message and exit
  --duration DURATION  The duration of each test, in seconds. The default is 10s.
  --output OUTPUT      The file to use for output after test results are complete. The default is wispmark-results.md.
  --print-md           Print a markdown table after test results are complete.
```

## License

Ember is MIT licensed.

WispMark is licensed under the GNU GPL v3.

```
WispMark: A benchmarking tool for Wisp protocol implementations.
Copyright (C) 2025 ading2210
```
