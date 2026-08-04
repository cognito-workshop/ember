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

    runtime.block_on(ember::server::run(config)).expect("server error");
}

fn thread_per_core_main(config: ember::config::Config, num_cores: usize) {
    // Thread-per-core: each core gets its own single-threaded tokio runtime
    // All bind to the same port with SO_REUSEPORT (Linux only)
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

                runtime.block_on(ember::server::run(config)).expect("server error");
            })
            .expect("failed to spawn worker thread");

        handles.push(handle);
    }

    tracing::info!("Spawned {} worker threads", num_cores);

    // Wait for all threads (they run until the process is killed)
    for handle in handles {
        handle.join().expect("worker thread panicked");
    }
}
