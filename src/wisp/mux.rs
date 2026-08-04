use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use flume;
use futures_util::{SinkExt, StreamExt};
use rustc_hash::FxHashMap;
use tokio_websockets::{Message, WebSocketStream};

use crate::circuit_breaker::{CircuitBreaker, CircuitBreakerError};
use crate::error::WispError;
use crate::pool::ConnectionPool;
use crate::proxy::tcp::{proxy_tcp, proxy_tcp_connect};
use crate::proxy::udp::proxy_udp;
use crate::wisp::buffer::{AdaptiveBuffer, BufferConfig};
use crate::wisp::extensions::ExtensionNegotiation;
use crate::wisp::packet::{Packet, PacketType, StreamId};
use crate::wisp::plugin::{PluginEvent, PluginManager};
use crate::wisp::plugins::Metrics;

pub struct StreamEntry {
    pub sender: flume::Sender<Bytes>,
    pub buffer: AdaptiveBuffer,
}

pub struct MuxInner {
    streams: FxHashMap<StreamId, StreamEntry>,
    ws_write_tx: flume::Sender<Message>,
    buffer_config: BufferConfig,
    extensions: ExtensionNegotiation,
    motd: Option<String>,
    tcp_read_size: usize,
    plugins: Arc<PluginManager>,
    peer_addr: SocketAddr,
    metrics: Option<Arc<Metrics>>,
    max_streams: u32,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
    pool: Arc<ConnectionPool>,
}

impl MuxInner {
    pub fn new(
        buffer_config: BufferConfig,
        extensions: ExtensionNegotiation,
        motd: Option<String>,
        tcp_read_size: usize,
        plugins: Arc<PluginManager>,
        peer_addr: SocketAddr,
        metrics: Option<Arc<Metrics>>,
        max_streams: u32,
        circuit_breaker: Option<Arc<CircuitBreaker>>,
        pool: Arc<ConnectionPool>,
    ) -> Self {
        let (ws_write_tx, _) = flume::unbounded();

        Self {
            streams: FxHashMap::default(),
            ws_write_tx,
            buffer_config,
            extensions,
            motd,
            tcp_read_size,
            plugins,
            peer_addr,
            metrics,
            max_streams,
            circuit_breaker,
            pool,
        }
    }

    pub async fn run<S>(&mut self, ws: tokio_websockets::WebSocketStream<S>) -> Result<(), WispError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        // Split into independent read/write halves
        let (mut ws_write, mut ws_read) = ws.split();

        // Channel for proxy tasks to send WS messages
        let (ws_write_tx, ws_write_rx) = flume::unbounded::<Message>();
        self.ws_write_tx = ws_write_tx;

        // Notify plugins: connection open
        let _ = self.plugins.notify(&PluginEvent::ConnectionOpen {
            addr: self.peer_addr,
        }).await;

