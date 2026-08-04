mod common;

use bytes::Bytes;
use common::wisp_client::{Packet, PacketType, WispClient};
use common::{start_echo_server, start_ember_server};
use ember::config::{BufferConfig, Config, ExtensionsConfig, LoggingConfig, ServerConfig, TlsConfig};

fn test_config(port: u16) -> Config {
    Config {
        server: ServerConfig {
            host: "127.0.0.1".to_string(),
            port,
            max_connections: 10000,
        },
        tls: TlsConfig::default(),
        buffer: BufferConfig::default(),
        extensions: ExtensionsConfig {
            udp: true,
            motd: "Test Server".to_string(),
            stream_open_confirmation: false,
        },
        logging: LoggingConfig {
            level: "error".to_string(),
        },
        plugins: ember::config::PluginsConfig::default(),
    }
}

#[tokio::test]
async fn test_v1_handshake() {
    let echo_addr = start_echo_server().await;
    let server_addr = start_ember_server(test_config(0)).await;

    let mut client = WispClient::connect_v1(server_addr).await.unwrap();

    // v1: server should send CONTINUE immediately
    let pkt = client.recv().await.unwrap();
    assert_eq!(pkt.packet_type, PacketType::Continue);
    assert_eq!(pkt.stream_id, 0);
    assert_eq!(pkt.payload.len(), 4); // buffer size u32
}

#[tokio::test]
async fn test_connect_and_data() {
    let echo_addr = start_echo_server().await;
    let server_addr = start_ember_server(test_config(0)).await;

    let mut client = WispClient::connect_v1(server_addr).await.unwrap();

    // Receive initial CONTINUE (v1)
    let _ = client.recv().await.unwrap();

    // Open a stream to the echo server
    let resp = client.open_stream(1, "127.0.0.1", echo_addr.port()).await.unwrap();

    // Should get CONTINUE for the stream
    assert_eq!(resp.packet_type, PacketType::Continue);
    assert_eq!(resp.stream_id, 1);

    // Send data through the stream
    let test_data = b"Hello, Wisp!";
    client.send_data(1, Bytes::from_static(test_data)).await.unwrap();

    // Receive echoed data
    let resp = client.recv().await.unwrap();
    assert_eq!(resp.packet_type, PacketType::Data);
    assert_eq!(resp.stream_id, 1);
    assert_eq!(resp.payload.as_ref(), test_data);
}

#[tokio::test]
async fn test_multiple_streams() {
    let echo_addr = start_echo_server().await;
    let server_addr = start_ember_server(test_config(0)).await;

    let mut client = WispClient::connect_v1(server_addr).await.unwrap();

    // v1 init
    let _ = client.recv().await.unwrap();

    // Open 3 streams
    for i in 1..=3 {
        let resp = client.open_stream(i, "127.0.0.1", echo_addr.port()).await.unwrap();
        assert_eq!(resp.packet_type, PacketType::Continue);
        assert_eq!(resp.stream_id, i);
    }

    // Send data on each stream
    for i in 1..=3 {
        let data = format!("stream-{}", i);
        client.send_data(i, Bytes::from(data.into_bytes())).await.unwrap();
    }

    // Receive 3 echoed responses (order may vary)
    let mut received = Vec::new();
    for _ in 0..3 {
        let resp = client.recv().await.unwrap();
        assert_eq!(resp.packet_type, PacketType::Data);
        received.push((resp.stream_id, resp.payload));
    }

    // Verify all 3 streams got their data back
    received.sort_by_key(|(id, _)| *id);
    assert_eq!(received[0].0, 1);
    assert_eq!(received[0].1.as_ref(), b"stream-1");
    assert_eq!(received[1].0, 2);
    assert_eq!(received[1].1.as_ref(), b"stream-2");
    assert_eq!(received[2].0, 3);
    assert_eq!(received[2].1.as_ref(), b"stream-3");
}

#[tokio::test]
async fn test_close_stream() {
    let echo_addr = start_echo_server().await;
    let server_addr = start_ember_server(test_config(0)).await;

    let mut client = WispClient::connect_v1(server_addr).await.unwrap();
    let _ = client.recv().await.unwrap(); // v1 init

    // Open stream
    let _ = client.open_stream(1, "127.0.0.1", echo_addr.port()).await.unwrap();

    // Send some data
    client.send_data(1, Bytes::from_static(b"before close")).await.unwrap();
    let resp = client.recv().await.unwrap();
    assert_eq!(resp.packet_type, PacketType::Data);

    // Close the stream
    client.close_stream(1, 0x02).await.unwrap();

    // Give server a moment to process
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Opening a new stream should still work
    let resp = client.open_stream(2, "127.0.0.1", echo_addr.port()).await.unwrap();
    assert_eq!(resp.packet_type, PacketType::Continue);
    assert_eq!(resp.stream_id, 2);
}

#[tokio::test]
async fn test_large_payload() {
    let echo_addr = start_echo_server().await;
    let server_addr = start_ember_server(test_config(0)).await;

    let mut client = WispClient::connect_v1(server_addr).await.unwrap();
    let _ = client.recv().await.unwrap(); // v1 init

    let _ = client.open_stream(1, "127.0.0.1", echo_addr.port()).await.unwrap();

    // Send 64KB payload
    let data = vec![0xABu8; 64 * 1024];
    client.send_data(1, Bytes::from(data.clone())).await.unwrap();

    let resp = client.recv().await.unwrap();
    assert_eq!(resp.packet_type, PacketType::Data);
    assert_eq!(resp.payload.as_ref(), data.as_slice());
}

#[tokio::test]
async fn test_packet_parse_roundtrip() {
    // Test that parse(serialize()) is identity
    let pkt = Packet::connect(42, "example.com", 80);
    let serialized = pkt.serialize();
    let parsed = Packet::parse(serialized).unwrap();
    assert_eq!(parsed.packet_type, PacketType::Connect);
    assert_eq!(parsed.stream_id, 42);
    assert_eq!(parsed.payload, pkt.payload);
}

#[tokio::test]
async fn test_packet_too_short() {
    let data = Bytes::from(vec![0x01, 0x00, 0x00]);
    let result = Packet::parse(data);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_packet_invalid_type() {
    let data = Bytes::from(vec![0xFF, 0x00, 0x00, 0x00, 0x00]);
    let result = Packet::parse(data);
    assert!(result.is_err());
}
