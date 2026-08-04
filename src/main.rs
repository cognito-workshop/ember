#![allow(unused)]

use clap::Parser;
use ember::config::Cli;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static JEMALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() {
    let cli = Cli::parse();
    let config_path = cli.config.clone();
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
        thread_per_core_main(config, config_path, workers);
    } else {
        tracing::info!("Ember v{} starting up ({} workers)", env!("CARGO_PKG_VERSION"), workers);
        multi_thread_main(config, config_path, workers);
    }
}

fn multi_thread_main(config: ember::config::Config, config_path: Option<String>, workers: usize) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(workers)
        .thread_name("ember-worker")
        .build()
        .expect("failed to create tokio runtime");

    runtime.block_on(async {
        // Spawn the server
        let server_handle = tokio::spawn(ember::server::run(config));

        // Spawn SIGHUP handler if config path is available
        #[cfg(unix)]
        if let Some(path) = config_path {
            tokio::spawn(sighup_reload_task(path));
        }

        // Wait for shutdown signal
        wait_for_shutdown().await;

        tracing::info!("Shutdown signal received, draining connections...");
        ember::server::IS_SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::SeqCst);
        server_handle.abort();

        // Give connections time to drain
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        tracing::info!("Ember shut down gracefully");
    });
}

fn thread_per_core_main(config: ember::config::Config, config_path: Option<String>, num_cores: usize) {
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
                    ember::server::IS_SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::SeqCst);
                    server_handle.abort();
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                });
            })
            .expect("failed to spawn worker thread");

        handles.push(handle);
    }

    tracing::info!("Spawned {} worker threads, waiting for shutdown...", num_cores);

    // Handle SIGHUP on the main thread (non-async)
    #[cfg(unix)]
    if let Some(path) = config_path {
        let path = Arc::new(path);

        // Initialize the self-pipe for SIGHUP
        let read_fd = sighup_pipe::init();
        unsafe {
            libc::signal(libc::SIGHUP, sighup_pipe::handler as libc::sighandler_t);
        }

        // Spawn a thread that reads SIGHUP from the pipe and reloads config
        let sighup_handle = std::thread::Builder::new()
            .name("ember-sighup".into())
            .spawn(move || loop {
                let mut buf = [0u8; 1];
                let n = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, 1) };
                if n > 0 && buf[0] == b'h' {
                    tracing::info!("Received SIGHUP, reloading config from '{}'...", path);
                    match ember::config::Config::load_from_path(&path) {
                        Ok(new_config) => {
                            tracing::info!(
                                "Config reloaded (max_connections={}, port={})",
                                new_config.server.max_connections,
                                new_config.server.port
                            );
                        }
                        Err(e) => {
                            tracing::warn!("Config reload failed, keeping current config: {}", e);
                        }
                    }
                }
            })
            .expect("failed to spawn SIGHUP handler thread");

        // Wait for shutdown on the main thread
        block_on_shutdown();

        // SIGHUP thread will exit when process exits
        drop(sighup_handle);
    } else {
        block_on_shutdown();
    }

    #[cfg(not(unix))]
    {
        block_on_shutdown();
    }

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

/// Async task that listens for SIGHUP and reloads config
#[cfg(unix)]
async fn sighup_reload_task(config_path: String) {
    use tokio::signal::unix::SignalKind;

    let mut sighup = match tokio::signal::unix::signal(SignalKind::hangup()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to register SIGHUP handler: {}", e);
            return;
        }
    };

    loop {
        sighup.recv().await;
        tracing::info!("Received SIGHUP, reloading config from '{}'...", config_path);
        match ember::config::Config::load_from_path(&config_path) {
            Ok(new_config) => {
                tracing::info!(
                    "Config reloaded (max_connections={}, port={})",
                    new_config.server.max_connections,
                    new_config.server.port
                );
            }
            Err(e) => {
                tracing::warn!("Config reload failed, keeping current config: {}", e);
            }
        }
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

/// SIGHUP handler that writes to a pipe for the listener thread
#[cfg(unix)]
mod sighup_pipe {
    use std::sync::atomic::{AtomicBool, Ordering};

    static WRITE_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
    static INITIALIZED: AtomicBool = AtomicBool::new(false);

    pub fn init() -> std::os::fd::RawFd {
        unsafe {
            let mut fds: [libc::c_int; 2] = [0; 2];
            libc::pipe(fds.as_mut_ptr());
            let read_fd = fds[0];
            let write_fd = fds[1];
            WRITE_FD.store(write_fd, Ordering::SeqCst);
            INITIALIZED.store(true, Ordering::SeqCst);
            read_fd
        }
    }

    pub extern "C" fn handler(_: libc::c_int) {
        if INITIALIZED.load(Ordering::SeqCst) {
            let fd = WRITE_FD.load(Ordering::SeqCst);
            if fd >= 0 {
                unsafe {
                    let b = b'h';
                    libc::write(fd, &b as *const _ as *const libc::c_void, 1);
                }
            }
        }
    }
}