        // Spawn a dedicated writer task
        let writer_handle = tokio::spawn(async move {
            while let Ok(msg) = ws_write_rx.recv_async().await {
                if ws_write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Main read loop
        let result = self.read_loop(&mut ws_read).await;

        // Notify plugins: connection close
        let _ = self.plugins.notify(&PluginEvent::ConnectionClose {
            addr: self.peer_addr,
        }).await;

        // Close channel to signal writer to exit
        drop(std::mem::replace(&mut self.ws_write_tx, flume::unbounded().0));

        let _ = writer_handle.await;
        result
    }

    #[inline(always)]
    async fn read_loop<S>(
        &mut self,
        ws_read: &mut futures_util::stream::SplitStream<tokio_websockets::WebSocketStream<S>>,
    ) -> Result<(), WispError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        loop {
            let msg = match ws_read.next().await {
                Some(Ok(msg)) => msg,
                Some(Err(e)) => return Err(WispError::WebSocket(e.to_string())),
                None => return Ok(()),
            };

            if msg.is_close() || !msg.is_binary() {
                if msg.is_close() { return Ok(()); }
                continue;
            }

            let payload: Bytes = msg.into_payload().into();

            // Inline packet parsing — avoid Packet struct for DATA (hot path)
            if payload.len() < 5 {
                continue;
            }

            let packet_type = payload[0];
            let stream_id = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
            let data = payload.slice(5..);

            match packet_type {
                0x01 => {
                    // CONNECT
                    if let Err(e) = self.handle_connect_raw(stream_id, data).await {
                        tracing::error!("connect error: {}", e);
                    }
                }
                0x02 => {
                    // DATA — inline hot path
                    if let Some(entry) = self.streams.get(&stream_id) {
                        let _ = entry.sender.try_send(data);
                    }
                }
                0x03 => {
                    // CONTINUE — no-op for now
                }
                0x04 => {
                    // CLOSE
                    self.handle_close(stream_id);
                }
                _ => {}
            }
        }
    }

    /// Raw connect handler — takes bytes directly, avoids Packet struct
    async fn handle_connect_raw(&mut self, stream_id: StreamId, payload: Bytes) -> Result<(), WispError> {
        // Secondary stream count check
        if self.streams.len() as u32 >= self.max_streams {
            self.send_close(stream_id, 0x45)?;
            return Err(WispError::ConnectionLimitReached);
        }

        if payload.len() < 3 {
            self.send_close(stream_id, 0x41)?;
            return Err(WispError::PacketTooShort(payload.len()));
        }

        let stream_type = payload[0];
        let port = u16::from_le_bytes([payload[1], payload[2]]);
        let hostname = String::from_utf8_lossy(&payload[3..]).to_string();

        if stream_type != 0x01 && stream_type != 0x02 {
            self.send_close(stream_id, 0x41)?;
            return Err(WispError::InvalidStreamType(stream_type));
        }

        // Notify plugins
        let event = PluginEvent::StreamOpen {
            stream_id,
            hostname: hostname.clone(),
            port,
            stream_type,
        };
        if let Err(reason) = self.plugins.notify(&event).await {
            self.send_close(stream_id, 0x48)?;
            return Err(WispError::WebSocket(reason));
        }

        let (data_tx, data_rx) = flume::bounded(self.buffer_config.initial_size as usize);
        let buffer = AdaptiveBuffer::new(self.buffer_config.clone());

        self.streams.insert(stream_id, StreamEntry { sender: data_tx, buffer });

        let ws_write_tx = self.ws_write_tx.clone();
        let tcp_read_size = self.tcp_read_size;
        let motd = self.motd.clone();
        let metrics = self.metrics.clone();
        let circuit_breaker = self.circuit_breaker.clone();
        let pool = self.pool.clone();
        let target = format!("{}:{}", hostname, port);

        if stream_type == 0x02 {
            // UDP
            let upstream_addr = format!("{}:{}", hostname, port);
            tokio::spawn(async move {
                let addr: std::net::SocketAddr = match upstream_addr.parse() {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::error!("Invalid UDP address {}: {}", upstream_addr, e);
                        let packet = Packet::close(stream_id, 0x41);
                        let _ = ws_write_tx.send(Message::binary(packet.serialize()));
                        return;
                    }
                };
                if let Err(e) = crate::proxy::udp::proxy_udp(stream_id, addr, data_rx, ws_write_tx).await {
                    tracing::trace!("UDP proxy error for stream {}: {}", stream_id, e);
                }
            });
        } else {
            // TCP
            tokio::spawn(async move {
                let tcp_stream = if let Some(stream) = pool.get(&target).await {
                    tracing::trace!("pool hit for {}", target);
                    stream
                } else {
                    pool.miss();
                    if let Some(ref cb) = circuit_breaker {
                        let hostname = hostname.clone();
                        let connect_result = cb.call(|| async move {
                            proxy_tcp_connect(hostname, port)
                                .await
                                .map_err(|e| CircuitBreakerError::UpstreamError(e.to_string()))
                        }).await;
                        match connect_result {
                            Ok(stream) => stream,
                            Err(CircuitBreakerError::CircuitOpen) => {
                                tracing::warn!("circuit breaker open, rejecting TCP connect for stream {}", stream_id);
                                let packet = Packet::close(stream_id, 0x44);
                                let _ = ws_write_tx.send(Message::binary(packet.serialize()));
                                return;
                            }
                            Err(CircuitBreakerError::UpstreamError(_)) => {
                                let packet = Packet::close(stream_id, 0x42);
                                let _ = ws_write_tx.send(Message::binary(packet.serialize()));
                                return;
                            }
                        }
                    } else {
                        let max_retries = 3;
                        let mut attempts = 0;
                        loop {
                            match proxy_tcp_connect(hostname.clone(), port).await {
                                Ok(stream) => break stream,
                                Err(e) => {
                                    attempts += 1;
                                    if attempts >= max_retries {
                                        tracing::error!("TCP connect failed for stream {} after {} attempts: {}", stream_id, attempts, e);
                                        let reason = match e {
                                            WispError::Io(ref io_err) if io_err.kind() == std::io::ErrorKind::ConnectionRefused => 0x44,
                                            WispError::Io(_) => 0x42,
                                            _ => 0x41,
                                        };
                                        let packet = Packet::close(stream_id, reason);
                                        let _ = ws_write_tx.send(Message::binary(packet.serialize()));
                                        return;
                                    }
                                    tracing::debug!("TCP connect attempt {} failed for stream {}, retrying: {}", attempts, stream_id, e);
                                    tokio::time::sleep(std::time::Duration::from_millis(100 * attempts as u64)).await;
                                }
                            }
                        }
                    }
                };
                if let Some(motd_text) = motd {
                    tracing::debug!("MOTD: {}", motd_text);
                }
                let continue_pkt = Packet::continue_packet(stream_id, 128);
                let _ = ws_write_tx.send(Message::binary(continue_pkt.serialize()));
                let result = crate::proxy::tcp::proxy_tcp(stream_id, tcp_stream, data_rx, ws_write_tx, tcp_read_size, metrics).await;
                match result {
                    Ok(stream) => {
                        pool.put(&target, stream).await;
                    }
                    Err(e) => {
                        tracing::trace!("proxy error for stream {}: {}", stream_id, e);
                        pool.remove(&target);
                    }
                }
            });
        }

        Ok(())
    }

