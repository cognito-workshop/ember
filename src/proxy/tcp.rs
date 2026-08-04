use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::{Bytes, BytesMut};
use flume::{Receiver, Sender};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_websockets::Message;

use crate::error::WispError;
use crate::wisp::packet::Packet;
use crate::wisp::plugins::Metrics;

pub async fn proxy_tcp(
    stream_id: u32,
    tcp_stream: TcpStream,
    data_rx: Receiver<Bytes>,
    ws_write_tx: Sender<Message>,
    buffer_size: usize,
    metrics: Option<Arc<Metrics>>,
) -> Result<(), WispError> {
    let (tcp_read, mut tcp_write) = tcp_stream.into_split();
    let mut reader = BufReader::with_capacity(buffer_size, tcp_read);

    let mut buf = BytesMut::with_capacity(128 * 1024);

    loop {
        tokio::select! {
            result = reader.read_buf(&mut buf) => {
                match result {
                    Ok(0) => break,
                    Ok(_) => {
                        let payload = buf.split().freeze();
                        let n = payload.len();
                        let packet = Packet::data(stream_id, payload);
                        if ws_write_tx.send(Message::binary(packet.serialize())).is_err() {
                            break;
                        }
                        if let Some(ref m) = metrics {
                            m.bytes_out.fetch_add(n as u64, Ordering::Relaxed);
                        }
                    }
                    Err(_) => break,
                }
            }
            result = data_rx.recv_async() => {
                match result {
                    Ok(data) => {
                        let n = data.len();
                        if tcp_write.write_all(&data).await.is_err() {
                            break;
                        }
                        if let Some(ref m) = metrics {
                            m.bytes_in.fetch_add(n as u64, Ordering::Relaxed);
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }

    Ok(())
}

pub async fn proxy_tcp_connect(host: String, port: u16) -> Result<TcpStream, WispError> {
    let addr = format!("{}:{}", host, port);
    let stream = TcpStream::connect(&addr).await?;
    stream.set_nodelay(true)?;
    Ok(stream)
}
