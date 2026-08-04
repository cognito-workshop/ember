mod common;

use bytes::{BufMut, Bytes, BytesMut};
use common::wisp_client::{PacketType, WispClient};
use common::{start_echo_server, start_ember_server};
use ember::config::{BufferConfig, Config, ExtensionsConfig, LoggingConfig, ServerConfig, TlsConfig};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn test_config() -> Config {
    Config {
        server: ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            max_connections: 10000,
            metrics_port: 0,
        },
        tls: TlsConfig::default(),
        buffer: BufferConfig::default(),
        extensions: ExtensionsConfig {
            udp: true,
            motd: String::new(),
            stream_open_confirmation: false,
        },
        logging: LoggingConfig {
            level: "error".to_string(),
            file: None,
            max_size_mb: 100,
        },
        plugins: ember::config::PluginsConfig::default(),
    }
}

#[tokio::test]
async fn flood_1x10() {
    run_flood_bench(1, 10, 3).await;
}

#[tokio::test]
async fn flood_5x10() {
    run_flood_bench(5, 10, 3).await;
}

async fn run_flood_bench(num_connections: usize, streams_per_conn: usize, duration_secs: u64) {
    let echo_addr = start_echo_server().await;
    let server_addr = start_ember_server(test_config()).await;

    let payload_size = 50 * 1024;
    let payload = Bytes::from(vec![0x42u8; payload_size]);

    let total_bytes_sent = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();

    for _ in 0..num_connections {
        let server_addr = server_addr;
        let echo_port = echo_addr.port();
        let payload = payload.clone();
        let total_bytes_sent = total_bytes_sent.clone();

        let handle = tokio::spawn(async move {
            let mut client = WispClient::connect_v1(server_addr).await.unwrap();
            let _ = client.recv().await.unwrap();

            for i in 1..=streams_per_conn as u32 {
                let _ = client.open_stream(i, "127.0.0.1", echo_port).await.unwrap();
            }

            // Split client for parallel sending
            let ws_tx = client.into_sender();

            // Spawn parallel sender tasks per stream (like WispMark's setInterval)
            let deadline = Instant::now() + std::time::Duration::from_secs(duration_secs);
            let mut stream_handles = Vec::new();

            for stream_id in 1..=streams_per_conn as u32 {
                let payload = payload.clone();
                let ws_tx = ws_tx.clone();

                let h = tokio::spawn(async move {
                    let mut sent: u64 = 0;
                    while Instant::now() < deadline {
                        for _ in 0..10 {
                            let packet = crate::common::wisp_client::Packet::data(stream_id, payload.clone());
                            let msg = tokio_websockets::Message::binary(packet.serialize());
                            if ws_tx.send(msg).is_err() { break; }
                            sent += 1;
                        }
                        tokio::task::yield_now().await;
                    }
                    sent
                });
                stream_handles.push(h);
            }

            let mut total_sent: u64 = 0;
            for h in stream_handles {
                total_sent += h.await.unwrap_or(0);
            }
            total_bytes_sent.fetch_add(total_sent * payload_size as u64, Ordering::Relaxed);
            let _ = client;
        });

        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }

    let total = total_bytes_sent.load(Ordering::Relaxed);
    let throughput = (total as f64) / duration_secs as f64 / (1024.0 * 1024.0);

    println!();
    println!("=== Flood Benchmark (WispMark-style) ===");
    println!("Connections:       {}", num_connections);
    println!("Streams/conn:      {}", streams_per_conn);
    println!("Total streams:     {}", num_connections * streams_per_conn);
    println!("Payload size:      {} KB", payload_size / 1024);
    println!("Duration:          {}s", duration_secs);
    println!("Total sent:        {:.2} MB", total as f64 / (1024.0 * 1024.0));
    println!("Throughput:        {:.2} MiB/s", throughput);
    println!();
}

