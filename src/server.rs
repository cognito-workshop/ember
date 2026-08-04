use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

pub static IS_SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "linux")]
use std::os::fd::FromRawFd;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_websockets::ServerBuilder;

use crate::config::Config;
use crate::tls::load_tls_config;
use crate::wisp::handshake::{handshake_v2, perform_v1_init, WispVersion};
use crate::wisp::mux::MuxInner;
use crate::wisp::extensions::{Extension, ExtensionNegotiation};
use crate::wisp::plugin::PluginManager;
use crate::wisp::plugins::{Metrics, RateLimiter, ConnectionLimiter, Logger};

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = create_listener(&addr).await?;
    run_with_listener(listener, config).await
}

/// Create a TcpListener with SO_REUSEPORT on Linux (for thread-per-core mode)
async fn create_listener(addr: &str) -> Result<TcpListener, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_os = "linux")]
    {
        let socket_addr: SocketAddr = addr.parse()?;
        let domain = if socket_addr.is_ipv4() { libc::AF_INET } else { libc::AF_INET6 };

        unsafe {
            let fd = libc::socket(domain, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0);
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }

            let one: libc::c_int = 1;
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, &one as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as libc::socklen_t);
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, &one as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as libc::socklen_t);

            let mut storage: libc::sockaddr_storage = std::mem::zeroed();
            let (addr_ptr, addr_len) = match socket_addr {
                SocketAddr::V4(ref a) => {
                    let sin = &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in);
                    sin.sin_family = libc::AF_INET as libc::sa_family_t;
                    sin.sin_port = a.port().to_be();
                    sin.sin_addr.s_addr = u32::from_ne_bytes(a.ip().octets());
                    (&storage as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t)
                }
                SocketAddr::V6(ref a) => {
                    let sin6 = &mut *(&mut storage as *mut _ as *mut libc::sockaddr_in6);
                    sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                    sin6.sin6_port = a.port().to_be();
                    sin6.sin6_addr.s6_addr = a.ip().octets();
                    (&storage as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t)
                }
            };

            if libc::bind(fd, addr_ptr, addr_len) < 0 {
                let err = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(err.into());
            }
            if libc::listen(fd, 64) < 0 {
                let err = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(err.into());
            }

            let std_listener = std::net::TcpListener::from_raw_fd(fd);
            let listener = TcpListener::from_std(std_listener)?;
            Ok(listener)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let listener = TcpListener::bind(addr).await?;
        Ok(listener)
    }
}

pub async fn run_with_listener(
    listener: TcpListener,
    config: Config,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = listener.local_addr()?;
    tracing::info!("Ember listening on {}", addr);

    // Create TLS acceptor if TLS is enabled
    let tls_acceptor = if config.tls.enabled {
        let tls_config = load_tls_config(&config.tls.cert_path, &config.tls.key_path)?;
        Some(Arc::new(tokio_rustls::TlsAcceptor::from(Arc::new(tls_config))))
    } else {
        None
    };

    // Create shared metrics
    let metrics = Metrics::new();

    // Spawn metrics HTTP endpoint
    {
        let metrics = metrics.clone();
        let metrics_addr = format!("{}:{}", config.server.host, config.server.metrics_port);
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(&metrics_addr, metrics).await {
                tracing::error!("Metrics server error: {}", e);
            }
        });
    }

    // Create shared plugin manager
    let mut pm = PluginManager::new();

    // Register plugins from config
    if let Some(ref rl_config) = config.plugins.rate_limiter {
        pm.register(RateLimiter::new(rl_config.max_connections_per_ip, rl_config.window_secs));
    }
    pm.register(ConnectionLimiter::new(config.server.max_connections));
    if config.plugins.logger {
        pm.register(Arc::new(Logger));
    }

    let plugins = Arc::new(pm);
    plugins.load_all().await?;

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
        let plugins = plugins.clone();
        let metrics = metrics.clone();
        let tls_acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, addr, config, plugins, metrics, tls_acceptor).await {
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
    plugins: Arc<PluginManager>,
    metrics: Arc<Metrics>,
    tls_acceptor: Option<Arc<tokio_rustls::TlsAcceptor>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    stream.set_nodelay(true)?;

    if let Some(acceptor) = tls_acceptor {
        let tls_stream = acceptor.accept(stream).await?;
        let (req, ws_stream) = ServerBuilder::new().accept(tls_stream).await?;
        handle_websocket(req, ws_stream, addr, config, plugins, metrics).await?;
    } else {
        let (req, ws_stream) = ServerBuilder::new().accept(stream).await?;
        handle_websocket(req, ws_stream, addr, config, plugins, metrics).await?;
    }

    Ok(())
}

async fn handle_websocket<S>(
    req: http::Request<()>,
    mut ws_stream: tokio_websockets::WebSocketStream<S>,
    addr: SocketAddr,
    config: Config,
    plugins: Arc<PluginManager>,
    metrics: Arc<Metrics>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{

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
        plugins,
        addr,
        Some(metrics),
        config.server.max_connections,
    );
    mux.run(ws_stream).await?;

    Ok(())
}

async fn run_metrics_server(
    addr: &str,
    metrics: Arc<Metrics>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Metrics server listening on {}", addr);

    loop {
        let (mut stream, _) = listener.accept().await?;
        let metrics = metrics.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 1024];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };

            let request = String::from_utf8_lossy(&buf[..n]);

            if request.starts_with("GET /health") || request.starts_with("GET /health ") {
                let body = r#"{"status":"ok"}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                let _ = stream.write_all(response.as_bytes()).await;
                return;
            }

            if request.starts_with("GET /ready") || request.starts_with("GET /ready ") {
                if IS_SHUTTING_DOWN.load(Ordering::Relaxed) {
                    let body = r#"{"status":"shutting_down"}"#;
                    let response = format!(
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body,
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                } else {
                    let body = r#"{"status":"ready"}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body,
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                }
                return;
            }

            let is_metrics_request = request.starts_with("GET /metrics")
                || request.starts_with("GET /metrics ");

            if !is_metrics_request {
                let response = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(response).await;
                return;
            }

            let snap = metrics.snapshot();
            let body = format!(
                "# HELP ember_connections_active Active connections\n\
                 # TYPE ember_connections_active gauge\n\
                 ember_connections_active {val}\n\
                 # HELP ember_connections_total Total connections accepted\n\
                 # TYPE ember_connections_total counter\n\
                 ember_connections_total {total}\n\
                 # HELP ember_streams_active Active streams\n\
                 # TYPE ember_streams_active gauge\n\
                 ember_streams_active {s_active}\n\
                 # HELP ember_streams_total Total streams opened\n\
                 # TYPE ember_streams_total counter\n\
                 ember_streams_total {s_total}\n\
                 # HELP ember_bytes_received_total Total bytes received from clients\n\
                 # TYPE ember_bytes_received_total counter\n\
                 ember_bytes_received_total {b_in}\n\
                 # HELP ember_bytes_sent_total Total bytes sent to clients\n\
                 # TYPE ember_bytes_sent_total counter\n\
                 ember_bytes_sent_total {b_out}\n",
                val = snap.connections_active,
                total = snap.connections_total,
                s_active = snap.streams_active,
                s_total = snap.streams_total,
                b_in = snap.bytes_in,
                b_out = snap.bytes_out,
            );

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body,
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}
