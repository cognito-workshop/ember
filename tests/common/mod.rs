pub mod wisp_client;

use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Simple TCP echo server that echoes back whatever it receives.
/// Returns the address it's listening on.
pub async fn start_echo_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                loop {
                    match stream.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if stream.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    });

    addr
}

/// Start the Ember server on a random port and return the address
pub async fn start_ember_server(config: ember::config::Config) -> SocketAddr {
    // Bind to port 0 to get a random available port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listen_addr = listener.local_addr().unwrap();

    let config = ember::config::Config {
        server: ember::config::ServerConfig {
            host: "127.0.0.1".to_string(),
            port: listen_addr.port(),
            max_connections: 10000,
            metrics_port: 0,
        },
        ..config
    };

    // Spawn the server with the pre-bound listener
    tokio::spawn(async move {
        ember::server::run_with_listener(listener, config).await.unwrap();
    });

    // Give the server a moment to start accepting
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    listen_addr
}
