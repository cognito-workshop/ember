use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use http::HeaderValue;
use tokio::net::TcpStream;
use tokio_websockets::{ClientBuilder, Message, WebSocketStream};

use std::net::SocketAddr;

/// Wisp packet types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketType {
    Connect = 0x01,
    Data = 0x02,
    Continue = 0x03,
    Close = 0x04,
    Info = 0x05,
}

/// A parsed Wisp packet
#[derive(Debug, Clone)]
pub struct Packet {
    pub packet_type: PacketType,
    pub stream_id: u32,
    pub payload: Bytes,
}

impl Packet {
    pub fn parse(data: Bytes) -> Result<Self, String> {
        if data.len() < 5 {
            return Err(format!("packet too short: {} bytes", data.len()));
        }
        let mut buf = data;
        let pt = buf.get_u8();
        let stream_id = buf.get_u32_le();
        let payload = buf.copy_to_bytes(buf.remaining());

        let packet_type = match pt {
            0x01 => PacketType::Connect,
            0x02 => PacketType::Data,
            0x03 => PacketType::Continue,
            0x04 => PacketType::Close,
            0x05 => PacketType::Info,
            _ => return Err(format!("invalid packet type: {:#04x}", pt)),
        };

        Ok(Packet { packet_type, stream_id, payload })
    }

    pub fn serialize(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5 + self.payload.len());
        buf.put_u8(self.packet_type as u8);
        buf.put_u32_le(self.stream_id);
        buf.put_slice(&self.payload);
        buf.freeze()
    }

    pub fn connect(stream_id: u32, host: &str, port: u16) -> Self {
        let mut payload = BytesMut::with_capacity(3 + host.len());
        payload.put_u8(0x01); // TCP
        payload.put_u16_le(port);
        payload.put_slice(host.as_bytes());
        Packet {
            packet_type: PacketType::Connect,
            stream_id,
            payload: payload.freeze(),
        }
    }

    pub fn data(stream_id: u32, data: Bytes) -> Self {
        Packet { packet_type: PacketType::Data, stream_id, payload: data }
    }

    pub fn close(stream_id: u32, reason: u8) -> Self {
        Packet {
            packet_type: PacketType::Close,
            stream_id,
            payload: Bytes::from(vec![reason]),
        }
    }
}

/// A simple Wisp client for testing
pub struct WispClient {
    ws: WebSocketStream<tokio_websockets::MaybeTlsStream<TcpStream>>,
}

impl WispClient {
    /// Connect to a Wisp server (v1 — no Sec-WebSocket-Protocol header)
    pub async fn connect_v1(addr: SocketAddr) -> Result<Self, String> {
        let uri = format!("ws://{}", addr);
        let (ws, _) = ClientBuilder::new()
            .uri(&uri)
            .map_err(|e| format!("URI error: {}", e))?
            .connect()
            .await
            .map_err(|e| format!("connect error: {}", e))?;

        Ok(Self { ws })
    }

    /// Connect to a Wisp server (v2 — with Sec-WebSocket-Protocol header)
    pub async fn connect_v2(addr: SocketAddr) -> Result<Self, String> {
        let uri = format!("ws://{}", addr);
        let (ws, _) = ClientBuilder::new()
            .uri(&uri)
            .map_err(|e| format!("URI error: {}", e))?
            .add_header(
                http::header::SEC_WEBSOCKET_PROTOCOL,
                HeaderValue::from_static("wisp"),
            )
            .map_err(|e| format!("header error: {}", e))?
            .connect()
            .await
            .map_err(|e| format!("connect error: {}", e))?;

        Ok(Self { ws })
    }

    /// Send a packet
    pub async fn send(&mut self, packet: &Packet) -> Result<(), String> {
        self.ws
            .send(Message::binary(packet.serialize()))
            .await
            .map_err(|e| format!("send error: {}", e))
    }

    /// Receive a packet (with timeout)
    pub async fn recv(&mut self) -> Result<Packet, String> {
        let msg = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.ws.next(),
        )
        .await
        .map_err(|_| "recv timeout".to_string())?
        .ok_or("connection closed".to_string())?
        .map_err(|e| format!("recv error: {}", e))?;

        if !msg.is_binary() {
            return Err(format!("expected binary, got non-binary message"));
        }

        let data: Bytes = msg.into_payload().into();
        Packet::parse(data)
    }

    /// Send CONNECT packet and wait for any response
    pub async fn open_stream(&mut self, stream_id: u32, host: &str, port: u16) -> Result<Packet, String> {
        let connect = Packet::connect(stream_id, host, port);
        self.send(&connect).await?;
        self.recv().await
    }

    /// Send DATA packet
    pub async fn send_data(&mut self, stream_id: u32, data: Bytes) -> Result<(), String> {
        let packet = Packet::data(stream_id, data);
        self.send(&packet).await
    }

    /// Split into read/write halves for parallel flood sending
    pub fn split(self) -> (
        flume::Sender<Message>,
        flume::Receiver<Message>,
        futures_util::stream::SplitSink<WebSocketStream<tokio_websockets::MaybeTlsStream<TcpStream>>, Message>,
        futures_util::stream::SplitStream<WebSocketStream<tokio_websockets::MaybeTlsStream<TcpStream>>>,
    ) {
        use futures_util::SinkExt;
        let (ws_write, ws_read) = self.ws.split();
        let (send_tx, send_rx) = flume::unbounded();
        (send_tx, send_rx, ws_write, ws_read)
    }

    /// Get a raw sender for flood benchmarking
    pub fn into_sender(self) -> flume::Sender<Message> {
        use futures_util::SinkExt;
        let (mut ws_write, mut ws_read) = self.ws.split();
        let (tx, rx) = flume::unbounded::<Message>();

        // Spawn a writer task
        tokio::spawn(async move {
            while let Ok(msg) = rx.recv_async().await {
                if ws_write.send(msg).await.is_err() { break; }
            }
        });

        // Spawn a reader task (just drain)
        tokio::spawn(async move {
            while let Some(_) = ws_read.next().await {}
        });

        tx
    }

    /// Send CLOSE packet
    pub async fn close_stream(&mut self, stream_id: u32, reason: u8) -> Result<(), String> {
        let packet = Packet::close(stream_id, reason);
        self.send(&packet).await
    }
}
