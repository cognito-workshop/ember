use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
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
    /// IP -> (count, window_start)
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

                // Reset window if expired
                if now.duration_since(entry.1) > self.window {
                    *entry = (0, now);
                }

                entry.0 += 1;

                if entry.0 > self.max_connections_per_ip {
                    tracing::warn!(
                        "Rate limit exceeded for {}: {} connections in {}s",
                        ip,
                        entry.0,
                        self.window.as_secs()
                    );
                    return HookResult::Reject(format!(
                        "Rate limit: {} connections per {}s",
                        self.max_connections_per_ip,
                        self.window.as_secs()
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
