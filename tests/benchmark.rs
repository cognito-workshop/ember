mod common;

use bytes::Bytes;
use common::wisp_client::{PacketType, WispClient};
use common::{start_echo_server, start_ember_server};
use ember::config::{BufferConfig, Config, ExtensionsConfig, LoggingConfig, ServerConfig, TlsConfig};
use std::time::Instant;

fn test_config() -> Config {
    Config {
        server: ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            max_connections: 10000,
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
        },
    }
}

// === FLOOD BENCHMARKS (match WispMark methodology) ===
// Sends as fast as possible, measures total bytes over time

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

    let payload_size = 50 * 1024; // 50KB like WispMark
    let payload = Bytes::from(vec![0x42u8; payload_size]);
    let max_buffered = 5 * 1024 * 1024; // 5MB WS buffer limit

    // Track total bytes sent
    let total_bytes_sent = std::sync::atomic::AtomicU64::new(0);
    let total_bytes_sent = &total_bytes_sent;

    let mut handles = Vec::new();

    for conn_id in 0..num_connections {
        let server_addr = server_addr;
        let echo_port = echo_addr.port();
        let payload = payload.clone();

        let handle = tokio::spawn(async move {
            let mut client = WispClient::connect_v1(server_addr).await.unwrap();
            let _ = client.recv().await.unwrap(); // v1 init

            // Open all streams
            for i in 1..=streams_per_conn as u32 {
                let _ = client.open_stream(i, "127.0.0.1", echo_port).await.unwrap();
            }

            // Flood: send packets as fast as possible for duration_secs
            let deadline = Instant::now() + std::time::Duration::from_secs(duration_secs);
            let mut sent: u64 = 0;

            while Instant::now() < deadline {
                for stream_id in 1..=streams_per_conn as u32 {
                    // Send 10 packets per stream per iteration (like WispMark)
                    for _ in 0..10 {
                        client.send_data(stream_id, payload.clone()).await.unwrap();
                        sent += 1;
                    }
                }

                // Yield to let the runtime process outgoing data
                tokio::task::yield_now().await;
            }

            total_bytes_sent.fetch_add(sent * payload_size, std::sync::atomic::Ordering::Relaxed);

            // Drain remaining responses
            let _ = client;
        });

        handles.push(handle);
    }

    // Wait for all senders to finish
    for h in handles {
        let _ = h.await;
    }

    let total = total_bytes_sent.load(std::sync::atomic::Ordering::Relaxed);
    let elapsed = duration_secs as f64;
    let throughput = (total as f64) / elapsed / (1024.0 * 1024.0);

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

// === LATENCY BENCHMARK ===

#[tokio::test]
async fn benchmark_latency() {
    let echo_addr = start_echo_server().await;
    let server_addr = start_ember_server(test_config()).await;

    let mut client = WispClient::connect_v1(server_addr).await.unwrap();
    let _ = client.recv().await.unwrap(); // v1 init

    let _ = client.open_stream(1, "127.0.0.1", echo_addr.port()).await.unwrap();

    let payload = Bytes::from(vec![0x42u8; 1024]); // 1KB
    let iterations = 30;

    // Warmup
    for _ in 0..3 {
        client.send_data(1, payload.clone()).await.unwrap();
        loop {
            let pkt = client.recv().await.unwrap();
            if pkt.packet_type == PacketType::Data { break; }
        }
    }

    // Measure latency
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
    let min = latencies[0];
    let max = latencies.last().unwrap();

    println!();
    println!("=== Latency Benchmark ===");
    println!("Payload size:  1 KB");
    println!("Iterations:    {}", iterations);
    println!("Average:       {:.1} us", avg);
    println!("Min:           {} us", min.as_micros());
    println!("Max:           {} us", max.as_micros());
    println!("P50:           {} us", p50.as_micros());
    println!("P99:           {} us", p99.as_micros());
    println!();
}