#[tokio::test]
async fn benchmark_latency() {
    let echo_addr = start_echo_server().await;
    let server_addr = start_ember_server(test_config()).await;

    let mut client = WispClient::connect_v1(server_addr).await.unwrap();
    let _ = client.recv().await.unwrap();

    let _ = client.open_stream(1, "127.0.0.1", echo_addr.port()).await.unwrap();

    let payload = Bytes::from(vec![0x42u8; 1024]);
    let iterations = 30;

    for _ in 0..3 {
        client.send_data(1, payload.clone()).await.unwrap();
        loop {
            let pkt = client.recv().await.unwrap();
            if pkt.packet_type == PacketType::Data { break; }
        }
    }

    let mut latencies = Vec::new();
    for _ in 0..iterations {
        let start = Instant::now();
        client.send_data(1, payload.clone()).await.unwrap();
        loop {
            let pkt = client.recv().await.unwrap();
            if pkt.packet_type == PacketType::Data {
                latencies.push(start.elapsed());
                break;
            }
        }
    }

    latencies.sort();
    let avg = latencies.iter().map(|d| d.as_micros()).sum::<u128>() as f64 / iterations as f64;
    let p50 = latencies[iterations / 2];
    let p99 = latencies[(iterations * 99) / 100];

    println!();
    println!("=== Latency Benchmark ===");
    println!("Average:  {:.1} us", avg);
    println!("P50:      {} us", p50.as_micros());
    println!("P99:      {} us", p99.as_micros());
    println!();
}

// === UDP FLOOD BENCHMARK ===

#[tokio::test]
async fn flood_udp_1x10() {
    run_udp_flood_bench(1, 10, 3).await;
}

async fn run_udp_flood_bench(num_connections: usize, streams_per_conn: usize, duration_secs: u64) {
    use common::wisp_client::Packet;

    let udp_addr = common::start_udp_echo_server().await;
    let server_addr = start_ember_server(test_config()).await;

    let payload_size = 50 * 1024;
    let payload = Bytes::from(vec![0x42u8; payload_size]);

    let total_bytes_sent = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();

    for _ in 0..num_connections {
        let server_addr = server_addr;
        let udp_port = udp_addr.port();
        let payload = payload.clone();
        let total_bytes_sent = total_bytes_sent.clone();

        let handle = tokio::spawn(async move {
            let mut client = WispClient::connect_v1(server_addr).await.unwrap();
            let _ = client.recv().await.unwrap();

            // Open UDP streams
            for i in 1..=streams_per_conn as u32 {
                let connect = Packet {
                    packet_type: PacketType::Connect,
                    stream_id: i,
                    payload: {
                        let mut p = bytes::BytesMut::new();
                        p.put_u8(0x02); // UDP
                        p.put_u16_le(udp_port);
                        p.put_slice(b"127.0.0.1");
                        p.freeze()
                    },
                };
                client.send(&connect).await.unwrap();
                let _ = client.recv().await.unwrap(); // CONTINUE
            }

            let ws_tx = client.into_sender();

            let deadline = Instant::now() + std::time::Duration::from_secs(duration_secs);
            let mut stream_handles = Vec::new();

            for stream_id in 1..=streams_per_conn as u32 {
                let payload = payload.clone();
                let ws_tx = ws_tx.clone();

                let h = tokio::spawn(async move {
                    let mut sent: u64 = 0;
                    while Instant::now() < deadline {
                        for _ in 0..10 {
                            let packet = Packet::data(stream_id, payload.clone());
                            let msg = tokio_websockets::Message::binary(packet.serialize());
                            if ws_tx.send(msg).is_err() { break; }
                            sent += 1;
                        }
                        tokio::task::yield_now().await;
                    }
                    sent
                });
                stream_handles.push(h);
            }

            let mut total_sent: u64 = 0;
            for h in stream_handles {
                total_sent += h.await.unwrap_or(0);
            }
            total_bytes_sent.fetch_add(total_sent * payload_size as u64, Ordering::Relaxed);
            let _ = client;
        });

        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }

    let total = total_bytes_sent.load(Ordering::Relaxed);
    let throughput = (total as f64) / duration_secs as f64 / (1024.0 * 1024.0);

    println!();
    println!("=== UDP Flood Benchmark ===");
    println!("Connections:       {}", num_connections);
    println!("Streams/conn:      {}", streams_per_conn);
    println!("Payload size:      {} KB", payload_size / 1024);
    println!("Duration:          {}s", duration_secs);
    println!("Total sent:        {:.2} MB", total as f64 / (1024.0 * 1024.0));
    println!("Throughput:        {:.2} MiB/s", throughput);
    println!();
}
