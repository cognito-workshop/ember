use std::net::SocketAddr;
use std::sync::Arc;

use crate::wisp::packet::StreamId;

/// Events that plugins can hook into
#[derive(Debug, Clone)]
pub enum PluginEvent {
    /// New WebSocket connection established
    ConnectionOpen { addr: SocketAddr },
    /// Wisp v1 or v2 handshake completed
    HandshakeComplete {
        addr: SocketAddr,
        version: u8, // 1 or 2
    },
    /// New stream opened (CONNECT packet processed)
    StreamOpen {
        stream_id: StreamId,
        hostname: String,
        port: u16,
        stream_type: u8, // 0x01=TCP, 0x02=UDP
    },
    /// DATA packet received/sent
    DataTransfer { stream_id: StreamId, bytes: u64 },
    /// Stream closed
    StreamClose { stream_id: StreamId },
    /// Connection closed
    ConnectionClose { addr: SocketAddr },
    /// Server shutting down
    Shutdown,
}

/// Plugin hook result
#[derive(Debug)]
pub enum HookResult {
    /// Continue processing normally
    Continue,
    /// Reject the connection/stream (with reason)
    Reject(String),
}

/// The core plugin trait. Implement this to create a plugin.
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    /// Plugin name
    fn name(&self) -> &str;

    /// Called when a plugin event occurs.
    /// Return HookResult::Reject to block the operation.
    async fn on_event(&self, event: &PluginEvent) -> HookResult;

    /// Called when the plugin is loaded
    async fn on_load(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Called when the server is shutting down
    async fn on_unload(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

/// Plugin manager that holds all registered plugins
pub struct PluginManager {
    plugins: Vec<Arc<dyn Plugin>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin
    pub fn register(&mut self, plugin: Arc<dyn Plugin>) {
        tracing::info!("Registered plugin: {}", plugin.name());
        self.plugins.push(plugin);
    }

    /// Notify all plugins of an event. Returns Err if any plugin rejects.
    pub async fn notify(&self, event: &PluginEvent) -> Result<(), String> {
        for plugin in &self.plugins {
            match plugin.on_event(event).await {
                HookResult::Continue => {}
                HookResult::Reject(reason) => {
                    tracing::warn!(
                        "Plugin '{}' rejected {:?}: {}",
                        plugin.name(),
                        event,
                        reason
                    );
                    return Err(reason);
                }
            }
        }
        Ok(())
    }

    /// Load all plugins
    pub async fn load_all(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for plugin in &self.plugins {
            plugin.on_load().await?;
            tracing::info!("Loaded plugin: {}", plugin.name());
        }
        Ok(())
    }

    /// Unload all plugins
    pub async fn unload_all(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for plugin in self.plugins.iter().rev() {
            plugin.on_unload().await?;
            tracing::info!("Unloaded plugin: {}", plugin.name());
        }
        Ok(())
    }

    /// Get plugin count
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

// Default implementation
impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
