use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio_websockets::ServerBuilder;

use crate::config::Config;
use crate::wisp::handshake::{handshake_v2, perform_v1_init, WispVersion};
use crate::wisp::mux::MuxInner;
use crate::wisp::extensions::{Extension, ExtensionNegotiation};

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&addr).await?;
    run_with_listener(listener, config).await
}

pub async fn run_with_listener(
    listener: TcpListener,
    config: Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = listener.local_addr()?;
    tracing::info!("Ember listening on {}", addr);

    let connection_count = Arc::new(AtomicU32::new(0));

    loop {
        let (stream, addr) = listener.accept().await?;
        let count = connection_count.fetch_add(1, Ordering::Relaxed) + 1;

        if count > config.server.max_connections {
            connection_count.fetch_sub(1, Ordering::Relaxed);
            tracing::warn!("Connection limit reached from {}", addr);
            drop(stream);
            continue;
        }

        let config = config.clone();
        let count = connection_count.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, addr, config).await {
                tracing::error!("Connection error from {}: {}", addr, e);
            }
            count.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    config: Config,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_nodelay(true)?;

    let (req, mut ws_stream) = ServerBuilder::new().accept(stream).await?;

    let version = if req.headers().contains_key("sec-websocket-protocol") {
        WispVersion::V2
    } else {
        WispVersion::V1
    };

    tracing::debug!("{}: Wisp {:?}", addr, version);

    let mut server_extensions = Vec::new();
    if config.extensions.udp {
        server_extensions.push(Extension::Udp);
    }
    if !config.extensions.motd.is_empty() {
        server_extensions.push(Extension::Motd);
    }
    if config.extensions.stream_open_confirmation {
        server_extensions.push(Extension::StreamOpenConfirmation);
    }

    let motd = if config.extensions.motd.is_empty() {
        None
    } else {
        Some(config.extensions.motd.clone())
    };

    let extensions = match version {
        WispVersion::V1 => {
            perform_v1_init(&mut ws_stream, config.buffer.initial_size).await?;
            ExtensionNegotiation::negotiate(&[], &[])
        }
        WispVersion::V2 => {
            let (ext, _v) = handshake_v2(
                &mut ws_stream,
                &server_extensions,
                &motd,
                config.buffer.initial_size,
            ).await?;
            ext
        }
    };

    let mut mux = MuxInner::new(
        config.buffer.clone().into(),
        extensions,
        motd,
        config.buffer.tcp_read_size,
    );
    mux.run(ws_stream).await?;

    Ok(())
}
