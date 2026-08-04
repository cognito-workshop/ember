use bytes::{Bytes, BytesMut};
use flume::{Receiver, Sender};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio_websockets::Message;

use crate::error::WispError;
use crate::wisp::packet::Packet;

/// UDP proxy: forwards datagrams between client (via Wisp) and upstream UDP server.
///
/// Unlike TCP, UDP has no flow control — datagrams are sent immediately.
/// Each stream gets its own UdpSocket bound to a random local port.
pub async fn proxy_udp(
    stream_id: u32,
    upstream_addr: SocketAddr,
    data_rx: Receiver<Bytes>,
    ws_write_tx: Sender<Message>,
) -> Result<(), WispError> {
    // Bind to a random local port for this stream
    let local_socket = UdpSocket::bind("0.0.0.0:0").await?;
    local_socket.connect(upstream_addr).await?;

    tracing::debug!(
        "UDP stream {}: bound to {}, upstream {}",
        stream_id,
        local_socket.local_addr()?,
        upstream_addr
    );

    // Arc for sharing between read/write tasks
    let local_socket = Arc::new(local_socket);

    // Task 1: Read from upstream UDP, send to client via WS
    let ws_write_tx_clone = ws_write_tx.clone();
    let socket_read = local_socket.clone();
    let upstream_to_client = tokio::spawn(async move {
        let mut buf = BytesMut::with_capacity(65536);
        loop {
            buf.clear();
            match socket_read.recv_buf(&mut buf).await {
                Ok(0) => break,
                Ok(_n) => {
                    let payload = buf.split().freeze();
                    let packet = Packet::data(stream_id, payload);
                    if ws_write_tx_clone
                        .send(Message::binary(packet.serialize()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(e) => {
                    tracing::trace!("UDP recv error on stream {}: {}", stream_id, e);
                    break;
                }
            }
        }
    });

    // Task 2: Read from client via WS channel, send to upstream UDP
    let client_to_upstream = tokio::spawn(async move {
        while let Ok(data) = data_rx.recv_async().await {
            if local_socket.send(&data).await.is_err() {
                break;
            }
        }
    });

    // Wait for either direction to finish
    tokio::select! {
        _ = upstream_to_client => {},
        _ = client_to_upstream => {},
    }

    tracing::debug!("UDP stream {} closed", stream_id);
    Ok(())
}
