# Ember — High-Performance Wisp Server

A Wisp v1/v2 server written in Rust. Built for maximum throughput.

---

## Part 1: Wisp Protocol Specification (v2.1)

Based on the canonical spec from [MercuryWorkshop/wisp-protocol](https://github.com/MercuryWorkshop/wisp-protocol).

### Overview

Wisp is a low-overhead protocol for proxying multiple TCP/UDP sockets over a single WebSocket connection.

### Packet Format

All packets share the same header. All data types are little-endian. Strings are never null-terminated.

| Field Name  | Field Type | Notes                                         |
|-------------|------------|-----------------------------------------------|
| Packet Type | `uint8_t`  | The packet type.                              |
| Stream ID   | `uint32_t` | Random stream ID assigned by the client.      |
| Payload     | `char[]`   | Payload takes up the rest of the packet.      |

Stream ID `0` is reserved for the initial handshake and must not be used elsewhere.

### Packet Types

#### `0x01` — CONNECT

**Payload:**

| Field Name           | Field Type | Notes                                                  |
|----------------------|------------|--------------------------------------------------------|
| Stream Type          | `uint8_t`  | `0x01` = TCP, `0x02` = UDP.                            |
| Destination Port     | `uint16_t` | Destination TCP/UDP port.                              |
| Destination Hostname | `char[]`   | Destination hostname (UTF-8).                          |

**Behavior:**
- Client sends CONNECT to create a new stream under the same WebSocket.
- The stream ID chosen is associated with this stream for all future messages.
- Server validates the payload; if invalid, sends a CLOSE packet.
- Server immediately attempts to establish a TCP/UDP socket to the destination.
- Client may begin sending DATA before receiving a CONTINUE from the server.
- UDP support is optional for both server and client.

#### `0x02` — DATA

**Payload:**

| Field Name     | Field Type | Notes                                         |
|----------------|------------|-----------------------------------------------|
| Stream Payload | `char[]`   | Data sent to/from the destination server.     |

**Behavior:**
- Client → Server: payload is proxied to the TCP/UDP socket for the stream.
- Server → Client: payload comes from the TCP/UDP socket for the stream.
- Server must buffer received DATA in a FIFO queue per TCP stream.
- Buffer size is predetermined and must be the same for every stream.

#### `0x03` — CONTINUE

**Payload:**

| Field Name       | Field Type | Notes                                                   |
|------------------|------------|---------------------------------------------------------|
| Buffer Remaining | `uint32_t` | Number of packets the server can buffer for the stream. |

**Behavior:**
- Not sent for UDP streams; client does not track buffer for UDP.
- Client decrements buffer remaining by 1 for each DATA packet sent.
- Client cannot send DATA when buffer remaining reaches 0; must wait for another CONTINUE.
- Server sends CONTINUE when it has received its own maximum buffer count of packets.
- Server should send CONTINUE proactively to minimize client-side delays.

#### `0x04` — CLOSE

**Payload:**

| Field Name   | Field Type | Notes                                  |
|--------------|------------|----------------------------------------|
| Close Reason | `uint8_t`  | Reason for closing the connection.     |

**Behavior:**
- Immediately closes the associated stream and TCP socket.
- Close reason is informational (for debugging).

**Close Reasons (Client & Server):**

| Code  | Meaning                                    |
|-------|--------------------------------------------|
| `0x01` | Unspecified or unknown reason.           |
| `0x02` | Voluntary stream closure (reset).        |
| `0x03` | Unexpected closure due to network error. |
| `0x04` | Incompatible extensions (handshake only).|

**Server-Only Close Reasons:**

| Code  | Meaning                                                                |
|-------|------------------------------------------------------------------------|
| `0x41` | Stream creation failed — invalid info (reserved addr, bad port).     |
| `0x42` | Stream creation failed — unreachable destination host.               |
| `0x43` | Stream creation timed out — server not responding.                   |
| `0x44` | Stream creation failed — server refused connection.                  |
| `0x47` | TCP data transfer timed out.                                         |
| `0x48` | Destination address/domain intentionally blocked by proxy.           |
| `0x49` | Connection throttled by server.                                      |

**Client-Only Close Reasons:**

| Code  | Meaning                            |
|-------|------------------------------------|
| `0x81` | Client encountered unexpected error. |

**Extension Close Reasons:**

| Code  | Meaning                                    |
|-------|--------------------------------------------|
| `0xc0` | Auth failed — invalid username/password.  |
| `0xc1` | Auth failed — invalid signature.          |
| `0xc2` | Auth required but client provided no creds.|

#### `0x05` — INFO

**Payload:**

| Field Name         | Field Type | Notes                                          |
|--------------------|------------|------------------------------------------------|
| Major Wisp Version | `uint8_t`  | Major version of latest supported protocol.    |
| Minor Wisp Version | `uint8_t`  | Minor version of latest supported protocol.    |
| Extension Data     | `char[]`   | Array of extension metadata entries.           |

**Behavior:**
- Sent by both server and client immediately after WebSocket connection is established.
- Version numbers follow Semantic Versioning.
- If an extension is missing, it is assumed unsupported.

### Protocol Extensions

#### Extension Metadata Format

| Field Name         | Field Type | Notes                                                       |
|--------------------|------------|-------------------------------------------------------------|
| Extension ID       | `uint8_t`  | ID of the protocol extension.                               |
| Payload Length     | `uint32_t` | Length of the extension metadata payload, in bytes.         |
| Extension Metadata | `char[]`   | Custom byte array with extension status info.               |

#### `0x01` — UDP

Indicates UDP support. No payload.

#### `0x02` — Password Authentication

Adds username/password auth. A payload is required.

**Server message:**

| Field Name | Field Type | Notes                              |
|------------|------------|------------------------------------|
| Required   | `uint8_t`  | Whether password auth is required. |

**Client message:**

| Field Name      | Field Type | Notes                                     |
|-----------------|------------|-------------------------------------------|
| Username Length | `uint8_t`  | Length of username string.                |
| Username String | `char[]`   | UTF-8 encoded username.                  |
| Password String | `char[]`   | UTF-8 encoded password (rest of payload).|

**Behavior:**
- Server checks credentials; if invalid → CLOSE `0xc0` then close WebSocket.

#### `0x03` — Public/Private Key Authentication

Adds key-based auth. A payload is required.

**Server message:**

| Field Name           | Field Type | Notes                                                      |
|----------------------|------------|------------------------------------------------------------|
| Required             | `uint8_t`  | Whether key auth is required.                              |
| Supported Algorithms | `uint8_t`  | Bitmask of supported signature algorithms.                 |
| Challenge Data       | `char[]`   | Random challenge bytes (~512 bits).                        |

**Client message:**

| Field Name          | Field Type | Notes                                            |
|---------------------|------------|--------------------------------------------------|
| Username Length     | `uint8_t`  | Length of username string.                        |
| Username String     | `char[]`   | UTF-8 encoded username.                          |
| Selected Algorithm  | `uint8_t`  | Bitmask of selected algorithm.                   |
| Public Key Hash     | `char[32]` | SHA-256 hash of public key (always 32 bytes).    |
| Challenge Signature | `char[]`   | Signature using client's private key.            |

**Supported algorithms:** Ed25519 (`0b00000001`).

**Behavior:**
- Server verifies signature against stored public keys.
- If no match → CLOSE `0xc1` then close WebSocket.

#### `0x04` — Server MOTD

Server sends a welcome/notice message (UTF-8 string). Client can display it to the user. No client payload.

#### `0x05` — Stream Open Confirmation

If both client and server support this extension, the server sends a CONTINUE packet (stream ID 0) after a stream's underlying TCP socket is connected. Client can wait for this before sending DATA, but doesn't have to (waiting incurs latency).

#### Authentication Behavior

- Each auth extension's server message has a `Required` field.
- If no auth methods are required (or auth extensions are absent), auth is optional.
- If both key and password auth are required, client may choose which to use.

### HTTP / WebSocket Behavior

#### Server Architecture

The server must be an HTTP + WebSocket server conforming to their respective standards.

#### WebSocket URL

URLs should end with a trailing `/` to avoid confusion with wsproxy endpoints:
- wsproxy: `ws://example.com/customprefix/host:port`
- Wisp:    `ws://example.com/customprefix/`

The prefix may be used for gatekeeping / basic auth.

#### WebSocket Upgrade

- `Sec-WebSocket-Protocol` header must be present (value unspecified).
- If header is present → use Wisp v2.
- If header is absent → act as Wisp v1 server.

#### Handshake (v2)

1. Server sends INFO packet (version, supported extensions) on stream ID 0.
2. Client receives INFO; checks extension compatibility. Can reject with CLOSE `0x04` on stream ID 0.
3. Client sends its own INFO packet. If authenticating, credentials are included here.
4. If client receives a CONTINUE first → fall back to Wisp v1 (remaining steps don't apply).
5. Server receives client INFO; accepts or rejects.
   - Accept: sends CONTINUE with initial buffer size on stream ID 0.
   - Reject: sends CLOSE with reason on stream ID 0, then closes WebSocket.
6. Client waits for CONTINUE or CLOSE. After that, normal communication begins.
7. Only extensions both sides support may be used.

---

## Part 2: Ember Implementation Spec

### Design Goals

1. **Beat epoxy-server** — target >5 GiB/s on WispMark 5x10 config (epoxy: ~4.6 GiB/s)
2. **Adaptive buffers** — small for DNS, large for video streaming
3. **Plugin-ready architecture** — clean interfaces for future extension
4. **v1 + v2** — both always present, auto-detected per connection

### Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Async runtime | `tokio` multi-threaded | Work-stealing scheduler, proven |
| WebSocket | `tokio-websockets` | SIMD-accelerated masking (AVX2/NEON/SSE2) |
| TLS | `rustls` (optional) | Pure Rust, no C deps |
| Allocator | `tikv-jemallocator` | 20-40% less fragmentation than glibc malloc |
| Stream map | `rustc_hash::FxHashMap` | 3-5x faster than std HashMap (non-crypto hash) |
| Per-stream channels | `flume` | Lock-free MPMC, faster than tokio::sync::mpsc for try_send |
| DNS | `hickory-resolver` | Async, cached resolution |
| Config | `clap` (CLI) + `toml` (file) | Standard |
| Logging | `tracing` + `tracing-subscriber` | Structured, spans for perf |

### Dependencies

```toml
[package]
name = "ember"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
tokio-websockets = { version = "0.13", features = ["server", "client"] }
bytes = "1"
rustls = { version = "0.23", optional = true }
toml = "0.8"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
hickory-resolver = "0.24"
rand = "0.8"
thiserror = "1"
rustc-hash = "2"
flume = "0.11"

[target.'cfg(not(target_env = "msvc"))'.dependencies]
tikv-jemallocator = "0.6"

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

### Architecture

```
src/
├── main.rs              # Entry point, runtime setup
├── config.rs            # CLI + TOML configuration
├── server.rs            # TCP listener, WS upgrade, v1/v2 dispatch
├── wisp/
│   ├── mod.rs           # Re-exports
│   ├── packet.rs        # PacketType, parse/serialize (zero-copy)
│   ├── handshake.rs     # v1/v2 detection and handshake
│   ├── mux.rs           # MuxInner — single-owner multiplexor
│   ├── buffer.rs        # Adaptive flow control
│   └── extensions.rs    # Extension negotiation
├── proxy/
│   ├── mod.rs           # Proxy orchestration
│   ├── tcp.rs           # TCP forwarding via copy_buf
│   └── udp.rs           # UDP forwarding (future)
└── error.rs             # Error types
```

### Core Data Types

```rust
// src/wisp/packet.rs

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketType {
    Connect  = 0x01,
    Data     = 0x02,
    Continue = 0x03,
    Close    = 0x04,
    Info     = 0x05,
}

pub type StreamId = u32;

pub struct Packet {
    pub packet_type: PacketType,
    pub stream_id: StreamId,
    pub payload: Bytes,  // Zero-copy from WebSocket frame
}

impl Packet {
    /// Parse from raw bytes. Returns (Packet, remaining_bytes).
    /// Uses Buf::get_u8() / get_u32_le() — cursor advances, no copy.
    pub fn parse(data: Bytes) -> Result<Self, PacketError> { ... }

    /// Serialize to Bytes. Allocates once for header + payload.
    pub fn serialize(&self) -> Bytes { ... }
}
```

### MuxInner — The Hot Path

This is the single-owner multiplexor. One async task owns it entirely. No locks.

```rust
// src/wisp/mux.rs

use rustc_hash::FxHashMap;

pub struct StreamEntry {
    pub sender: flume::Sender<Bytes>,     // Lock-free channel to TCP write task
    pub flow_control: FlowControl,
}

pub enum FlowControl {
    /// TCP stream: bounded buffer, sends CONTINUE packets
    Enabled { buffer_size: u32, queued: u32 },
    /// UDP stream: no flow control
    Disabled,
}

pub struct MuxInner {
    /// Stream registry — FxHashMap, 3-5x faster than std HashMap
    streams: FxHashMap<StreamId, StreamEntry>,
    /// WS write sink (shared via Arc)
    ws_write: Arc<Mutex<WebSocketSink>>,
    /// Server config
    config: Arc<ServerConfig>,
}

impl MuxInner {
    /// Main loop — runs as single async task per WS connection.
    /// Reads WS frames, parses wisp headers, dispatches to streams.
    /// NO async in the inner loop — pure synchronous dispatch.
    pub async fn run(&mut self, mut ws_read: WebSocketRead) -> Result<(), WispError> {
        loop {
            let msg = ws_read.next().await.ok_or(WispError::ConnectionClosed)??;
            let packet = Packet::parse(msg.into_data())?;

            // Synchronous dispatch — no allocation, no await
            match packet.packet_type {
                PacketType::Data     => self.handle_data(packet.stream_id, packet.payload)?,
                PacketType::Connect  => self.handle_connect(packet).await?,
                PacketType::Continue => self.handle_continue(packet.stream_id, packet.payload)?,
                PacketType::Close    => self.handle_close(packet.stream_id, packet.payload),
                PacketType::Info     => unreachable!("handled during handshake"),
            }
        }
    }

    /// DATA dispatch — the critical hot path
    /// FxHashMap lookup + flume try_send. No locks. No await.
    #[inline]
    fn handle_data(&mut self, stream_id: StreamId, payload: Bytes) -> Result<(), WispError> {
        let entry = self.streams.get(&stream_id)
            .ok_or(WispError::UnknownStream(stream_id))?;

        // Non-blocking send — if channel is full, backpressure kicks in
        entry.sender.try_send(payload)
            .map_err(|_| WispError::BufferFull(stream_id))?;

        // Update flow control
        if let FlowControl::Enabled { queued, buffer_size } = &mut entry.flow_control {
            *queued += 1;
            if *queued >= *buffer_size {
                // Send CONTINUE to signal more capacity
                self.send_continue(stream_id)?;
                *queued = 0;
            }
        }

        Ok(())
    }
}
```

### Adaptive Buffer

```rust
// src/wisp/buffer.rs

pub struct AdaptiveBuffer {
    capacity: u32,
    queued: u32,
    min_capacity: u32,
    max_capacity: u32,
    high_watermark: f64,  // 0.8 default
    low_watermark: f64,   // 0.2 default
}

impl AdaptiveBuffer {
    #[inline]
    pub fn can_accept(&self) -> bool {
        self.queued < self.capacity
    }

    #[inline]
    pub fn on_send(&mut self) {
        self.queued += 1;
    }

    #[inline]
    pub fn on_drain(&mut self) {
        self.queued = self.queued.saturating_sub(1);
        // Shrink if below low watermark
        if self.queued as f64 / self.max_capacity as f64 < self.low_watermark {
            self.capacity = self.capacity.saturating_sub(64).max(self.min_capacity);
        }
    }

    #[inline]
    pub fn adapt(&mut self) {
        let usage = self.queued as f64 / self.capacity as f64;
        if usage > self.high_watermark && self.capacity < self.max_capacity {
            self.capacity = (self.capacity + 64).min(self.max_capacity);
        }
    }

    #[inline]
    pub fn remaining(&self) -> u32 {
        self.capacity.saturating_sub(self.queued)
    }
}
```

### TCP Proxy — The Data Plane

```rust
// src/proxy/tcp.rs

use tokio::io::{BufReader, copy_buf};

pub async fn proxy_tcp(
    stream_id: StreamId,
    tcp_stream: TcpStream,
    data_rx: flume::Receiver<Bytes>,
    ws_write: Arc<Mutex<WebSocketSink>>,
    buffer_config: BufferConfig,
) -> Result<(), WispError> {
    let (tcp_read, mut tcp_write) = tcp_stream.into_split();

    // 128KB BufReader — amortizes syscall overhead
    let mut tcp_read = BufReader::with_capacity(buffer_config.tcp_read_size, tcp_read);

    // Wisp stream → TCP: forward client data to upstream
    let mut data_rx_stream = data_rx.into_stream();

    // TCP → Wisp stream: forward upstream data to client
    let forward_to_client = async {
        let mut buf = BytesMut::with_capacity(65536);
        loop {
            buf.clear();
            let n = tcp_read.read_buf(&mut buf).await?;
            if n == 0 { break; }

            let packet = Packet {
                packet_type: PacketType::Data,
                stream_id,
                payload: buf.split().freeze(),
            };

            let mut ws = ws_write.lock().await;
            ws.send(Message::Binary(packet.serialize())).await?;
        }
        Ok::<(), WispError>(())
    };

    // Bidirectional copy using select
    tokio::select! {
        result = forward_to_client => result,
        // Client → TCP: forward data from channel
        _ = async {
            while let Ok(data) = data_rx_stream.next().await {
                tcp_write.write_all(&data).await?;
            }
            Ok::<(), WispError>(())
        } => result,
    }
}
```

### v1/v2 Handshake

```rust
// src/wisp/handshake.rs

pub fn detect_version(headers: &HeaderMap) -> WispVersion {
    if headers.contains_key("Sec-WebSocket-Protocol") {
        WispVersion::V2
    } else {
        WispVersion::V1
    }
}

pub async fn handshake_v2(
    ws_read: &mut WebSocketRead,
    ws_write: &mut WebSocketSink,
    config: &ServerConfig,
) -> Result<ExtensionNegotiation, WispError> {
    // 1. Server sends INFO (stream_id=0)
    let server_info = InfoPacket::new(config);
    ws_write.send(Message::Binary(server_info.serialize())).await?;

    // 2. Client sends INFO (stream_id=0)
    let msg = ws_read.next().await.ok_or(WispError::ConnectionClosed)??;
    let client_info = InfoPacket::parse(msg.into_data())?;

    // 3. Negotiate extensions
    let extensions = ExtensionNegotiation::negotiate(
        &config.extensions,
        &client_info.extensions,
    );

    // 4. Check for incompatible extensions
    if client_info.version_major > config.version_major {
        ws_write.send(Message::Binary(
            Packet::close(0, 0x04).serialize()
        )).await?;
        return Err(WispError::IncompatibleVersion);
    }

    // 5. Send CONTINUE with initial buffer size (stream_id=0)
    ws_write.send(Message::Binary(
        Packet::continue_packet(0, config.buffer.initial_size).serialize()
    )).await?;

    Ok(extensions)
}
```

### Extension Negotiation

```rust
// src/wisp/extensions.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extension {
    Udp       = 0x01,
    Motd      = 0x04,
    Soc       = 0x05,
}

pub struct ExtensionNegotiation {
    pub agreed: Vec<Extension>,
}

impl ExtensionNegotiation {
    pub fn negotiate(server: &[Extension], client: &[Extension]) -> Self {
        let agreed = server.iter()
            .filter(|e| client.contains(e))
            .cloned()
            .collect();
        Self { agreed }
    }

    pub fn has(&self, ext: Extension) -> bool {
        self.agreed.contains(&ext)
    }
}
```

### Server Loop

```rust
// src/server.rs

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(&config.listen_addr).await?;
    tracing::info!(addr = %config.listen_addr, "Ember listening");

    loop {
        let (stream, addr) = listener.accept().await?;
        let config = config.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, addr, config).await {
                tracing::error!(%addr, "connection error: {e}");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    config: Config,
) -> Result<(), WispError> {
    stream.set_nodelay(true)?;

    let mut ws = accept_http(stream).await?;
    let version = detect_version(&ws.headers());

    // Split WS into read/write halves
    let (ws_read, ws_write) = ws.split();

    // Create mux
    let mut mux = MuxInner::new(Arc::new(ws_write), config.clone());

    match version {
        WispVersion::V1 => {
            // v1: send CONTINUE(stream_id=0) immediately
            mux.send_continue(0, config.buffer.initial_size).await?;
        }
        WispVersion::V2 => {
            // v2: perform INFO handshake
            let extensions = handshake_v2(&mut ws_read, &mut ws_write, &config).await?;
            mux.extensions = extensions;
        }
    }

    // Run the multiplexor — this is the hot path
    mux.run(ws_read).await
}
```

### Configuration

```toml
# ember.toml
[server]
host = "0.0.0.0"
port = 443
max_connections = 10000

[tls]
enabled = false
cert_path = ""
key_path = ""

[buffer]
initial_size = 128
min_size = 32
max_size = 1024
high_watermark = 0.8
low_watermark = 0.2
tcp_read_size = 131072  # 128KB

[extensions]
udp = true
motd = "Welcome to Ember"
stream_open_confirmation = false

[logging]
level = "info"
```

### CLI

```
ember [OPTIONS]

Options:
  -c, --config <FILE>       Config file path [default: ember.toml]
  -H, --host <HOST>         Listen address [default: 0.0.0.0]
  -p, --port <PORT>         Listen port [default: 443]
      --tls                 Enable TLS
      --cert <FILE>         TLS certificate path
      --key <FILE>          TLS key path
  -v, --verbose             Enable debug logging
  -h, --help                Print help
```

### Performance Optimizations Summary

| Optimization | Impact | Source |
|-------------|--------|--------|
| `tokio-websockets` SIMD masking | 2-3x faster than byte-by-byte XOR | epoxy-server |
| Single-owner MuxInner | No locks on hot path | epoxy-server |
| `FxHashMap` stream registry | 3-5x faster lookups than std HashMap | epoxy-server |
| `flume` channels | Lock-free, faster than tokio::sync::mpsc | epoxy-server |
| `tikv-jemallocator` | 20-40% less fragmentation | epoxy-server |
| 128KB `BufReader` | Amortizes TCP syscalls | epoxy-server |
| `Bytes` zero-copy | No payload copies in hot path | tungstenite/tokio-websockets |
| `LTO=true` + `codegen-units=1` | Max cross-crate inlining | Standard perf practice |
| `panic="abort"` | No unwinding overhead | Standard perf practice |
| `TCP_NODELAY` | No Nagle buffering | Standard perf practice |

### MVP Feature Set (v0.1.0)

- [x] TCP CONNECT + DATA + CLOSE (core proxy)
- [x] CONTINUE flow control
- [x] Adaptive buffer sizing
- [x] v1 support (no handshake)
- [x] v2 handshake (INFO exchange)
- [x] Extension negotiation (UDP, MOTD, SOC)
- [x] UDP support (extension `0x01`)
- [x] MOTD support (extension `0x04`)
- [x] Stream Open Confirmation (extension `0x05`)
- [x] CLI config (clap) + TOML config file
- [x] Structured logging (tracing)
- [x] TCP_NODELAY on all sockets
- [x] jemalloc allocator

**Deferred:** Auth, Prometheus metrics, rate limiting, connection limits, graceful shutdown.

### Performance Targets

| Metric | epoxy-server (current best) | Ember Target |
|--------|---------------------------|--------------|
| 1x10 streams | ~1.3 GiB/s | >1.4 GiB/s |
| 5x10 streams | ~4.6 GiB/s | >5.0 GiB/s |
| Memory per connection | ~10 KB | <8 KB |

### Implementation Order

1. **Packet parsing** — `packet.rs`, types, serialize/deserialize
2. **Adaptive buffer** — `buffer.rs`
3. **TCP proxy** — `proxy/tcp.rs`, the hot path
4. **MuxInner** — `mux.rs`, single-owner multiplexor
5. **v1 handler** — simple loop, no handshake
6. **v2 handshake** — `handshake.rs`, INFO exchange
7. **Extensions** — `extensions.rs`, UDP/MOTD/SOC
8. **Server** — `server.rs`, accept loop, WS upgrade
9. **Config** — `config.rs`, CLI + TOML
10. **Main** — `main.rs`, wire it all together
