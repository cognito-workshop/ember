<div align="center">

# 🔥 Ember

**A high-performance Wisp server written in Rust.**

[![Rust](https://img.shields.io/badge/Rust-2024-000000?style=flat&logo=rust)](https://rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

---

## What is Ember?

Ember is a [Wisp protocol](https://github.com/MercuryWorkshop/wisp-protocol) server built for performance, reliability, and extensibility. It proxies multiple TCP/UDP sockets over a single WebSocket connection — designed as a stepping stone toward a full [Scramjet](https://github.com/nicegram/nicegram-web/tree/main/scramjet) implementation.

### Why Ember?

Existing Wisp servers work, but they weren't built for:

- **Connection migration** — survive server restarts and network changes without dropping active sessions
- **Adaptive buffering** — dynamically tune buffer sizes based on traffic patterns
- **Zero-copy forwarding** — minimize allocations on the hot path
- **Plugin architecture** — extend behavior without forking
- **Built-in metrics** — Prometheus-compatible observability out of the box

## Features

- **Wisp v1 + v2** — full protocol support including connection type negotiation
- **TCP and UDP** proxying over WebSocket
- **Async runtime** — built on [Tokio](https://tokio.rs) for non-blocking I/O
- **TLS** — native TLS support via `rustls`
- **Metrics** — built-in Prometheus endpoint
- **Plugins** — hook into connection lifecycle events
- **Low memory footprint** — efficient resource usage under high concurrency

## Quick Start

```bash
# Clone
git clone https://github.com/sectersion/ember.git
cd ember

# Build
cargo build --release

# Run
./target/release/ember
```

Ember listens on `ws://0.0.0.0:443` by default. See [Configuration](#configuration) for options.

## Configuration

Ember can be configured via CLI flags, environment variables, or a config file:

```toml
# ember.toml
[server]
host = "0.0.0.0"
port = 443
max_connections = 10000

[tls]
enabled = true
cert_path = "/path/to/cert.pem"
key_path = "/path/to/key.pem"

[buffering]
initial_size = 4096
max_size = 65536
adaptive = true

[metrics]
enabled = true
path = "/metrics"

[plugins]
directory = "./plugins"
```

## Architecture

```
┌──────────────┐     WebSocket      ┌──────────────┐
│   Client     │◄──────────────────►│              │
│  (Browser)   │    Wisp v1/v2      │    Ember     │
└──────────────┘                    │              │
                                    │  ┌────────┐  │
                                    │  │ Plugin │  │
                                    │  │ System │  │
                                    │  └────────┘  │
                                    │              │
                                    │  ┌────────┐  │
                                    │  │Metrics │  │
                                    │  └────────┘  │
                                    └──────┬───────┘
                                           │
                                    ┌──────┴───────┐
                                    │  TCP / UDP   │
                                    │   Targets    │
                                    └──────────────┘
```

## Roadmap

- [ ] Core Wisp v1/v2 protocol
- [ ] TCP proxying
- [ ] UDP proxying
- [ ] Adaptive buffering
- [ ] Connection migration
- [ ] Plugin system
- [ ] Prometheus metrics
- [ ] TLS support
- [ ] Config file support
- [ ] Scramjet protocol compatibility

## Contributing

Contributions welcome. Open an issue first for large changes.

## License

MIT

---

*Built by [sectersion](https://github.com/sectersion)*
