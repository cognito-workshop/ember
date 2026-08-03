use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use std::time::Duration;

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
            motd: String::new(),
            stream_open_confirmation: false,
        },
        logging: LoggingConfig {
            level: "error".to_string(),
        },
    }
}

fn bench_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("wisp_throughput");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(15));

    for num_streams in [1, 10] {
        group.bench_with_input(
            BenchmarkId::new("echo", format!("{}s", num_streams)),
            &num_streams,
            |b, &num_streams| {
                b.iter(|| {
                    rt.block_on(async {
                        let echo_addr = start_echo_server().await;
                        let server_addr = start_ember_server(test_config(0)).await;

                        let mut client = WispClient::connect(server_addr).await.unwrap();
                        let _ = client.recv().await.unwrap();

                        for i in 1..=num_streams as u32 {
                            let _ = client.open_stream(i, "127.0.0.1", echo_addr.port()).await.unwrap();
                        }

                        let payload = vec![0x42u8; 50 * 1024]; // 50KB
                        let iterations = 50;

                        // Warmup
                        for i in 1..=num_streams as u32 {
                            client.send_data(i, Bytes::from(payload.clone())).await.unwrap();
                        }
                        for _ in 0..num_streams {
                            let _ = client.recv().await;
                        }

                        // Benchmark
                        for _ in 0..iterations {
                            for i in 1..=num_streams as u32 {
                                client.send_data(i, Bytes::from(payload.clone())).await.unwrap();
                            }
                        }
                        for _ in 0..(num_streams * iterations) {
                            let _ = client.recv().await;
                        }
                    });
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_throughput);
criterion_main!(benches);
