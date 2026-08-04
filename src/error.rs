#[derive(Debug, thiserror::Error)]
pub enum WispError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Invalid packet type: {0:#04x}")]
    InvalidPacketType(u8),

    #[error("Packet too short: need at least 5 bytes, got {0}")]
    PacketTooShort(usize),

    #[error("Invalid stream type: {0:#04x}")]
    InvalidStreamType(u8),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Unknown stream ID: {0}")]
    UnknownStream(u32),

    #[error("Buffer full for stream: {0}")]
    BufferFull(u32),

    #[error("Incompatible version")]
    IncompatibleVersion,

    #[error("Handshake timeout")]
    HandshakeTimeout,

    #[error("Invalid close reason: {0:#04x}")]
    InvalidCloseReason(u8),

    #[error("Connection limit reached")]
    ConnectionLimitReached,

    #[error("Invalid config: {0}")]
    Config(String),
}

impl From<tokio_websockets::Error> for WispError {
    fn from(e: tokio_websockets::Error) -> Self {
        WispError::WebSocket(e.to_string())
    }
}
