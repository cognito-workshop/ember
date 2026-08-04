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
      --key <FILE>        TLS private key
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

## License

MIT