    async fn handle_connect(&mut self, packet: Packet) -> Result<(), WispError> {
        // Secondary stream count check
        if self.streams.len() as u32 >= self.max_streams {
            self.send_close(packet.stream_id, 0x45)?;
            return Err(WispError::ConnectionLimitReached);
        }

        if packet.payload.len() < 3 {
            self.send_close(packet.stream_id, 0x41)?;
            return Err(WispError::PacketTooShort(packet.payload.len()));
        }

        let stream_type = packet.payload[0];
        let port = u16::from_le_bytes([packet.payload[1], packet.payload[2]]);
        let hostname = String::from_utf8_lossy(&packet.payload[3..]).to_string();

        if stream_type != 0x01 && stream_type != 0x02 {
            self.send_close(packet.stream_id, 0x41)?;
            return Err(WispError::InvalidStreamType(stream_type));
        }

        // Notify plugins: stream open
        let event = PluginEvent::StreamOpen {
            stream_id: packet.stream_id,
            hostname: hostname.clone(),
            port,
            stream_type,
        };
        if let Err(reason) = self.plugins.notify(&event).await {
            self.send_close(packet.stream_id, 0x48)?;
            return Err(WispError::WebSocket(reason));
        }

        tracing::info!(
            addr = %self.peer_addr,
            stream_id = packet.stream_id,
            hostname = %hostname,
            port = port,
            stream_type = if stream_type == 0x01 { "TCP" } else { "UDP" },
            "stream opened"
        );

        let (data_tx, data_rx) = flume::bounded(self.buffer_config.initial_size as usize);
        let buffer = AdaptiveBuffer::new(self.buffer_config.clone());

        self.streams.insert(
            packet.stream_id,
            StreamEntry { sender: data_tx, buffer },
        );

        let stream_id = packet.stream_id;
        let ws_write_tx = self.ws_write_tx.clone();
        let tcp_read_size = self.tcp_read_size;
        let motd = self.motd.clone();
        let metrics = self.metrics.clone();
        let circuit_breaker = self.circuit_breaker.clone();
        let pool = self.pool.clone();
        let target = format!("{}:{}", hostname, port);

        if stream_type == 0x02 {
            // UDP proxy
            let upstream_addr = format!("{}:{}", hostname, port);
            tokio::spawn(async move {
                let addr: std::net::SocketAddr = match upstream_addr.parse() {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::error!("Invalid UDP address {}: {}", upstream_addr, e);
                        let packet = Packet::close(stream_id, 0x41);
                        let _ = ws_write_tx.send(Message::binary(packet.serialize()));
                        return;
                    }
                };
                if let Err(e) = proxy_udp(stream_id, addr, data_rx, ws_write_tx).await {
                    tracing::trace!("UDP proxy error for stream {}: {}", stream_id, e);
                }
            });
        } else {
            // TCP proxy
            tokio::spawn(async move {
                let tcp_stream = if let Some(stream) = pool.get(&target).await {
                    tracing::trace!("pool hit for {}", target);
                    stream
                } else {
                    pool.miss();
                    if let Some(ref cb) = circuit_breaker {
                        let hostname = hostname.clone();
                        let connect_result = cb.call(|| async move {
                            proxy_tcp_connect(hostname, port)
                                .await
                                .map_err(|e| CircuitBreakerError::UpstreamError(e.to_string()))
                        }).await;
                        match connect_result {
                            Ok(stream) => stream,
                            Err(CircuitBreakerError::CircuitOpen) => {
                                tracing::warn!("circuit breaker open, rejecting TCP connect for stream {}", stream_id);
                                let packet = Packet::close(stream_id, 0x44);
                                let _ = ws_write_tx.send(Message::binary(packet.serialize()));
                                return;
                            }
                            Err(CircuitBreakerError::UpstreamError(_)) => {
                                let packet = Packet::close(stream_id, 0x42);
                                let _ = ws_write_tx.send(Message::binary(packet.serialize()));
                                return;
                            }
                        }
                    } else {
                        let max_retries = 3;
                        let mut attempts = 0;
                        loop {
                            match proxy_tcp_connect(hostname.clone(), port).await {
                                Ok(stream) => break stream,
                                Err(e) => {
                                    attempts += 1;
                                    if attempts >= max_retries {
                                        tracing::error!("TCP connect failed for stream {} after {} attempts: {}", stream_id, attempts, e);
                                        let reason = match e {
                                            WispError::Io(ref io_err) if io_err.kind() == std::io::ErrorKind::ConnectionRefused => 0x44,
                                            WispError::Io(_) => 0x42,
                                            _ => 0x41,
                                        };
                                        let packet = Packet::close(stream_id, reason);
                                        let _ = ws_write_tx.send(Message::binary(packet.serialize()));
                                        return;
                                    }
                                    tracing::debug!("TCP connect attempt {} failed for stream {}, retrying: {}", attempts, stream_id, e);
                                    tokio::time::sleep(std::time::Duration::from_millis(100 * attempts as u64)).await;
                                }
                            }
                        }
                    }
                };
                if let Some(motd_text) = motd {
                    tracing::debug!("MOTD: {}", motd_text);
                }
                let continue_pkt = Packet::continue_packet(stream_id, 128);
                let _ = ws_write_tx.send(Message::binary(continue_pkt.serialize()));
                let result = proxy_tcp(stream_id, tcp_stream, data_rx, ws_write_tx, tcp_read_size, metrics).await;
                match result {
                    Ok(stream) => {
                        pool.put(&target, stream).await;
                    }
                    Err(e) => {
                        tracing::trace!("proxy error for stream {}: {}", stream_id, e);
                        pool.remove(&target);
                    }
                }
            });
        }

