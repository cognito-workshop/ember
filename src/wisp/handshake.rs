use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use http::HeaderMap;
use tokio_websockets::{Message, WebSocketStream};

use crate::error::WispError;
use crate::wisp::extensions::{
    parse_extension_data, Extension, ExtensionData, ExtensionNegotiation,
};
use crate::wisp::packet::{Packet, PacketType};

const WISP_VERSION_MAJOR: u8 = 2;
const WISP_VERSION_MINOR: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WispVersion {
    V1,
    V2,
}

pub fn detect_version(headers: &HeaderMap) -> WispVersion {
    if headers.contains_key("sec-websocket-protocol") {
        WispVersion::V2
    } else {
        WispVersion::V1
    }
}

#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub version_major: u8,
    pub version_minor: u8,
    pub extensions: Vec<Extension>,
}

impl ServerInfo {
    pub fn new(major: u8, minor: u8, extensions: Vec<Extension>) -> Self {
        Self { version_major: major, version_minor: minor, extensions }
    }

    pub fn to_payload(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(2 + self.extensions.len() * 5);
        buf.put_u8(self.version_major);
        buf.put_u8(self.version_minor);
        for ext in &self.extensions {
            buf.put_u8(*ext as u8);
            buf.put_u32_le(0);
        }
        buf.freeze()
    }
}

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub version_major: u8,
    pub version_minor: u8,
    pub extensions: Vec<ExtensionData>,
}

impl ClientInfo {
    pub fn parse(payload: Bytes) -> Result<Self, WispError> {
        if payload.len() < 2 {
            return Err(WispError::PacketTooShort(payload.len()));
        }
        let version_major = payload[0];
        let version_minor = payload[1];
        let extensions = parse_extension_data(&payload[2..]);
        Ok(Self { version_major, version_minor, extensions })
    }
}

pub async fn handshake_v2<S>(
    ws: &mut WebSocketStream<S>,
    server_extensions: &[Extension],
    motd: &Option<String>,
    buffer_size: u32,
) -> Result<(ExtensionNegotiation, WispVersion), WispError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut ext_bytes = BytesMut::new();
    for ext in server_extensions {
        ext_bytes.put_u8(*ext as u8);
        if *ext == Extension::Motd {
            if let Some(motd_text) = motd {
                let motd_utf8 = motd_text.as_bytes();
                ext_bytes.put_u32_le(motd_utf8.len() as u32);
                ext_bytes.put_slice(motd_utf8);
            } else {
                ext_bytes.put_u32_le(0);
            }
        } else {
            ext_bytes.put_u32_le(0);
        }
    }

    let info_packet = Packet::info(0, WISP_VERSION_MAJOR, WISP_VERSION_MINOR, &ext_bytes);
    ws.send(Message::binary(info_packet.serialize())).await?;

    let client_msg = ws
        .next()
        .await
        .ok_or(WispError::ConnectionClosed)?
        .map_err(|e| WispError::WebSocket(e.to_string()))?;

    if !client_msg.is_binary() {
        return Err(WispError::WebSocket("expected binary message during handshake".into()));
    }

    let client_bytes: Bytes = client_msg.into_payload().into();
    let packet = Packet::parse(client_bytes)?;

    match packet.packet_type {
        PacketType::Continue => {
            tracing::debug!("client responded with CONTINUE, falling back to v1");
            let negotiation = ExtensionNegotiation::negotiate(&[], &[]);
            Ok((negotiation, WispVersion::V1))
        }
        PacketType::Info => {
            let client_info = ClientInfo::parse(packet.payload)?;

            let client_extensions: Vec<Extension> = client_info
                .extensions
                .iter()
                .filter_map(|ed| Extension::from_u8(ed.id))
                .collect();

            let negotiation = ExtensionNegotiation::negotiate(server_extensions, &client_extensions);

            let continue_pkt = Packet::continue_packet(0, buffer_size);
            ws.send(Message::binary(continue_pkt.serialize())).await?;

            Ok((negotiation, WispVersion::V2))
        }
        other => {
            tracing::error!("unexpected packet type {:#04x} during v2 handshake", other as u8);
            Err(WispError::WebSocket(format!(
                "unexpected packet type {:#04x} during handshake",
                other as u8
            )))
        }
    }
}

pub async fn perform_v1_init<S>(ws: &mut WebSocketStream<S>, buffer_size: u32) -> Result<(), WispError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let continue_pkt = Packet::continue_packet(0, buffer_size);
    ws.send(Message::binary(continue_pkt.serialize())).await?;
    Ok(())
}
