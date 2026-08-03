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

/// Benchmark: measure throughput with 1 stream, sending 50KB payloads
#[tokio::test]
async fn benchmark_throughput_1_stream() {
    run_throughput_bench(1, 10).await;
}

/// Benchmark: measure throughput with 10 streams
#[tokio::test]
async fn benchmark_throughput_10_streams() {
    run_throughput_bench(10, 5).await;
}

async fn run_throughput_bench(num_streams: usize, iterations: usize) {
    let echo_addr = start_echo_server().await;
    let server_addr = start_ember_server(test_config()).await;

    let mut client = WispClient::connect_v1(server_addr).await.unwrap();
    let init = client.recv().await.unwrap(); // v1 init CONTINUE(0)
    assert_eq!(init.packet_type, PacketType::Continue);

    // Open all streams
    for i in 1..=num_streams as u32 {
        let resp = client.open_stream(i, "127.0.0.1", echo_addr.port()).await.unwrap();
        assert_eq!(resp.packet_type, PacketType::Continue);
    }

    let payload_size = 50 * 1024; // 50KB like WispMark
    let payload = vec![0x42u8; payload_size];

    let total_packets = num_streams * iterations;

    // Benchmark: send one packet at a time and receive its response
    // This avoids overwhelming the server with backlogged packets
    let start = Instant::now();

    let mut sent = 0;
    let mut received = 0;

    while received < total_packets {
        // Send next packet if we haven't sent them all
        if sent < total_packets {
            let stream_id = ((sent % num_streams) + 1) as u32;
            client.send_data(stream_id, Bytes::from(payload.clone())).await.unwrap();
            sent += 1;
        }

        // Try to receive (with timeout)
        match tokio::time::timeout(std::time::Duration::from_secs(5), client.recv()).await {
            Ok(Ok(pkt)) if pkt.packet_type == PacketType::Data => received += 1,
            Ok(Ok(_)) => {} // skip CONTINUE
            Ok(Err(e)) => {
                eprintln!("recv error: {}", e);
                break;
            }
            Err(_) => {
                eprintln!("timeout waiting for packet {}/{}", received, total_packets);
                break;
            }
        }
    }

    let elapsed = start.elapsed();
    let total_bytes = (received as u64) * (payload_size as u64);
    let throughput_mbps = (total_bytes as f64) / elapsed.as_secs_f64() / (1024.0 * 1024.0);

    println!();
    println!("=== Throughput Benchmark ===");
    println!("Streams:            {}", num_streams);
    println!("Iterations:        {}", iterations);
    println!("Payload size:      {} KB", payload_size / 1024);
    println!("Total packets:     {}/{}", received, total_packets);
    println!("Total data:        {:.2} MB", total_bytes as f64 / (1024.0 * 1024.0));
    println!("Elapsed:           {:.3} ms", elapsed.as_secs_f64() * 1000.0);
    println!("Throughput:        {:.2} MiB/s", throughput_mbps);
    println!();
}

/// Benchmark: measure latency of single round-trip
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
