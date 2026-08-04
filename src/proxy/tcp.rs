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

/// Optimize TCP socket for high-throughput proxying
fn optimize_tcp_socket(stream: &TcpStream) {
    stream.set_nodelay(true).ok();

    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        unsafe {
            let fd = stream.as_raw_fd();

            // Enable TCP keepalive
            let one: libc::c_int = 1;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_KEEPALIVE,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );

            // Set larger send buffer (256KB)
            let buf_size: libc::c_int = 256 * 1024;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &buf_size as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );

            // Set larger receive buffer (256KB)
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &buf_size as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
}

pub async fn proxy_tcp(
    stream_id: u32,
    tcp_stream: TcpStream,
    data_rx: Receiver<Bytes>,
    ws_write_tx: Sender<Message>,
    buffer_size: usize,
    metrics: Option<Arc<Metrics>>,
) -> Result<(), WispError> {
    // Optimize the upstream TCP socket
    optimize_tcp_socket(&tcp_stream);

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
    optimize_tcp_socket(&stream);
    Ok(stream)
}
