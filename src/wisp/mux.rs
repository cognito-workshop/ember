use bytes::Bytes;
use flume;
use futures_util::{SinkExt, StreamExt};
use rustc_hash::FxHashMap;
use tokio::net::TcpStream;
use tokio_websockets::{Message, WebSocketStream};

use crate::error::WispError;
use crate::proxy::tcp::{proxy_tcp, proxy_tcp_connect};
use crate::wisp::buffer::{AdaptiveBuffer, BufferConfig};
use crate::wisp::extensions::ExtensionNegotiation;
use crate::wisp::packet::{Packet, PacketType, StreamId};

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
}

impl MuxInner {
    pub fn new(
        buffer_config: BufferConfig,
        extensions: ExtensionNegotiation,
        motd: Option<String>,
        tcp_read_size: usize,
    ) -> Self {
        let (ws_write_tx, _) = flume::unbounded();

        Self {
            streams: FxHashMap::default(),
            ws_write_tx,
            buffer_config,
            extensions,
            motd,
            tcp_read_size,
        }
    }

    pub async fn run(&mut self, ws: WebSocketStream<TcpStream>) -> Result<(), WispError> {
        // Split into independent read/write halves
        let (mut ws_write, mut ws_read) = ws.split();

        // Channel for proxy tasks to send WS messages
        let (ws_write_tx, ws_write_rx) = flume::unbounded::<Message>();
        self.ws_write_tx = ws_write_tx;

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

        // Close channel to signal writer to exit
        drop(std::mem::replace(&mut self.ws_write_tx, flume::unbounded().0));

        let _ = writer_handle.await;
        result
    }

    async fn read_loop(
        &mut self,
        ws_read: &mut futures_util::stream::SplitStream<WebSocketStream<TcpStream>>,
    ) -> Result<(), WispError> {
        loop {
            let msg = match ws_read.next().await {
                Some(Ok(msg)) => msg,
                Some(Err(e)) => return Err(WispError::WebSocket(e.to_string())),
                None => return Ok(()),
            };

            if msg.is_close() {
                return Ok(());
            }

            if !msg.is_binary() {
                continue;
            }

            let payload: Bytes = msg.into_payload().into();
            let packet = Packet::parse(payload)?;

            match packet.packet_type {
                PacketType::Connect => {
                    if let Err(e) = self.handle_connect(packet).await {
                        tracing::error!("connect error: {}", e);
                    }
                }
                PacketType::Data => {
                    if let Err(e) = self.handle_data(packet) {
                        tracing::trace!("data error: {}", e);
                    }
                }
                PacketType::Continue => {
                    if let Err(e) = self.handle_continue(packet.stream_id) {
                        tracing::trace!("continue error: {}", e);
                    }
                }
                PacketType::Close => {
                    self.handle_close(packet.stream_id);
                }
                PacketType::Info => {
                    unreachable!("INFO handled during handshake");
                }
            }
        }
    }

    async fn handle_connect(&mut self, packet: Packet) -> Result<(), WispError> {
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

        tokio::spawn(async move {
            match proxy_tcp_connect(hostname, port).await {
                Ok(tcp_stream) => {
                    if let Some(motd_text) = motd {
                        tracing::debug!("MOTD: {}", motd_text);
                    }
                    let continue_pkt = Packet::continue_packet(stream_id, 128);
                    let _ = ws_write_tx.send(Message::binary(continue_pkt.serialize()));
                    if let Err(e) = proxy_tcp(stream_id, tcp_stream, data_rx, ws_write_tx, tcp_read_size).await {
                        tracing::trace!("proxy error for stream {}: {}", stream_id, e);
                    }
                }
                Err(e) => {
                    tracing::error!("TCP connect failed for stream {}: {}", stream_id, e);
                    let reason = match e {
                        WispError::Io(ref io_err) if io_err.kind() == std::io::ErrorKind::ConnectionRefused => 0x44,
                        WispError::Io(_) => 0x42,
                        _ => 0x41,
                    };
                    let packet = Packet::close(stream_id, reason);
                    let _ = ws_write_tx.send(Message::binary(packet.serialize()));
                }
            }
        });

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
        tracing::trace!("stream {} closed", stream_id);
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
