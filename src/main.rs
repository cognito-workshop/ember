#![allow(unused)]

use clap::Parser;
use ember::config::Cli;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static JEMALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() {
    let cli = Cli::parse();
    let config = ember::config::Config::load(&cli).unwrap_or_else(|e| {
        eprintln!("Configuration error: {}", e);
        std::process::exit(1);
    });

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.logging.level));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .init();

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    if cli.thread_per_core {
        tracing::info!("Ember v{} starting in thread-per-core mode ({} cores)", env!("CARGO_PKG_VERSION"), workers);
        thread_per_core_main(config, workers);
    } else {
        tracing::info!("Ember v{} starting up ({} workers)", env!("CARGO_PKG_VERSION"), workers);
        multi_thread_main(config, workers);
    }
}

fn multi_thread_main(config: ember::config::Config, workers: usize) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(workers)
        .thread_name("ember-worker")
        .build()
        .expect("failed to create tokio runtime");

    runtime.block_on(async {
        // Spawn the server
        let server_handle = tokio::spawn(ember::server::run(config));

        // Wait for shutdown signal
        wait_for_shutdown().await;

        tracing::info!("Shutdown signal received, draining connections...");
        server_handle.abort();

        // Give connections time to drain
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        tracing::info!("Ember shut down gracefully");
    });
}

fn thread_per_core_main(config: ember::config::Config, num_cores: usize) {
    let mut handles = Vec::new();

    for i in 0..num_cores {
        let config = config.clone();
        let handle = std::thread::Builder::new()
            .name(format!("ember-core-{}", i))
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create runtime");

                runtime.block_on(async {
                    let server_handle = tokio::spawn(ember::server::run(config));
                    wait_for_shutdown().await;
                    server_handle.abort();
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                });
            })
            .expect("failed to spawn worker thread");

        handles.push(handle);
    }

    tracing::info!("Spawned {} worker threads, waiting for shutdown...", num_cores);

    // Wait for shutdown signal on the main thread
    block_on_shutdown();

    tracing::info!("Shutting down {} worker threads...", num_cores);
    for handle in handles {
        let _ = handle.join();
    }
    tracing::info!("Ember shut down gracefully");
}

/// Wait for SIGTERM or SIGINT
async fn wait_for_shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");

        tokio::select! {
            _ = ctrl_c => tracing::info!("Received SIGINT"),
            _ = sigterm.recv() => tracing::info!("Received SIGTERM"),
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        tracing::info!("Received SIGINT");
    }
}

/// Block on shutdown signal (for non-async contexts)
fn block_on_shutdown() {
    let (tx, rx) = std::sync::mpsc::channel();

    ctrlc::set_handler(move || {
        tx.send(()).ok();
    })
    .expect("failed to set Ctrl-C handler");

    rx.recv().ok();
}
