use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::wisp::plugin::{Plugin, PluginEvent, HookResult};

/// Rate limiter plugin — limits connections per IP
pub struct RateLimiter {
    max_connections_per_ip: u32,
    window: Duration,
    state: Mutex<RateLimiterState>,
}

struct RateLimiterState {
    connections: HashMap<std::net::IpAddr, (u32, Instant)>,
}

impl RateLimiter {
    pub fn new(max_connections_per_ip: u32, window_secs: u64) -> Arc<Self> {
        Arc::new(Self {
            max_connections_per_ip,
            window: Duration::from_secs(window_secs),
            state: Mutex::new(RateLimiterState {
                connections: HashMap::new(),
            }),
        })
    }
}

#[async_trait::async_trait]
impl Plugin for RateLimiter {
    fn name(&self) -> &str {
        "rate-limiter"
    }

    async fn on_event(&self, event: &PluginEvent) -> HookResult {
        match event {
            PluginEvent::ConnectionOpen { addr } => {
                let mut state = self.state.lock().await;
                let ip = addr.ip();
                let now = Instant::now();

                let entry = state.connections.entry(ip).or_insert((0, now));

                if now.duration_since(entry.1) > self.window {
                    *entry = (0, now);
                }

                entry.0 += 1;

                if entry.0 > self.max_connections_per_ip {
                    tracing::warn!(
                        "Rate limit exceeded for {}: {} connections in {}s",
                        ip, entry.0, self.window.as_secs()
                    );
                    return HookResult::Reject(format!(
                        "Rate limit: {} connections per {}s",
                        self.max_connections_per_ip, self.window.as_secs()
                    ));
                }

                HookResult::Continue
            }
            PluginEvent::ConnectionClose { addr } => {
                let mut state = self.state.lock().await;
                let ip = addr.ip();
                if let Some(entry) = state.connections.get_mut(&ip) {
                    entry.0 = entry.0.saturating_sub(1);
                }
                HookResult::Continue
            }
            _ => HookResult::Continue,
        }
    }
}

/// Connection limiter plugin — global max connections
pub struct ConnectionLimiter {
    max: u32,
    count: AtomicU64,
}

impl ConnectionLimiter {
    pub fn new(max: u32) -> Arc<Self> {
        Arc::new(Self {
            max,
            count: AtomicU64::new(0),
        })
    }
}

#[async_trait::async_trait]
impl Plugin for ConnectionLimiter {
    fn name(&self) -> &str {
        "connection-limiter"
    }

    async fn on_event(&self, event: &PluginEvent) -> HookResult {
        match event {
            PluginEvent::ConnectionOpen { .. } => {
                let current = self.count.fetch_add(1, Ordering::Relaxed) + 1;
                if current > self.max as u64 {
                    self.count.fetch_sub(1, Ordering::Relaxed);
                    tracing::warn!("Connection limit reached: {}/{}", current, self.max);
                    return HookResult::Reject(format!("Connection limit: {}", self.max));
                }
                HookResult::Continue
            }
            PluginEvent::ConnectionClose { .. } => {
                self.count.fetch_sub(1, Ordering::Relaxed);
                HookResult::Continue
            }
            _ => HookResult::Continue,
        }
    }
}

/// Metrics plugin — tracks connection and stream counts
pub struct Metrics {
    pub connections_total: AtomicU64,
    pub connections_active: AtomicU64,
    pub streams_total: AtomicU64,
    pub streams_active: AtomicU64,
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            connections_total: AtomicU64::new(0),
            connections_active: AtomicU64::new(0),
            streams_total: AtomicU64::new(0),
            streams_active: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
        })
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            connections_total: self.connections_total.load(Ordering::Relaxed),
            connections_active: self.connections_active.load(Ordering::Relaxed),
            streams_total: self.streams_total.load(Ordering::Relaxed),
            streams_active: self.streams_active.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub connections_total: u64,
    pub connections_active: u64,
    pub streams_total: u64,
    pub streams_active: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

impl std::fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "conns={}/{} streams={}/{} bytes={}/{}",
            self.connections_active, self.connections_total,
            self.streams_active, self.streams_total,
            self.bytes_in, self.bytes_out,
        )
    }
}

#[async_trait::async_trait]
impl Plugin for Metrics {
    fn name(&self) -> &str {
        "metrics"
    }

    async fn on_event(&self, event: &PluginEvent) -> HookResult {
        match event {
            PluginEvent::ConnectionOpen { .. } => {
                self.connections_total.fetch_add(1, Ordering::Relaxed);
                self.connections_active.fetch_add(1, Ordering::Relaxed);
            }
            PluginEvent::ConnectionClose { .. } => {
                self.connections_active.fetch_sub(1, Ordering::Relaxed);
            }
            PluginEvent::StreamOpen { .. } => {
                self.streams_total.fetch_add(1, Ordering::Relaxed);
                self.streams_active.fetch_add(1, Ordering::Relaxed);
            }
            PluginEvent::StreamClose { .. } => {
                self.streams_active.fetch_sub(1, Ordering::Relaxed);
            }
            PluginEvent::DataTransfer { bytes, .. } => {
                self.bytes_in.fetch_add(*bytes, Ordering::Relaxed);
            }
            _ => {}
        }
        HookResult::Continue
    }
}

/// Simple logger plugin — logs all events
pub struct Logger;

#[async_trait::async_trait]
impl Plugin for Logger {
    fn name(&self) -> &str {
        "logger"
    }

    async fn on_event(&self, event: &PluginEvent) -> HookResult {
        match event {
            PluginEvent::ConnectionOpen { addr } => {
                tracing::info!("[plugin:logger] Connection open from {}", addr);
            }
            PluginEvent::HandshakeComplete { addr, version } => {
                tracing::info!("[plugin:logger] Handshake complete with {} (v{})", addr, version);
            }
            PluginEvent::StreamOpen { stream_id, hostname, port, stream_type } => {
                let proto = if *stream_type == 0x01 { "TCP" } else { "UDP" };
                tracing::info!(
                    "[plugin:logger] Stream {} opened: {}:{} ({})",
                    stream_id, hostname, port, proto
                );
            }
            PluginEvent::StreamClose { stream_id } => {
                tracing::info!("[plugin:logger] Stream {} closed", stream_id);
            }
            PluginEvent::ConnectionClose { addr } => {
                tracing::info!("[plugin:logger] Connection closed from {}", addr);
            }
            PluginEvent::DataTransfer { stream_id, bytes } => {
                tracing::trace!("[plugin:logger] Stream {} transferred {} bytes", stream_id, bytes);
            }
            PluginEvent::Shutdown => {
                tracing::info!("[plugin:logger] Server shutting down");
            }
        }
        HookResult::Continue
    }
}
