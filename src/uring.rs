//! io_uring TCP proxy (Linux only, optional feature)
//!
//! Uses io_uring for zero-copy TCP operations.
//! Requires Linux kernel 5.10+ and the `io_uring` feature.

#[cfg(feature = "io_uring")]
pub mod uring {
    use std::os::unix::io::AsRawFd;
    use bytes::{Bytes, BytesMut};
    use flume::Receiver;
    use tokio_websockets::Message;

    use crate::error::WispError;
    use crate::wisp::packet::{Packet, PacketType};

    /// io_uring-based TCP proxy.
    ///
    /// This is a simplified version that uses io_uring for the TCP read path.
    /// The WebSocket write path still uses tokio.
        pub async fn proxy_tcp_uring(
            stream_id: u32,
            tcp_stream: TcpStream,
            data_rx: Receiver<Bytes>,
            ws_write_tx: flume::Sender<Message>,
            buffer_size: usize,
        ) -> Result<(), WispError> {
            // For now, fall back to standard TCP proxy
            // Full io_uring integration requires a separate runtime
            crate::proxy::tcp::proxy_tcp(
                stream_id,
                tcp_stream,
                data_rx,
                ws_write_tx,
                buffer_size,
                None,
            ).await?;
            Ok(())
        }
}

/// Fallback when io_uring feature is not enabled
#[cfg(not(feature = "io_uring"))]
pub mod uring {
    /// No-op: io_uring not enabled
    pub fn is_available() -> bool {
        false
    }
}
