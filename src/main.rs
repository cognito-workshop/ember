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

    tracing::info!("Ember v{} starting up", env!("CARGO_PKG_VERSION"));

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(workers)
        .thread_name("ember-worker")
        .build()
        .expect("failed to create tokio runtime");

    runtime.block_on(ember::server::run(config)).expect("server error");
}