        Ok(())
    }

    fn handle_data(&self, packet: Packet) -> Result<(), WispError> {
        let entry = self.streams.get(&packet.stream_id)
            .ok_or(WispError::UnknownStream(packet.stream_id))?;

        entry.sender.try_send(packet.payload)
            .map_err(|_| WispError::BufferFull(packet.stream_id))?;

        Ok(())
    }

    fn handle_continue(&self, _stream_id: StreamId) -> Result<(), WispError> {
        Ok(())
    }

    fn handle_close(&mut self, stream_id: StreamId) {
        self.streams.remove(&stream_id);

        tracing::info!(
            addr = %self.peer_addr,
            stream_id = stream_id,
            "stream closed"
        );

        // Notify plugins: stream close
        let plugins = self.plugins.clone();
        let stream_id_copy = stream_id;
        tokio::spawn(async move {
            let _ = plugins.notify(&PluginEvent::StreamClose { stream_id: stream_id_copy }).await;
        });
    }

    fn send_close(&self, stream_id: StreamId, reason: u8) -> Result<(), WispError> {
        let packet = Packet::close(stream_id, reason);
        self.ws_write_tx.send(Message::binary(packet.serialize()))
            .map_err(|_| WispError::ConnectionClosed)
    }
}

impl Drop for MuxInner {
    fn drop(&mut self) {
        self.streams.clear();
    }
}
